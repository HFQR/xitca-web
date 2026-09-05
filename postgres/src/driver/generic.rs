use core::{
    future::{Future, poll_fn},
    ops::Deref,
    pin::Pin,
    task::{Poll, Waker},
};

use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
};

use tokio::sync::{Semaphore, SemaphorePermit};
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{
    error::{DriverBusy, DriverDown, Error, unexpected_eof_err},
    iter::AsyncLendingIterator,
    protocol::message::{backend, frontend},
};

use super::codec::{Response, ResponseMessage, ResponseSender};

type PagedBytesMut = xitca_unsafe_collection::bytes::PagedBytesMut<4096>;

const INTEREST_READ_WRITE: Interest = Interest::READABLE.add(Interest::WRITABLE);

pub(crate) struct DriverTx(Arc<SharedState>);

impl Drop for DriverTx {
    fn drop(&mut self) {
        let mut state = self.0.guarded.lock().unwrap();
        if !state.closed {
            frontend::terminate(&mut state.buf);
            state.closed = true;
            state.wake();
        }
    }
}

impl DriverTx {
    pub(crate) fn is_closed(&self) -> bool {
        self.0.sem.is_closed()
    }

    // A scheduling hint only. send still acquires a permit and checks for closure.
    pub(crate) fn has_capacity(&self) -> bool {
        self.0.sem.available_permits() != 0
    }

    pub(crate) fn try_send<F, O>(&self, func: F) -> Result<(O, Response), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<O, Error>,
    {
        // sync caller can not wait for an outstanding request to complete.
        let permit = self.0.sem.try_acquire().map_err(|e| match e {
            tokio::sync::TryAcquireError::Closed => Error::from(DriverDown),
            tokio::sync::TryAcquireError::NoPermits => Error::from(DriverBusy),
        })?;
        self._send(Some(permit), func, response_pair)
    }

    /// waits for request capacity instead of failing when the driver is backed up. a caller that goes
    /// away while waiting never encodes its request so the query is not sent to database.
    pub(crate) async fn send<F, O>(&self, func: F) -> Result<(O, Response), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<O, Error>,
    {
        let permit = self.0.sem.acquire().await.map_err(|_| Error::from(DriverDown))?;
        self._send(Some(permit), func, response_pair)
    }

    /// send a request without taking a queue slot.
    ///
    /// it's for message that complete a protocol state already in progress: transaction rollback,
    /// statement close, copy done and copy fail. they are issued from `Drop` or from a completion
    /// path where failing would leave the server side in a broken state. a queue limit must not
    /// apply to them: dropping a `ROLLBACK` leaks a transaction and dropping a `CopyDone` leaves
    /// the connection stuck in copy mode.
    pub(crate) fn send_unbounded<F, O>(&self, func: F) -> Result<(O, Response), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<O, Error>,
    {
        self._send(None, func, response_pair)
    }

    /// [`DriverTx::send_unbounded`] for request that expect no response.
    pub(crate) fn send_one_way_unbounded<F>(&self, func: F) -> Result<(), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self._send(None, func, |_, _| {})?;
        Ok(())
    }

    fn _send<F, F2, O, T>(&self, permit: Option<SemaphorePermit<'_>>, func: F, on_send: F2) -> Result<(O, T), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<O, Error>,
        F2: FnOnce(&mut State, bool) -> T,
    {
        let mut inner = self.0.guarded.lock().unwrap();

        if inner.closed {
            return Err(DriverDown.into());
        }

        let len = inner.buf.len();

        let o = func(&mut inner.buf).inspect_err(|_| inner.buf.truncate(len))?;
        let t = on_send(&mut inner, permit.is_some());

        if let Some(permit) = permit {
            permit.forget();
        }
        inner.wake();

        Ok((o, t))
    }
}

fn response_pair(inner: &mut State, counted: bool) -> Response {
    let (tx, rx) = super::codec::request_pair();
    inner.res.push_back(PendingResponse { sender: tx, counted });
    rx
}

pub(crate) struct DriverRx(Arc<SharedState>);

impl Deref for DriverRx {
    type Target = SharedState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// in case driver is dropped without closing the shared state
impl Drop for DriverRx {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(crate) struct SharedState {
    pub(super) guarded: Mutex<State>,
    // slots for requests queued or awaiting a complete protocol response. send forgets its
    // borrowed permit; removing a counted response returns it, independent of socket writes.
    pub(super) sem: Semaphore,
}

impl SharedState {
    // DriverTx can outlive DriverRx and keeps the semaphore alive. close it explicitly so
    // callers waiting for a queue slot observe DriverDown even when every permit is in use.
    pub(super) fn close(&self) {
        self.guarded.lock().unwrap().close();
        self.sem.close();
    }

    pub(super) fn shutdown(&self) {
        self.sem.close();
        self.guarded.lock().unwrap().shutdown();
    }

