use core::{
    future::{Future, poll_fn},
    pin::Pin,
    task::{Context, Poll, Waker},
};

use std::{
    io::{self, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use xitca_io::{
    bytes::BytesMut,
    io::{AsyncIo, Interest, Ready},
};

use crate::{
    client::{Client, ClientBorrow},
    driver::generic::GenericDriver,
    iter::AsyncLendingIterator,
    session::Session,
};

use super::*;

#[derive(Clone, Default)]
struct MockIo(Arc<Mutex<BytesMut>>);

impl Read for MockIo {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut data = self.0.lock().unwrap();
        if data.is_empty() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data.split_to(n));
        Ok(n)
    }
}

impl Write for MockIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl AsyncIo for MockIo {
    async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
        poll_fn(|cx| self.poll_ready(interest, cx)).await
    }

    fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        let mut ready = Ready::EMPTY;
        if interest.is_readable() && !self.0.lock().unwrap().is_empty() {
            ready |= Ready::READABLE;
        }
        if interest.is_writable() {
            ready |= Ready::WRITABLE;
        }
        if ready == Ready::EMPTY {
            Poll::Pending
        } else {
            Poll::Ready(Ok(ready))
        }
    }

    fn is_vectored_write(&self) -> bool {
        false
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[derive(Clone, Default)]
struct Connector(Arc<ConnectorState>);

type TestDriver = (GenericDriver<MockIo>, MockIo);

#[derive(Default)]
struct ConnectorState {
    calls: AtomicUsize,
    // 0: succeed, 1: fail, 2: remain pending until the attempt is cancelled.
    mode: AtomicUsize,
    drivers: Mutex<Vec<Option<TestDriver>>>,
}

impl Connect for Connector {
    async fn connect(&self, cfg: Config) -> Result<Client, Error> {
        self.0.calls.fetch_add(1, Ordering::Relaxed);
        match self.0.mode.load(Ordering::Relaxed) {
            1 => return Err(Error::todo()),
            2 => core::future::pending::<()>().await,
            _ => {}
        }
        let io = MockIo::default();
        let (drv, tx) = GenericDriver::new(io.clone(), cfg.get_max_in_flight_requests());
        let mut drivers = self.0.drivers.lock().unwrap();
        drivers.push(Some((drv, io)));
        Ok(Client::new(
            tx,
            Session {
                id: drivers.len() as i32,
                key: 0,
                info: Default::default(),
            },
        ))
    }
}

impl Connector {
    fn builder(&self, capacity: usize) -> PoolBuilder {
        let mut cfg = Config::new();
        cfg.max_in_flight_requests(1);
        Pool::builder(cfg).capacity(capacity).connector(self.clone())
    }

    fn calls(&self) -> usize {
        self.0.calls.load(Ordering::Relaxed)
    }

    fn complete(&self, id: i32) {
        let mut drivers = self.0.drivers.lock().unwrap();
        let (drv, io) = drivers[id as usize - 1].as_mut().unwrap();
        io.0.lock().unwrap().extend_from_slice(b"Z\0\0\0\x05I");
        let mut drive = core::pin::pin!(drv.try_next());
        assert!(
            drive
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }

    fn close(&self, id: i32) {
        self.0.drivers.lock().unwrap()[id as usize - 1].take();
    }
}

fn saturate<P: PermitLike>(conn: &GenericPoolConnection<P>) {
    conn.borrow_cli_ref()
        .tx
        .try_send(|buf| {
            buf.extend_from_slice(b"request");
            Ok(())
        })
        .unwrap();
}

async fn no_hang<F: Future>(fut: F) -> F::Output {
    tokio::time::timeout(core::time::Duration::from_secs(5), fut)
        .await
        .expect("checkout remained blocked")
}

#[tokio::test]
async fn checkout_prefers_capacity_and_only_creates_when_queue_is_empty() {
    let connector = Connector::default();
    let pool = connector.builder(2).build().unwrap();
    let conn = pool.get().await.unwrap();
    assert_eq!(conn.cancel_token().id, 1);
    saturate(&conn);
    drop(conn);

    let conn = pool.get().await.unwrap();
    assert_eq!(conn.cancel_token().id, 1);
    assert_eq!(connector.calls(), 1, "saturation alone must not create a connection");
    // Holding the first checkout leaves the queue empty, so another checkout may connect.
    let second = pool.get().await.unwrap();
    assert_eq!(second.cancel_token().id, 2);
    drop(conn);
    drop(second);
    // Connection 1 is still first in the queue and still busy; connection 2 is reusable.
    let conn = pool.get().await.unwrap();
    assert_eq!(conn.cancel_token().id, 2);
    assert_eq!(connector.calls(), 2);
}

#[tokio::test]
async fn saturated_checkout_rotates_without_exceeding_connection_limit() {
    for capacity in 1..=6 {
        let connector = Connector::default();
        let pool = connector.builder(capacity).build().unwrap();
        let mut connections = Vec::new();
        for id in 1..=capacity {
            let conn = pool.get().await.unwrap();
            assert_eq!(conn.cancel_token().id, id as i32);
            saturate(&conn);
            connections.push(conn);
        }
        for conn in connections {
            drop(conn);
        }

        // Immediate returns join the back, giving each saturated connection one turn per cycle.
        for id in (1..=capacity).cycle().take(capacity * 8) {
            let conn = no_hang(pool.get()).await.unwrap();
            assert_eq!(conn.cancel_token().id, id as i32);
            assert!(!conn.borrow_cli_ref().tx.has_capacity());
        }

        // Holding checkouts must preserve the same order among the remaining candidates.
        let mut held = Vec::new();
        for id in 1..=capacity {
            let conn = no_hang(pool.get()).await.unwrap();
            assert_eq!(conn.cancel_token().id, id as i32);
            held.push(conn);
        }
        // FIFO follows return order when checkouts finish out of order.
        while let Some(conn) = held.pop() {
            drop(conn);
        }
        for id in (1..=capacity).rev().cycle().take(capacity * 2) {
            let conn = no_hang(pool.get()).await.unwrap();
            assert_eq!(conn.cancel_token().id, id as i32);
        }
        assert_eq!(connector.calls(), capacity);
    }
}

#[tokio::test]
async fn owned_checkout_uses_completed_capacity_and_replaces_closed_connections() {
    let connector = Connector::default();
    let pool = connector.builder(2).build_owned().unwrap();
    let first = pool.get().await.unwrap();
    let second = pool.get().await.unwrap();
    saturate(&first);
    saturate(&second);
    drop(first);
    drop(second);
    connector.complete(2);
    let conn = pool.get().await.unwrap();
    assert_eq!(conn.cancel_token().id, 2);
    connector.close(2);
    drop(conn);
    let busy = pool.get().await.unwrap();
    assert_eq!(busy.cancel_token().id, 1);
    let conn = pool.get().await.unwrap();
    assert_eq!(conn.cancel_token().id, 3);
    drop(busy);
    drop(conn);
    connector.close(1);
    let conn = pool.get().await.unwrap();
    assert_eq!(conn.cancel_token().id, 3);
    assert_eq!(connector.calls(), 3);
}

#[tokio::test]
async fn all_checked_out_waits_for_return_without_opening_another_connection() {
    let connector = Connector::default();
    let pool = connector.builder(2).build().unwrap();
    let first = pool.get().await.unwrap();
    let second = pool.get().await.unwrap();
    let mut waiting = Box::pin(pool.get());
    assert!(poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx))).await.is_pending());
    assert_eq!(connector.calls(), 2);
    drop(first);
    let conn = no_hang(waiting).await.unwrap();
    assert_eq!(conn.cancel_token().id, 1);
    drop(second);
}

