use super::*;

use core::task::Waker;

use crate::bytes::{Buf, BufMut};

use super::super::frame::{head, headers::Pseudo, settings};

fn window_updates(flow: &mut FlowControl) -> Vec<(u32, u32)> {
    let mut buf = BytesMut::new();
    let mut cx = Context::from_waker(Waker::noop());
    let _ = flow.poll_encode(&mut buf, &mut cx);
    let mut updates = Vec::new();
    while !buf.is_empty() {
        let len = buf.get_uint(3) as usize;
        let head = head::Head::parse(&buf);
        buf.advance(6);
        let payload = buf.split_to(len);
        if head.kind() == head::Kind::WindowUpdate {
            let update = WindowUpdate::load(head, &payload).unwrap();
            updates.push((update.stream_id().into(), update.size_increment()));
        }
    }
    updates
}

fn assert_connection_update(flow: &mut FlowControl, expected: u32) {
    let updates = window_updates(flow)
        .into_iter()
        .filter(|(id, _)| *id == 0)
        .collect::<Vec<_>>();
    if expected == 0 {
        assert!(updates.is_empty(), "unexpected connection update: {updates:?}");
    } else {
        assert_eq!(updates, [(0, expected)]);
    }
}

fn new_flow(initial_window: u32) -> FlowControl {
    let mut settings = Settings::default();
    settings.set_initial_window_size(Some(initial_window));
    settings.set_max_concurrent_streams(Some(2));
    let mut flow = FlowControl::new(&settings);
    flow.init(settings);
    let initial_updates = window_updates(&mut flow);
    if initial_window > settings::DEFAULT_INITIAL_WINDOW_SIZE {
        assert_eq!(
            initial_updates,
            [(0, initial_window - settings::DEFAULT_INITIAL_WINDOW_SIZE)]
        );
    } else {
        assert!(initial_updates.is_empty());
    }

    for id in [1, 3] {
        let id = StreamId::from(id);
        let pseudo = Pseudo::request(Method::POST, "http://localhost/upload".parse().unwrap(), None);
        assert!(matches!(
            flow.recv_header(id, Headers::new(id, pseudo, HeaderMap::new())),
            Ok(Some(_))
        ));
    }
    flow
}

fn receive(flow: &mut FlowControl, id: u32, len: usize) {
    let mut bytes = Bytes::from(vec![0; len]);
    while !bytes.is_empty() {
        let chunk = bytes.split_to(bytes.len().min(settings::DEFAULT_MAX_FRAME_SIZE as usize));
        assert!(flow.recv_data(Data::new(StreamId::from(id), chunk)).is_ok());
    }
}

fn consume(flow: &mut FlowControl, id: u32, pending: &mut RecvWindow) -> usize {
    let mut cx = Context::from_waker(Waker::noop());
    let mut consumed = 0;
    loop {
        match flow.poll_stream_frame(&StreamId::from(id), pending, &mut cx) {
            Poll::Ready(Some(Ok(Frame::Data(bytes)))) => consumed += bytes.len(),
            Poll::Pending => return consumed,
            _ => panic!("expected DATA or a pending unfinished body"),
        }
    }
}

#[test]
fn connection_updates_are_batched_per_encode() {
    let mut flow = new_flow(65_535);
    let mut pending = [RecvWindow::ZERO; 2];

    // Both bodies reach Pending after each chunk. Their consumed credit still
    // accumulates into one connection update when the writer is polled.
    for n in 0..8 {
        let index = n % 2;
        let id = if index == 0 { 1 } else { 3 };
        receive(&mut flow, id, 1024);
        assert_eq!(consume(&mut flow, id, &mut pending[index]), 1024);
    }
    assert_eq!(window_updates(&mut flow), [(0, 8192)]);
    assert!(window_updates(&mut flow).is_empty());

    // The next encode reports only newly consumed bytes, even though stream
    // credit from the earlier chunks is still waiting for its threshold.
    receive(&mut flow, 1, 1024);
    assert_eq!(consume(&mut flow, 1, &mut pending[0]), 1024);
    assert_eq!(window_updates(&mut flow), [(0, 1024)]);
}