    pub(super) fn wait(&self) -> impl Future<Output = WriteState> + use<'_> {
        poll_fn(|cx| {
            let mut inner = self.guarded.lock().unwrap();
            if !inner.buf.is_empty() {
                Poll::Ready(WriteState::WantWrite)
            } else if inner.closed {
                Poll::Ready(WriteState::Finished)
            } else {
                inner.register(cx.waker());
                Poll::Pending
            }
        })
    }
}

// Unbounded cleanup also expects responses, but must not return a permit it never acquired.
pub(super) struct PendingResponse {
    sender: ResponseSender,
    counted: bool,
}

pub(super) struct State {
    pub(super) closed: bool,
    pub(super) buf: BytesMut,
    pub(super) res: VecDeque<PendingResponse>,
    pub(super) waker: Option<Waker>,
}

impl State {
    // stop the shared state from accepting new request. pending requests are kept so message
    // already received can still be dispatched to them before the driver shuts down.
    pub(super) fn close(&mut self) {
        self.closed = true;
        self.buf.clear();
        self.wake();
    }

    // final shutdown of shared state. pending requests are dropped so their response receiver
    // observes an unfinished response.
    pub(super) fn shutdown(&mut self) {
        self.close();
        self.res.clear();
    }

    fn register(&mut self, waker: &Waker) {
        self.waker = Some(waker.clone());
    }

    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

pub struct GenericDriver<Io> {
    pub(super) io: Io,
    pub(super) read_buf: PagedBytesMut,
    pub(super) rx: DriverRx,
    read_state: ReadState,
    write_state: WriteState,
}

pub(super) enum WriteState {
    Waiting,
    WantWrite,
    WantFlush,
    // client dropped its DriverTx and every queued request has been flushed. write half has no
    // more work to do but the connection is not finished: read half keeps draining until remote
    // closes it. it's distinct from Closed which means the write half itself is done.
    Finished,
    Closed(Option<io::Error>),
}

enum ReadState {
    WantRead,
    Closed(Option<io::Error>),
}

impl<Io> GenericDriver<Io>
where
    Io: AsyncIo + Send,
{
    pub(crate) fn new(io: Io, max_in_flight_requests: usize) -> (Self, DriverTx) {
        let state = Arc::new(SharedState {
            guarded: Mutex::new(State {
                closed: false,
                buf: BytesMut::new(),
                res: VecDeque::new(),
                waker: None,
            }),
            sem: Semaphore::new(max_in_flight_requests),
        });

        (
            Self {
                io,
                rx: DriverRx(state.clone()),
                read_buf: PagedBytesMut::new(),
                read_state: ReadState::WantRead,
                write_state: WriteState::Waiting,
            },
            DriverTx(state),
        )
    }

    // session preparation writes and reads directly without taking a request permit.
    // DriverTx is not shared with a Client at this point.
    pub(crate) async fn send(&mut self, msg: BytesMut) -> Result<(), Error> {
        self.rx.guarded.lock().unwrap().buf.extend_from_slice(&msg);
        self.write_state = WriteState::WantWrite;
        loop {
            self.try_write()?;
            if matches!(self.write_state, WriteState::Waiting) {
                return Ok(());
            }
            self.io.ready(Interest::WRITABLE).await?;
        }
    }

    pub(crate) async fn recv(&mut self) -> Result<backend::Message, Error> {
        loop {
            if let Some(msg) = backend::Message::parse(self.read_buf.get_mut())? {
                return Ok(msg);
            }
            self.io.ready(Interest::READABLE).await?;
            if let Some(0) = self.try_read()? {
                return Err(Error::from(io::Error::from(io::ErrorKind::UnexpectedEof)));
            }
        }
    }

    async fn _try_next(&mut self) -> Result<Option<backend::Message>, Error> {
        loop {
            if let Some(msg) = self.rx.try_decode(self.read_buf.get_mut())? {
                return Ok(Some(msg));
            }

            let res = match (&mut self.read_state, &mut self.write_state) {
                (ReadState::WantRead, WriteState::Waiting) => {
                    self.io.ready(Interest::READABLE).select(self.rx.wait()).await
                }
                (ReadState::WantRead, WriteState::WantWrite | WriteState::WantFlush) => {
                    SelectOutput::A(self.io.ready(INTEREST_READ_WRITE).await)
                }
                // client asked for shutdown after Terminate was flushed. read half is still open
                // and drains until remote closes the connection.
                (ReadState::WantRead, WriteState::Finished) => SelectOutput::A(self.io.ready(Interest::READABLE).await),
                (ReadState::Closed(read_err), WriteState::Closed(write_err)) => {
                    // every message that could be decoded has been dispatched by now. requests
                    // still pending can never be answered.
                    self.rx.shutdown();
                    return match (read_err, write_err) {
                        // decode above consumed every complete message so a non empty read buffer
                        // can only hold a partial one. remote closed in the middle of it.
                        (None, None) if !self.read_buf.get_mut().is_empty() => {
                            Err(Error::driver_io(Some(unexpected_eof_err()), None))
                        }
                        (None, None) => {
                            poll_fn(|cx| Pin::new(&mut self.io).poll_shutdown(cx)).await?;
                            Ok(None)
                        }
                        (read_err, write_err) => Err(Error::driver_io(read_err.take(), write_err.take())),
                    };
                }
                _ => unreachable!(),
            };

            match res {
                SelectOutput::A(res) => {
                    // AsyncIo::ready only errors on runtime shutdown, when no further IO is
                    // possible. abort immediately and leave cleanup to driver drop. connection
                    // errors must come from Read/Write and follow the close paths below.
                    let ready = res?;
                    if ready.is_readable() {
                        let state = match self.try_read() {
                            Ok(Some(0)) => ReadState::Closed(None),
                            Ok(_) => ReadState::WantRead,
                            Err(err) => ReadState::Closed(Some(err)),
                        };

                        if let ReadState::Closed(reason) = state {
                            self.on_read_close(reason);
                            continue;
                        }
                    }

                    if ready.is_writable()
                        && let Err(e) = self.try_write()
                    {
                        self.on_write_err(e);
                    }
                }
                SelectOutput::B(write_state) => self.write_state = write_state,
            }
        }
    }

    fn try_read(&mut self) -> io::Result<Option<usize>> {
        let mut read = 0;
        loop {
            match xitca_unsafe_collection::bytes::read_buf(&mut self.io, &mut self.read_buf) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(_) if read != 0 => break,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) => return Err(e),
            }
        }
        Ok(Some(read))
    }