#[tokio::test]
async fn failed_opening_returns_connection_and_checkout_capacity() {
    let connector = Connector::default();
    let pool = connector.builder(1).build().unwrap();
    connector.0.mode.store(1, Ordering::Relaxed);
    assert!(pool.get().await.is_err());
    assert_eq!(pool.permits.available_permits(), 1);
    connector.0.mode.store(0, Ordering::Relaxed);
    let _conn = no_hang(pool.get()).await.unwrap();
    assert_eq!(connector.calls(), 2);
}

#[tokio::test]
async fn cancelled_openings_return_capacity_and_bound_concurrent_creation() {
    let connector = Connector::default();
    let pool = connector.builder(2).build_owned().unwrap();
    connector.0.mode.store(2, Ordering::Relaxed);
    let mut first = Box::pin(pool.get());
    let mut second = Box::pin(pool.get());
    let mut third = Box::pin(pool.get());
    for fut in [&mut first, &mut second, &mut third] {
        assert!(poll_fn(|cx| Poll::Ready(fut.as_mut().poll(cx))).await.is_pending());
    }
    assert_eq!(connector.calls(), 2);
    drop(first);
    connector.0.mode.store(0, Ordering::Relaxed);
    let conn = no_hang(third).await.unwrap();
    drop(second);
    drop(conn);
    assert_eq!(pool.permits.available_permits(), 2);
    assert_eq!(connector.calls(), 3);
}