#[test]
fn connection_credit_waits_for_body_consumption() {
    let mut flow = new_flow(65_535);
    let mut pending = RecvWindow::ZERO;
    receive(&mut flow, 1, 16_384);
    receive(&mut flow, 3, 49_151);
    assert_eq!(flow.recv_connection_window.value(), 0);
    assert!(window_updates(&mut flow).is_empty());

    // Only body 1 is consumed. Its credit is returned on the next encode,
    // while the unread body 3 continues occupying connection credit.
    assert_eq!(consume(&mut flow, 1, &mut pending), 16_384);
    assert_eq!(window_updates(&mut flow), [(0, 16_384)]);
    assert_eq!(flow.recv_connection_window.value(), 16_384);

    // Stream 1 can continue. Receiving more DATA alone does not return credit.
    receive(&mut flow, 1, 1024);
    assert!(window_updates(&mut flow).is_empty());
    assert_eq!(consume(&mut flow, 1, &mut pending), 1024);
    assert_eq!(window_updates(&mut flow), [(0, 1024)]);
}

#[test]
fn body_drop_returns_unread_credit_without_counting_consumption_twice() {
    // Cover consumption below/above the stream threshold, empty/unread bodies,
    // and encoding, response completion, and END_STREAM before/after Drop.
    for consumed in [0, 1024, 49_152] {
        for unread in [0, 4096] {
            for encode_before_drop in [false, true] {
                for response_before_drop in [false, true] {
                    for end_before_drop in [false, true] {
                        let mut flow = new_flow(65_535);
                        let id = StreamId::from(1);
                        let mut pending = RecvWindow::ZERO;
                        receive(&mut flow, 1, consumed);
                        assert_eq!(consume(&mut flow, 1, &mut pending), consumed);
                        if encode_before_drop {
                            assert_connection_update(&mut flow, consumed as u32);
                        }
                        receive(&mut flow, 1, unread);
                        if end_before_drop {
                            let mut end = Data::new(id, Bytes::new());
                            end.set_end_stream(true);
                            assert!(flow.recv_data(end).is_ok());
                        }
                        if response_before_drop {
                            flow.response_task_done(id).unwrap();
                        }
                        assert!(flow.stream_map.contains_key(&id), "body must retain its stream");
                        flow.request_body_drop(id);
                        if !response_before_drop {
                            flow.response_task_done(id).unwrap();
                        }

                        let expected = unread + if encode_before_drop { 0 } else { consumed };
                        assert_connection_update(&mut flow, expected as u32);
                        assert_eq!(flow.recv_connection_window.value(), 65_535);
                        assert_eq!(flow.stream_map.contains_key(&id), !end_before_drop);

                        // A reset after Drop may remove the retained stream,
                        // but must not return its already-released credit again.
                        assert!(flow.recv_reset(Reset::new(id, Reason::CANCEL)).is_ok());
                        assert!(!flow.stream_map.contains_key(&id));
                        assert_connection_update(&mut flow, 0);
                    }
                }
            }
        }
    }
}

#[test]
fn body_drop_and_later_data_return_disjoint_credit() {
    for response_before_data in [false, true] {
        for encode_before_data in [false, true] {
            let mut flow = new_flow(65_535);
            let id = StreamId::from(1);
            receive(&mut flow, 1, 1024);
            flow.request_body_drop(id);
            if encode_before_data {
                assert_connection_update(&mut flow, 1024);
            }
            if response_before_data {
                flow.response_task_done(id).unwrap();
            }

            // DATA arriving after Drop is discarded, never put in the body queue.
            receive(&mut flow, 1, 2048);
            let mut end = Data::new(id, Bytes::from(vec![0; 256]));
            end.set_end_stream(true);
            assert!(flow.recv_data(end).is_ok());
            if !response_before_data {
                flow.response_task_done(id).unwrap();
            }
            assert!(!flow.stream_map.contains_key(&id));
            assert_connection_update(&mut flow, 2304 + if encode_before_data { 0 } else { 1024 });
            assert_eq!(flow.recv_connection_window.value(), 65_535);

            // Even after removal, a rejected frame must return its own credit.
            let res = flow.recv_data(Data::new(id, Bytes::from(vec![0; 128])));
            assert!(matches!(res, Err(Error::Reset(Reason::STREAM_CLOSED))));
            assert_connection_update(&mut flow, 128);
            assert_eq!(flow.recv_connection_window.value(), 65_535);
        }
    }
}