    fn try_write(&mut self) -> io::Result<()> {
        debug_assert!(
            matches!(self.write_state, WriteState::WantWrite | WriteState::WantFlush),
            "try_write must not be called when WriteState is Waiting, Shutdown or Closed"
        );

        if matches!(self.write_state, WriteState::WantWrite) {
            let mut inner = self.rx.guarded.lock().unwrap();

            let mut written = 0;

            let res = loop {
                match io::Write::write(&mut self.io, &inner.buf[written..]) {
                    Ok(0) => break Err(io::Error::from(io::ErrorKind::WriteZero)),
                    Ok(n) => {
                        written += n;
                        if written == inner.buf.len() {
                            break Ok(true);
                        }
                    }
                    Err(e) => break Err(e),
                }
            };

            if matches!(res, Ok(true)) {
                inner.buf.clear();
            } else {
                inner.buf.advance(written);
            }

            drop(inner);

            match res {
                Ok(_) => self.write_state = WriteState::WantFlush,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
        }

        match io::Write::flush(&mut self.io) {
            Ok(_) => self.write_state = WriteState::Waiting,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }

        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn on_read_close(&mut self, reason: Option<io::Error>) {
        self.rx.close();
        self.read_state = ReadState::Closed(reason);
        self.on_close();
    }

    #[cold]
    #[inline(never)]
    fn on_write_err(&mut self, e: io::Error) {
        self.rx.close();
        self.write_state = WriteState::Closed(Some(e));
        self.on_close();
    }

    fn on_close(&mut self) {
        if !matches!(self.read_state, ReadState::Closed(_)) {
            self.read_state = ReadState::Closed(None);
        }
        if !matches!(self.write_state, WriteState::Closed(_)) {
            self.write_state = WriteState::Closed(None);
        }
    }
}

impl DriverRx {
    pub(super) fn try_decode(&self, read_buf: &mut BytesMut) -> Result<Option<backend::Message>, Error> {
        let mut guard = None;
        let mut completed = 0;

        let result = loop {
            match ResponseMessage::try_from_buf(read_buf) {
                Ok(Some(ResponseMessage::Normal(mut msg))) => {
                    // lock the shared state only when needed and keep the lock around a bit for possible multiple messages
                    let inner = guard.get_or_insert_with(|| self.guarded.lock().unwrap());

                    let Some(res) = inner.res.pop_front() else {
                        break Err(msg.parse_error());
                    };

                    let _ = res.sender.send(msg.buf);
                    completed += usize::from(msg.complete & res.counted);

                    if !msg.complete {
                        inner.res.push_front(res);
                    }
                }
                Ok(Some(ResponseMessage::Async(msg))) => break Ok(Some(msg)),
                Ok(None) => break Ok(None),
                Err(e) => break Err(e),
            }
        };

        // Also return completed slots when a later message is asynchronous or malformed.
        // Wake senders after releasing the state lock they need to enqueue their requests.
        drop(guard);
        self.sem.add_permits(completed);

        result
    }
}

impl<Io> AsyncLendingIterator for GenericDriver<Io>
where
    Io: AsyncIo + Send,
{
    type Ok<'i>
        = backend::Message
    where
        Self: 'i;
    type Err = Error;

    #[inline]
    fn try_next(&mut self) -> impl Future<Output = Result<Option<Self::Ok<'_>>, Self::Err>> + Send {
        self._try_next()
    }
}

#[cfg(test)]
mod test {
    use core::task::Context;

    use std::io::{Read, Write};

    use xitca_io::io::Ready;

    use crate::error::{DbError, DriverBusy, Severity, SqlState};

    use super::*;

    // remote closed the connection. read reports eof and write is still accepted by the kernel
    // until a reset is observed.
    struct EofIo;

    impl Read for EofIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for EofIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncIo for EofIo {
        async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
            poll_fn(|cx| self.poll_ready(interest, cx)).await
        }

        fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            let mut ready = Ready::EMPTY;
            if interest.is_readable() {
                ready |= Ready::READABLE;
            }
            if interest.is_writable() {
                ready |= Ready::WRITABLE;
            }
            Poll::Ready(Ok(ready))
        }