#[tokio::test]
async fn saturated_fifo_handles_checkouts_and_closed_candidates() {
    let connector = Connector::default();
    let pool = connector.builder(4).build_owned().unwrap();
    let mut connections = Vec::new();
    for _ in 0..4 {
        let conn = pool.get().await.unwrap();
        saturate(&conn);
        connections.push(conn);
    }
    for conn in connections {
        drop(conn);
    }

    let first = pool.get().await.unwrap();
    let second = pool.get().await.unwrap();
    assert_eq!(first.cancel_token().id, 1);
    assert_eq!(second.cancel_token().id, 2);
    connector.close(3);
    let last = pool.get().await.unwrap();
    assert_eq!(last.cancel_token().id, 4);
    drop(last);
    drop(second);
    drop(first);

    let mut counts = [0; 4];
    for _ in 0..30 {
        let conn = pool.get().await.unwrap();
        counts[conn.cancel_token().id as usize - 1] += 1;
    }
    assert_eq!(counts, [10, 10, 0, 10]);
    assert_eq!(connector.calls(), 4);
}

#[tokio::test]
async fn ready_checkout_preserves_saturated_fifo_order() {
    let connector = Connector::default();
    let pool = connector.builder(4).build().unwrap();
    let mut connections = Vec::new();
    for _ in 0..4 {
        let conn = pool.get().await.unwrap();
        saturate(&conn);
        connections.push(conn);
    }
    for conn in connections {
        drop(conn);
    }
    connector.complete(2);
    for _ in 0..3 {
        let conn = pool.get().await.unwrap();
        assert_eq!(conn.cancel_token().id, 2);
    }
    {
        let conn = pool.get().await.unwrap();
        saturate(&conn);
    }

    // Skipping connection 1 to select the ready connection must not move it behind 3 and 4.
    for id in [1, 3, 4, 2].into_iter().cycle().take(12) {
        let conn = pool.get().await.unwrap();
        assert_eq!(conn.cancel_token().id, id);
    }
}

#[tokio::test]
async fn saturated_send_waits_and_cancellation_preserves_checkout_capacity() {
    let connector = Connector::default();
    let pool = connector.builder(1).build_owned().unwrap();
    {
        let conn = pool.get().await.unwrap();
        saturate(&conn);
    }
    {
        let conn = no_hang(pool.get()).await.unwrap();
        let mut send = Box::pin(
            conn.borrow_cli_ref()
                .tx
                .send::<_, ()>(|_| panic!("waiting send encoded")),
        );
        assert!(poll_fn(|cx| Poll::Ready(send.as_mut().poll(cx))).await.is_pending());
    }
    let conn = no_hang(pool.get()).await.unwrap();
    let mut send = Box::pin(conn.borrow_cli_ref().tx.send(|buf| {
        buf.extend_from_slice(b"next");
        Ok(())
    }));
    assert!(poll_fn(|cx| Poll::Ready(send.as_mut().poll(cx))).await.is_pending());
    connector.complete(1);
    no_hang(send).await.unwrap();
    assert_eq!(connector.calls(), 1);
}