#[test]
fn body_drop_does_not_credit_padding_twice() {
    for padding in [0, 7, 255] {
        let mut flow = new_flow(65_535);
        let id = StreamId::from(1);
        let padded = |len: usize| {
            let mut payload = BytesMut::new();
            payload.put_u8(padding);
            payload.resize(1 + len + usize::from(padding), 0);
            Data::load(head::Head::new(head::Kind::Data, 0x8, id), payload.freeze()).unwrap()
        };
        let padding_credit = u32::from(padding) + 1;

        assert!(flow.recv_data(padded(1024)).is_ok());
        // Padding, including the length byte, is returned before body polling.
        assert_connection_update(&mut flow, padding_credit);
        let mut pending = RecvWindow::ZERO;
        assert_eq!(consume(&mut flow, 1, &mut pending), 1024);

        assert!(flow.recv_data(padded(4096)).is_ok());
        flow.request_body_drop(id);
        // Consumed payload + unread payload + only the second frame's padding.
        assert_connection_update(&mut flow, 5120 + padding_credit);
        assert_eq!(flow.recv_connection_window.value(), 65_535);
        flow.response_task_done(id).unwrap();
        assert_connection_update(&mut flow, 0);
    }
}

#[test]
fn body_drop_after_termination_returns_only_queued_data() {
    #[derive(Clone, Copy)]
    enum Termination {
        PeerReset,
        InternalReset,
        StreamWindowError,
        Trailers,
        ReadClosed,
        IoError,
    }

    for termination in [
        Termination::PeerReset,
        Termination::InternalReset,
        Termination::StreamWindowError,
        Termination::Trailers,
        Termination::ReadClosed,
        Termination::IoError,
    ] {
        for consume_after_termination in [false, true] {
            for encode_before_drop in [false, true] {
                let mut flow = new_flow(1024);
                let id = StreamId::from(1);
                let mut pending = RecvWindow::ZERO;
                receive(&mut flow, 1, 128);
                assert_eq!(consume(&mut flow, 1, &mut pending), 128);
                receive(&mut flow, 1, 256);

                let mut credited_before_drop = 128;
                match termination {
                    Termination::PeerReset => assert!(flow.recv_reset(Reset::new(id, Reason::CANCEL)).is_ok()),
                    Termination::InternalReset => {
                        flow.internal_reset(&id);
                        // DATA received in the error state is discarded and
                        // credited immediately, alongside the older queued DATA.
                        receive(&mut flow, 1, 128);
                        credited_before_drop += 128;
                    }
                    Termination::StreamWindowError => {
                        // This frame exceeds stream credit but fits connection
                        // credit. It is rejected and immediately credited in full.
                        receive(&mut flow, 1, 1024);
                        credited_before_drop += 1024;
                    }
                    Termination::Trailers => {
                        let mut trailers = Headers::new(id, Pseudo::default(), HeaderMap::new());
                        trailers.set_end_stream();
                        assert!(flow.recv_header(id, trailers).is_ok());
                    }
                    Termination::ReadClosed => flow.reset_all_stream(&Ok(())),
                    Termination::IoError => flow.reset_all_stream(&Err(io::ErrorKind::BrokenPipe.into())),
                }

                // A reset does not drain earlier DATA. The body can still
                // consume that DATA, or leave it for Drop to release.
                if consume_after_termination {
                    let mut cx = Context::from_waker(Waker::noop());
                    assert!(matches!(
                        flow.poll_stream_frame(&id, &mut pending, &mut cx),
                        Poll::Ready(Some(Ok(Frame::Data(bytes)))) if bytes.len() == 256
                    ));
                    credited_before_drop += 256;
                }
                if encode_before_drop {
                    assert_connection_update(&mut flow, credited_before_drop);
                }
                let unread = if consume_after_termination { 0 } else { 256 };
                flow.request_body_drop(id);
                flow.response_task_done(id).unwrap();
                assert!(!flow.stream_map.contains_key(&id));
                assert_connection_update(
                    &mut flow,
                    unread + if encode_before_drop { 0 } else { credited_before_drop },
                );
                assert_eq!(flow.recv_connection_window.value(), 65_535);
            }
        }
    }
}