        fn is_vectored_write(&self) -> bool {
            false
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    // connection is reset on write while read has nothing to offer.
    struct WriteErrIo;

    impl Read for WriteErrIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    impl Write for WriteErrIo {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::ConnectionReset.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncIo for WriteErrIo {
        async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
            poll_fn(|cx| self.poll_ready(interest, cx)).await
        }

        fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            // nothing is ever readable. only write readiness resolves.
            if interest.is_writable() {
                Poll::Ready(Ok(Ready::WRITABLE))
            } else {
                Poll::Pending
            }
        }

        fn is_vectored_write(&self) -> bool {
            false
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    // no data is readable until the driver has a request to write. it puts the driver in
    // (WantRead, WantWrite) where a single readiness reports both halves at once.
    struct QueuedWriteEofIo {
        readable: bool,
    }

    impl Read for QueuedWriteEofIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for QueuedWriteEofIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncIo for QueuedWriteEofIo {
        async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
            poll_fn(|cx| self.poll_ready(interest, cx)).await
        }

        fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            if !self.readable {
                // remote closes after the request is queued for write.
                self.readable = true;
                return Poll::Pending;
            }
            let mut ready = Ready::READABLE;
            if interest.is_writable() {
                ready |= Ready::WRITABLE;
            }
            Poll::Ready(Ok(ready))
        }

        fn is_vectored_write(&self) -> bool {
            false
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    // a CommandComplete header claiming 10 bytes of body followed by nothing.
    struct PartialThenEofIo {
        sent: bool,
    }

    impl Read for PartialThenEofIo {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.sent {
                return Ok(0);
            }
            self.sent = true;
            let msg = b"C\x00\x00\x00\x0a";
            buf[..msg.len()].copy_from_slice(msg);
            Ok(msg.len())
        }
    }

    impl Write for PartialThenEofIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncIo for PartialThenEofIo {
        async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
            poll_fn(|cx| self.poll_ready(interest, cx)).await
        }

        fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            let mut ready = Ready::READABLE;
            if interest.is_writable() {
                ready |= Ready::WRITABLE;
            }
            Poll::Ready(Ok(ready))
        }

        fn is_vectored_write(&self) -> bool {
            false
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    // a FATAL ErrorResponse followed by the connection closing, as an administrator commanded
    // shutdown does.
    struct FatalThenEofIo {
        sent: bool,
    }

    fn error_response() -> Vec<u8> {
        let mut body = Vec::new();
        for (ty, value) in [
            (b'S', "FATAL"),
            (b'V', "FATAL"),
            (b'C', "57P01"),
            (b'M', "terminating connection due to administrator command"),
        ] {
            body.push(ty);
            body.extend_from_slice(value.as_bytes());
            body.push(0);
        }
        body.push(0);

        let mut frame = vec![b'E'];
        frame.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_be_bytes());
        frame.extend_from_slice(&body);
        frame
    }

