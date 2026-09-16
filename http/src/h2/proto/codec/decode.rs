use core::mem;

use crate::bytes::{Buf, BytesMut};

use super::super::{
    error::Error as ProtoError,
    flow::{DecodedRequest, Error as FlowError, FlowControl},
    frame::{
        data::Data, go_away::GoAway, head, headers, ping::Ping, priority::Priority, reason::Reason, reset::Reset,
        settings, window_update::WindowUpdate,
    },
    hpack,
};

pub(crate) struct DecodeContext {
    max_frame_size: usize,
    max_header_list_size: usize,
    decoder: hpack::Decoder,
    next_frame_len: usize,
    continuation: Option<(headers::Headers, BytesMut)>,
}

impl DecodeContext {
    pub(crate) fn new(max_frame_size: usize, max_header_list_size: usize) -> Self {
        Self {
            max_frame_size,
            max_header_list_size,
            decoder: hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE),
            next_frame_len: 0,
            continuation: None,
        }
    }

    pub(crate) fn try_decode(
        &mut self,
        buf: &mut BytesMut,
        flow: &mut FlowControl,
    ) -> Result<Option<DecodedRequest>, FlowError> {
        loop {
            if self.next_frame_len == 0 {
                if buf.len() < 3 {
                    return Ok(None);
                }
                let payload_len = buf.get_uint(3) as usize;
                if payload_len > self.max_frame_size {
                    return Err(FlowError::GoAway(Reason::FRAME_SIZE_ERROR));
                }
                self.next_frame_len = payload_len + 6;
            }

            if buf.len() < self.next_frame_len {
                return Ok(None);
            }

            let len = mem::replace(&mut self.next_frame_len, 0);
            let mut frame = buf.split_to(len);
            let head = head::Head::parse(&frame);

            // TODO: Make Head::parse auto advance the frame?
            frame.advance(6);

            if let Some(decoded) = self.decode_frame(head, frame, flow)? {
                return Ok(Some(decoded));
            }
        }
    }

    fn decode_frame(
        &mut self,
        head: head::Head,
        frame: BytesMut,
        flow: &mut FlowControl,
    ) -> Result<Option<DecodedRequest>, FlowError> {
        match self._decode_frame(head, frame, flow) {
            Err(FlowError::Reset(reason)) => {
                flow.try_push_reset(head.stream_id(), reason)?;
                Ok(None)
            }
            res => res,
        }
    }

    fn _decode_frame(
        &mut self,
        head: head::Head,
        frame: BytesMut,
        flow: &mut FlowControl,
    ) -> Result<Option<DecodedRequest>, FlowError> {
        if self.continuation.is_some() && !matches!(head.kind(), head::Kind::Continuation) {
            return Err(FlowError::GoAway(Reason::PROTOCOL_ERROR));
        }

        match head.kind() {
            head::Kind::Headers => {
                let (headers, payload) = headers::Headers::load(head, frame)?;
                let is_end_headers = headers.is_end_headers();
                return self.handle_header(headers, payload, is_end_headers, flow);
            }
            head::Kind::Data => {
                let data = Data::load(head, frame.freeze())?;
                flow.recv_data(data)?;
            }
            head::Kind::WindowUpdate => {
                let window = WindowUpdate::load(head, frame.as_ref())?;
                flow.recv_window_update(window)?;
            }
            head::Kind::Ping => {
                let ping = Ping::load(head, frame.as_ref())?;
                flow.recv_ping(ping);
            }
            head::Kind::Reset => {
                let reset = Reset::load(head, frame.as_ref())?;
                flow.recv_reset(reset)?;
            }
            head::Kind::GoAway => {
                let go_away = GoAway::load(head.stream_id(), frame.as_ref())?;
                return Err(FlowError::GoAway(go_away.reason()));
            }
            head::Kind::Continuation => return self.handle_continuation(head, frame, flow),
            head::Kind::PushPromise => return Err(FlowError::GoAway(Reason::PROTOCOL_ERROR)),
            head::Kind::Priority => {
                Priority::load(head, &frame)?;
            }
            head::Kind::Settings => {
                let setting = settings::Settings::load(head, &frame)?;
                flow.recv_setting(setting)?;
            }
            head::Kind::Unknown => {}
        }
        Ok(None)
    }

    #[cold]
    #[inline(never)]
    fn handle_continuation(
        &mut self,
        head: head::Head,
        frame: BytesMut,
        flow: &mut FlowControl,
    ) -> Result<Option<DecodedRequest>, FlowError> {
        let is_end_headers = (head.flag() & 0x4) == 0x4;

        let (headers, mut payload) = self
            .continuation
            .take()
            .ok_or(FlowError::GoAway(Reason::PROTOCOL_ERROR))?;

        // RFC 9113 §6.10: CONTINUATION without a preceding incomplete HEADERS
        // is a connection error PROTOCOL_ERROR
        if headers.stream_id() != head.stream_id() {
            return Err(FlowError::GoAway(Reason::PROTOCOL_ERROR));
        }

        payload.unsplit(frame);
        self.handle_header(headers, payload, is_end_headers, flow)
    }

    fn handle_header(
        &mut self,
        mut headers: headers::Headers,
        mut payload: BytesMut,
        is_end_headers: bool,
        flow: &mut FlowControl,
    ) -> Result<Option<DecodedRequest>, FlowError> {
        if let Err(e) = headers.load_hpack(&mut payload, self.max_header_list_size, &mut self.decoder) {
            return match e {
                // NeedMore on a multi-frame header block is normal; accumulate and wait
                // for CONTINUATION frames (RFC 9113 §6.10).
                ProtoError::Hpack(hpack::DecoderError::NeedMore(_)) if !is_end_headers => {
                    self.continuation = Some((headers, payload));
                    Ok(None)
                }
                // Pseudo-header validation errors are stream-level (RFC 9113 §8.1.1).
                // The HPACK context was decoded successfully so no compression error.
                ProtoError::MalformedMessage => {
                    let id = headers.stream_id();
                    if flow.try_set_last_stream_id(id)?.is_none() {
                        return Ok(None);
                    }
                    Err(FlowError::Reset(Reason::PROTOCOL_ERROR))
                }
                _ => Err(FlowError::GoAway(Reason::COMPRESSION_ERROR)),
            };
        }

        if !is_end_headers {
            self.continuation = Some((headers, payload));
            return Ok(None);
        }

        let id = headers.stream_id();

        flow.recv_header(id, headers)
    }
}