#[test]
fn stream_updates_keep_consumption_threshold() {
    let mut flow = new_flow(65_535);
    let mut pending = RecvWindow::ZERO;

    // Encoding after every body poll returns connection credit promptly,
    // while stream credit is still batched until the 75% threshold is met.
    for n in 0..48 {
        receive(&mut flow, 1, 1024);
        assert_eq!(consume(&mut flow, 1, &mut pending), 1024);
        let updates = window_updates(&mut flow);
        if n == 47 {
            assert_eq!(updates, [(1, 49_152), (0, 1024)]);
        } else {
            assert_eq!(updates, [(0, 1024)]);
        }
    }

    let mut flow = new_flow(1024);
    let mut pending = RecvWindow::ZERO;
    receive(&mut flow, 1, 768);
    assert_eq!(consume(&mut flow, 1, &mut pending), 768);
    assert_eq!(window_updates(&mut flow), [(1, 768), (0, 768)]);

    // A larger configured stream window also expands the connection window.
    // Check startup credit and threshold arithmetic at the protocol maximum.
    let flow = new_flow(settings::MAX_INITIAL_WINDOW_SIZE as u32);
    assert_eq!(flow.recv_connection_window.value(), 2_147_483_647);
    assert!(RecvWindow::new(1_610_612_733) == flow.recv_stream_threshold);
}

#[test]
fn consuming_body_wakes_writer_and_coalesces_wakeups() {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::Wake,
    };

    #[derive(Default)]
    struct WakeCount(AtomicUsize);

    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let wakes = Arc::new(WakeCount::default());
    let waker = Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    let mut buf = BytesMut::new();
    let mut flow = new_flow(65_535);
    let mut pending_a = RecvWindow::ZERO;
    let mut pending_b = RecvWindow::ZERO;

    assert!(flow.poll_encode(&mut buf, &mut cx).is_pending());
    receive(&mut flow, 1, 1024);
    assert_eq!(wakes.0.load(Ordering::Relaxed), 0);

    // A small consumption must wake the writer even when the body is polled
    // in a separate task and has not reached its stream threshold.
    assert_eq!(consume(&mut flow, 1, &mut pending_a), 1024);
    assert_eq!(wakes.0.load(Ordering::Relaxed), 1);
    receive(&mut flow, 3, 1024);
    assert_eq!(consume(&mut flow, 3, &mut pending_b), 1024);
    assert_eq!(wakes.0.load(Ordering::Relaxed), 1);
    assert_eq!(window_updates(&mut flow), [(0, 2048)]);

    // The next consumption wakes the writer again after it parks.
    assert!(flow.poll_encode(&mut buf, &mut cx).is_pending());
    receive(&mut flow, 1, 1024);
    assert_eq!(consume(&mut flow, 1, &mut pending_a), 1024);
    assert_eq!(wakes.0.load(Ordering::Relaxed), 2);
    assert_eq!(window_updates(&mut flow), [(0, 1024)]);

    // Dropping unread DATA outside the dispatcher must also wake the writer.
    assert!(flow.poll_encode(&mut buf, &mut cx).is_pending());
    receive(&mut flow, 3, 1024);
    flow.request_body_drop(StreamId::from(3));
    assert_eq!(wakes.0.load(Ordering::Relaxed), 3);
    assert_eq!(window_updates(&mut flow), [(0, 1024)]);
}

#[test]
fn body_drop_credit_can_share_the_startup_window_update() {
    let mut settings = Settings::default();
    settings.set_initial_window_size(Some(settings::MAX_INITIAL_WINDOW_SIZE as u32));
    settings.set_max_concurrent_streams(Some(2));
    let mut flow = FlowControl::new(&settings);
    flow.init(settings);

    // The peer can send DATA using its default credit before our initial
    // SETTINGS and connection window expansion have been encoded.
    let id = StreamId::from(1);
    let pseudo = Pseudo::request(Method::POST, "http://localhost/upload".parse().unwrap(), None);
    assert!(flow.recv_header(id, Headers::new(id, pseudo, HeaderMap::new())).is_ok());
    receive(&mut flow, 1, 4096);
    flow.request_body_drop(id);
    assert_connection_update(
        &mut flow,
        settings::MAX_INITIAL_WINDOW_SIZE as u32 - settings::DEFAULT_INITIAL_WINDOW_SIZE + 4096,
    );
    assert_eq!(
        flow.recv_connection_window.value(),
        settings::MAX_INITIAL_WINDOW_SIZE as u32
    );
    assert_connection_update(&mut flow, 0);
}