    impl Read for FatalThenEofIo {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.sent {
                return Ok(0);
            }
            self.sent = true;
            let frame = error_response();
            buf[..frame.len()].copy_from_slice(&frame);
            Ok(frame.len())
        }
    }

    impl Write for FatalThenEofIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncIo for FatalThenEofIo {
        async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
            poll_fn(|cx| self.poll_ready(interest, cx)).await
        }

        fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            let mut ready = Ready::READABLE;
            if interest.is_writable() {
                ready |= Ready::WRITABLE;
            }
            Poll::Ready(Ok(ready))
        }

        fn is_vectored_write(&self) -> bool {
            false
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    // nothing is readable until the driver has written and polled read once more. it lets the
    // driver reach (WantRead, WriteState::Shutdown) after the client queued its Terminate.
    struct ClientShutdownIo {
        wrote: bool,
        delay: u8,
    }

    impl Read for ClientShutdownIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for ClientShutdownIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.wrote = true;
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncIo for ClientShutdownIo {
        async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
            poll_fn(|cx| self.poll_ready(interest, cx)).await
        }

        fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            let mut ready = Ready::EMPTY;
            if interest.is_writable() {
                ready |= Ready::WRITABLE;
            }
            if interest.is_readable() && self.wrote {
                if self.delay > 0 {
                    self.delay -= 1;
                } else {
                    ready |= Ready::READABLE;
                }
            }
            if ready.is_empty() {
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

    // accepts every write. nothing is ever readable so the driver parks after flushing.
    struct WriteOkIo;

    impl Read for WriteOkIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    impl Write for WriteOkIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncIo for WriteOkIo {
        async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
            poll_fn(|cx| self.poll_ready(interest, cx)).await
        }

        fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            if interest.is_writable() {
                Poll::Ready(Ok(Ready::WRITABLE))
            } else {
                Poll::Pending
            }
        }

        fn is_vectored_write(&self) -> bool {
            false
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    // accepts a fixed number of bytes and then stops being writable.
    struct PartialWriteIo {
        budget: usize,
    }

    impl Read for PartialWriteIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    impl Write for PartialWriteIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let n = buf.len().min(self.budget);
            self.budget -= n;
            if n == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncIo for PartialWriteIo {
        async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
            poll_fn(|cx| self.poll_ready(interest, cx)).await
        }

        fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            if interest.is_writable() && self.budget > 0 {
                Poll::Ready(Ok(Ready::WRITABLE))
            } else {
                Poll::Pending
            }
        }

        fn is_vectored_write(&self) -> bool {
            false
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    // counts how many write calls the driver makes so batching can be observed.
    #[derive(Clone)]
    struct CountWriteIo(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl CountWriteIo {
        fn new() -> Self {
            Self(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))
        }

        fn writes(&self) -> usize {
            self.0.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl Read for CountWriteIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    impl Write for CountWriteIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncIo for CountWriteIo {
        async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
            poll_fn(|cx| self.poll_ready(interest, cx)).await
        }

        fn poll_ready(&mut self, interest: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            if interest.is_writable() {
                Poll::Ready(Ok(Ready::WRITABLE))
            } else {
                Poll::Pending
            }
        }

        fn is_vectored_write(&self) -> bool {
            false
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    const REQUEST: &[u8] = b"request";
    const READY: &[u8] = b"Z\x00\x00\x00\x05I";

    fn backend_message(tag: u8, body: &[u8]) -> BytesMut {
        let mut msg = BytesMut::new();
        msg.extend_from_slice(&[tag]);
        msg.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        msg.extend_from_slice(body);
        msg
    }

    fn complete<Io>(drv: &GenericDriver<Io>, count: usize) {
        let mut buf = BytesMut::new();
        for _ in 0..count {
            buf.extend_from_slice(READY);
        }
        assert!(drv.rx.try_decode(&mut buf).unwrap().is_none());
        assert!(buf.is_empty());
    }

    fn assert_busy(res: Result<Response, Error>) {
        match res {
            Ok(_) => panic!("expected a driver busy error"),
            Err(e) => assert!(e.downcast_ref::<DriverBusy>().is_some(), "unexpected error: {e}"),
        }
    }

    // give the driver a chance to flush. it parks afterwards with nothing left to do.
    async fn drive_once<Io>(drv: &mut GenericDriver<Io>)
    where
        Io: AsyncIo + Send,
    {
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), drv.try_next()).await;
    }

    fn request(tx: &DriverTx) -> Result<Response, Error> {
        tx.try_send(|buf| {
            buf.extend_from_slice(REQUEST);
            Ok(())
        })
        .map(|(_, res)| res)
    }

    fn assert_driver_down(res: Result<backend::Message, Error>) {
        match res {
            Ok(_) => panic!("expected a driver down error"),
            Err(e) => assert!(e.is_driver_down(), "unexpected error: {e}"),
        }
    }

    fn assert_io_err(res: Result<Option<backend::Message>, Error>, kind: io::ErrorKind) {
        match res {
            Ok(_) => panic!("expected {kind:?} error"),
            Err(e) => assert_eq!(e.downcast_ref::<io::Error>().expect("not an io error").kind(), kind),
        }
    }

    // the driver must not park on a closed connection. every await is bounded so a regression
    // fails the test instead of hanging it.
    async fn no_hang<F>(fut: F) -> F::Output
    where
        F: Future,
    {
        tokio::time::timeout(std::time::Duration::from_secs(5), fut)
            .await
            .expect("driver parked on a closed connection")
    }

    // client drop queues Terminate and closes the shared state while the read half is still
    // open. the driver writes it out then drains read until remote closes the connection.
    #[tokio::test]
    async fn client_shutdown_drains_until_remote_close() {
        let (mut drv, tx) = GenericDriver::new(ClientShutdownIo { wrote: false, delay: 1 }, 1024);

        // nothing to read and nothing to write. driver parks.
        let idle = std::time::Duration::from_millis(100);
        assert!(tokio::time::timeout(idle, drv.try_next()).await.is_err());

        drop(tx);

        assert!(no_hang(drv.try_next()).await.unwrap().is_none());
    }

    // a connection that stops draining must not grow the write buffer without bound.
    #[tokio::test]
    async fn queued_request_limit_is_enforced() {
        let (_drv, tx) = GenericDriver::new(WriteOkIo, 2);

        let _r1 = request(&tx).unwrap();
        let _r2 = request(&tx).unwrap();
        assert_busy(request(&tx));
    }

    #[tokio::test]
    async fn permit_is_released_only_after_response_completion() {
        let (mut drv, tx) = GenericDriver::new(WriteOkIo, 2);

        let _r1 = request(&tx).unwrap();
        let _r2 = request(&tx).unwrap();
        assert_busy(request(&tx));

        drive_once(&mut drv).await;

        assert!(drv.rx.guarded.lock().unwrap().buf.is_empty());
        assert_busy(request(&tx));

        complete(&drv, 2);
        let _r3 = request(&tx).unwrap();
        let _r4 = request(&tx).unwrap();
        assert_busy(request(&tx));
    }

    // socket writes, including complete requests in a partial batch, never return slots.
    #[tokio::test]
    async fn partial_write_retains_request_slots() {
        let budget = REQUEST.len() + 1;
        let (mut drv, tx) = GenericDriver::new(PartialWriteIo { budget }, 3);

        let _r1 = request(&tx).unwrap();
        let _r2 = request(&tx).unwrap();
        let _r3 = request(&tx).unwrap();
        assert_busy(request(&tx));

        drive_once(&mut drv).await;

        assert_eq!(drv.rx.guarded.lock().unwrap().buf.len(), REQUEST.len() * 3 - budget);
        assert_busy(request(&tx));
        complete(&drv, 1);
        let _r4 = request(&tx).unwrap();
        assert_busy(request(&tx));
    }

    // teardown message must reach the server even with the queue full: a dropped ROLLBACK leaks
    // a transaction and a dropped CopyDone leaves the connection stuck in copy mode.
    #[tokio::test]
    async fn teardown_send_bypasses_the_limit() {
        let io = CountWriteIo::new();
        let (mut drv, tx) = GenericDriver::new(io.clone(), 1);

        let _r1 = request(&tx).unwrap();
        assert_busy(request(&tx));

        tx.send_one_way_unbounded(|buf| {
            buf.extend_from_slice(b"rollback");
            Ok(())
        })
        .unwrap();

        // a one-way continuation has no independent response or request slot.
        assert_eq!(drv.rx.guarded.lock().unwrap().res.len(), 1);

        drive_once(&mut drv).await;

        assert_busy(request(&tx));
        complete(&drv, 1);
        let _r2 = request(&tx).unwrap();
        assert_busy(request(&tx));
    }

    #[tokio::test]
    async fn unbounded_responses_do_not_inflate_capacity() {
        let (drv, tx) = GenericDriver::new(WriteOkIo, 2);
        let cleanup = || {
            tx.send_unbounded(|buf| {
                buf.extend_from_slice(b"rollback");
                Ok(())
            })
            .unwrap()
        };
        let _cleanup1 = cleanup();
        let _r1 = request(&tx).unwrap();
        let _cleanup2 = cleanup();
        let _r2 = request(&tx).unwrap();
        assert_busy(request(&tx));

        complete(&drv, 1);
        assert_busy(request(&tx));
        complete(&drv, 2);
        let _r3 = request(&tx).unwrap();
        assert_busy(request(&tx));
        complete(&drv, 2);
        assert_eq!(tx.0.sem.available_permits(), 2);
    }

    #[tokio::test]
    async fn partial_response_and_sql_error_retain_the_slot() {
        let (drv, tx) = GenericDriver::new(WriteOkIo, 1);
        let mut res = request(&tx).unwrap();
        let mut buf = backend_message(b'2', b"");
        drv.rx.try_decode(&mut buf).unwrap();
        assert!(matches!(res.recv().await.unwrap(), backend::Message::BindComplete));
        assert_busy(request(&tx));

        buf.extend_from_slice(&backend_message(b'E', b"SERROR\0C42601\0Mbad query\0\0"));
        drv.rx.try_decode(&mut buf).unwrap();
        assert!(res.recv().await.err().unwrap().downcast_ref::<DbError>().is_some());
        assert_busy(request(&tx));

        buf.extend_from_slice(&READY[..4]);
        drv.rx.try_decode(&mut buf).unwrap();
        assert_busy(request(&tx));
        buf.extend_from_slice(&READY[4..]);
        drv.rx.try_decode(&mut buf).unwrap();
        request(&tx).unwrap();
        assert_busy(request(&tx));
    }

    #[tokio::test]
    async fn dropped_receiver_holds_capacity_until_protocol_completion() {
        let (drv, tx) = GenericDriver::new(WriteOkIo, 1);
        drop(request(&tx).unwrap());
        assert_busy(request(&tx));
        complete(&drv, 1);
        request(&tx).unwrap();
    }

    #[tokio::test]
    async fn completed_response_wakes_waiting_send() {
        let (drv, tx) = GenericDriver::new(WriteOkIo, 1);
        let _res = request(&tx).unwrap();
        let mut waiting = core::pin::pin!(tx.send(|buf| {
            buf.extend_from_slice(REQUEST);
            Ok(())
        }));
        let wake = Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct WakeFlag(Arc<std::sync::atomic::AtomicBool>);
        impl std::task::Wake for WakeFlag {
            fn wake(self: Arc<Self>) {
                self.0.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let waker = Waker::from(Arc::new(WakeFlag(wake.clone())));
        assert!(waiting.as_mut().poll(&mut Context::from_waker(&waker)).is_pending());
        complete(&drv, 1);
        assert!(wake.load(std::sync::atomic::Ordering::Relaxed));
        no_hang(waiting).await.unwrap();
        assert_busy(request(&tx));
    }

    #[tokio::test]
    async fn completion_before_async_message_or_decode_error_returns_capacity() {
        for tail in [
            backend_message(b'S', b"key\0value\0"),
            BytesMut::from(&b"Z\0\0\0\x03"[..]),
        ] {
            let (drv, tx) = GenericDriver::new(WriteOkIo, 1);
            let _res = request(&tx).unwrap();
            let mut buf = BytesMut::from(READY);
            buf.extend_from_slice(&tail);
            let result = drv.rx.try_decode(&mut buf);
            if tail[0] == b'S' {
                assert!(matches!(result.unwrap(), Some(backend::Message::ParameterStatus(_))));
            } else {
                assert!(result.is_err());
            }
            assert_eq!(tx.0.sem.available_permits(), 1);
        }
    }

    #[tokio::test]
    async fn copy_continuation_progresses_at_capacity_one() {
        let (mut drv, tx) = GenericDriver::new(WriteOkIo, 1);
        let _res = request(&tx).unwrap();
        let mut buf = backend_message(b'G', b"\0\0\0");
        drv.rx.try_decode(&mut buf).unwrap();
        assert_busy(request(&tx));
        tx.send_one_way_unbounded(|buf| {
            frontend::copy_done(buf);
            frontend::sync(buf);
            Ok(())
        })
        .unwrap();
        drive_once(&mut drv).await;
        assert_busy(request(&tx));
        complete(&drv, 1);
        request(&tx).unwrap();
    }

    #[tokio::test]
    async fn encode_error_returns_the_permit() {
        let (_drv, tx) = GenericDriver::new(WriteOkIo, 1);

        tx.try_send(|_: &mut BytesMut| Err::<(), _>(Error::todo()))
            .err()
            .unwrap();

        // a request that never made it into the buffer must not hold a slot.
        request(&tx).unwrap();
    }

    #[tokio::test]
    async fn large_limit_is_honored() {
        let (_drv, tx) = GenericDriver::new(WriteOkIo, 4096);

        for _ in 0..4096 {
            request(&tx).unwrap();
        }

        assert_busy(request(&tx));
    }

    // nothing is encoded until the future is polled. a caller that goes away first sends nothing.
    #[tokio::test]
    async fn dropped_send_never_encodes() {
        let io = CountWriteIo::new();
        let (mut drv, tx) = GenericDriver::new(io.clone(), 1024);

        let fut = tx.send(|buf| {
            buf.extend_from_slice(REQUEST);
            Ok(())
        });
        drop(fut);

        assert!(drv.rx.guarded.lock().unwrap().res.is_empty());

        drive_once(&mut drv).await;

        assert_eq!(io.writes(), 0);
    }

    // a caller waiting for a queue slot has not encoded anything yet so cancelling it leaves the
    // connection untouched. this is what the sync try_send path can not offer.
    #[tokio::test]
    async fn send_cancelled_while_waiting_never_encodes() {
        let io = CountWriteIo::new();
        let (drv, tx) = GenericDriver::new(io.clone(), 1);

        let _r1 = tx
            .send(|buf| {
                buf.extend_from_slice(REQUEST);
                Ok(())
            })
            .await
            .unwrap();

        let waiting = tx.send(|buf| {
            buf.extend_from_slice(REQUEST);
            Ok(())
        });

        // no slot left so it parks instead of failing.
        let dur = std::time::Duration::from_millis(100);
        assert!(tokio::time::timeout(dur, waiting).await.is_err());

        // and it gave up without queuing anything.
        assert_eq!(drv.rx.guarded.lock().unwrap().res.len(), 1);
    }

    // request encoded before the driver's next write share one write syscall. polling them
    // together is what puts them in the same batch now that encoding happens on first poll.
    #[tokio::test]
    async fn concurrent_send_is_pipelined() {
        let io = CountWriteIo::new();
        let (mut drv, tx) = GenericDriver::new(io.clone(), 1024);

        let send = || {
            tx.send(|buf| {
                buf.extend_from_slice(REQUEST);
                Ok(())
            })
        };

        let (r1, r2, r3) = tokio::join!(send(), send(), send());
        r1.unwrap();
        r2.unwrap();
        r3.unwrap();

        assert_eq!(io.writes(), 0, "nothing is written until the driver runs");
        assert_eq!(drv.rx.guarded.lock().unwrap().res.len(), 3);

        drive_once(&mut drv).await;

        assert_eq!(io.writes(), 1, "three request must share a single write");
    }

    // awaiting one at a time can not batch: each request is encoded only when its future is polled.
    #[tokio::test]
    async fn sequential_send_is_not_batched() {
        let io = CountWriteIo::new();
        let (mut drv, tx) = GenericDriver::new(io.clone(), 1024);

        let _r1 = tx
            .send(|buf| {
                buf.extend_from_slice(REQUEST);
                Ok(())
            })
            .await
            .unwrap();

        drive_once(&mut drv).await;
        assert_eq!(io.writes(), 1);

        let _r2 = tx
            .send(|buf| {
                buf.extend_from_slice(REQUEST);
                Ok(())
            })
            .await
            .unwrap();

        drive_once(&mut drv).await;
        assert_eq!(io.writes(), 2);
    }

    #[tokio::test]
    async fn remote_close_while_idle_is_orderly() {
        let (mut drv, tx) = GenericDriver::new(EofIo, 1024);

        assert!(no_hang(drv.try_next()).await.unwrap().is_none());
        assert_driver_down(request(&tx).map(|_| unreachable!()));

        // Closure is visible even when the terminated driver value is retained.
        assert!(tx.is_closed());
        drop(drv);
        assert!(tx.is_closed());
    }

    #[tokio::test]
    async fn remote_close_releases_pending_request() {
        let (mut drv, tx) = GenericDriver::new(EofIo, 1024);
        let mut res = request(&tx).unwrap();

        // remote closed cleanly. a request can be queued concurrently with the close so a
        // pending one is not evidence of truncation.
        assert!(no_hang(drv.try_next()).await.unwrap().is_none());

        // the caller is released instead of waiting on a response that can never arrive.
        assert_driver_down(no_hang(res.recv()).await);
        assert_driver_down(request(&tx).map(|_| unreachable!()));
    }

    // read close happens in the same readiness as a pending write. the write half is closed by
    // then so try_write must not be entered.
    #[tokio::test]
    async fn remote_close_with_queued_write() {
        let (mut drv, tx) = GenericDriver::new(QueuedWriteEofIo { readable: false }, 1024);
        let mut res = request(&tx).unwrap();

        assert!(no_hang(drv.try_next()).await.unwrap().is_none());

        assert_driver_down(no_hang(res.recv()).await);
        assert_driver_down(request(&tx).map(|_| unreachable!()));
    }

    // remote closed in the middle of a backend message.
    #[tokio::test]
    async fn truncated_message_reports_unexpected_eof() {
        let (mut drv, tx) = GenericDriver::new(PartialThenEofIo { sent: false }, 1024);
        let mut res = request(&tx).unwrap();

        assert_io_err(no_hang(drv.try_next()).await, io::ErrorKind::UnexpectedEof);

        assert_driver_down(no_hang(res.recv()).await);
    }

    // a server error arriving with no request to route it to is reported as a driver error.
    // it does not close the driver: whether to stop or keep polling is the caller decision.
    //
    // note this classifies a server initiated shutdown as a driver failure. routing it to the
    // async message variant instead would be more correct but callers depend on the Err so it
    // waits for a breaking release.
    #[tokio::test]
    async fn unrouted_error_is_reported_as_driver_error() {
        let (mut drv, tx) = GenericDriver::new(FatalThenEofIo { sent: false }, 1024);

        let e = match no_hang(drv.try_next()).await {
            Ok(_) => panic!("expected a db error"),
            Err(e) => e,
        };
        let db = e.downcast_ref::<DbError>().expect("not a db error");
        assert_eq!(db.parsed_severity(), Some(Severity::Fatal));
        assert_eq!(db.code(), &SqlState::ADMIN_SHUTDOWN);

        // a caller that keeps polling reaches the remote close and shuts down from there.
        assert!(no_hang(drv.try_next()).await.unwrap().is_none());

        drop(drv);
        assert_driver_down(request(&tx).map(|_| unreachable!()));
    }

    #[tokio::test]
    async fn write_error_releases_pending_request() {
        let (mut drv, tx) = GenericDriver::new(WriteErrIo, 1024);
        let mut res = request(&tx).unwrap();

        assert_io_err(no_hang(drv.try_next()).await, io::ErrorKind::ConnectionReset);

        assert_driver_down(no_hang(res.recv()).await);
        assert_driver_down(request(&tx).map(|_| unreachable!()));
    }

    #[tokio::test]
    async fn driver_drop_releases_pending_request() {
        let (drv, tx) = GenericDriver::new(EofIo, 1024);
        let mut res = request(&tx).unwrap();

        // shared state outlives the driver so pending requests must be dropped explicitly.
        drop(drv);

        assert_driver_down(no_hang(res.recv()).await);
        assert!(tx.is_closed());
    }

    // the waiting send owns no permit and has not encoded anything. closing the response
    // channels alone cannot release it: the shared semaphore must be closed as well.
    #[tokio::test]
    async fn full_queue_waiter_released_on_driver_drop() {
        let (drv, tx) = GenericDriver::new(WriteOkIo, 1);
        let mut res = request(&tx).unwrap();
        let mut waiting = core::pin::pin!(tx.send::<_, ()>(|_| panic!("closed driver must not encode")));
        assert!(poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx))).await.is_pending());

        drop(drv);

        assert_driver_down(no_hang(res.recv()).await);
        assert_driver_down(no_hang(waiting).await.map(|_| unreachable!()));
        assert_driver_down(request(&tx).map(|_| unreachable!()));
    }

    #[tokio::test]
    async fn full_queue_waiter_released_on_eof() {
        let (mut drv, tx) = GenericDriver::new(EofIo, 1);
        let _res = request(&tx).unwrap();
        let mut waiting = core::pin::pin!(tx.send::<_, ()>(|_| panic!("closed driver must not encode")));
        assert!(poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx))).await.is_pending());

        assert!(no_hang(drv.try_next()).await.unwrap().is_none());

        assert_driver_down(no_hang(waiting).await.map(|_| unreachable!()));
        assert_driver_down(request(&tx).map(|_| unreachable!()));
    }

    #[tokio::test]
    async fn full_queue_waiter_released_on_write_error() {
        let (mut drv, tx) = GenericDriver::new(WriteErrIo, 1);
        let _res = request(&tx).unwrap();
        let mut waiting = core::pin::pin!(tx.send::<_, ()>(|_| panic!("closed driver must not encode")));
        assert!(poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx))).await.is_pending());

        assert_io_err(no_hang(drv.try_next()).await, io::ErrorKind::ConnectionReset);

        assert_driver_down(no_hang(waiting).await.map(|_| unreachable!()));
        assert_driver_down(request(&tx).map(|_| unreachable!()));
    }
}
