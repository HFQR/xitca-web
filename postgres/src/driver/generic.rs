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

use postgres_protocol::message::{backend, frontend};
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{
    error::{DriverDown, Error},
    iter::AsyncLendingIterator,
};

use super::codec::{Response, ResponseMessage, ResponseSender};

type PagedBytesMut = xitca_unsafe_collection::bytes::PagedBytesMut<4096>;

const INTEREST_READ_WRITE: Interest = Interest::READABLE.add(Interest::WRITABLE);

pub(crate) struct DriverTx(Arc<SharedState>);

impl Drop for DriverTx {
    fn drop(&mut self) {
        let mut state = self.0.guarded.lock().unwrap();
        frontend::terminate(&mut state.buf);
        state.closed = true;
        state.wake();
    }
}

impl DriverTx {
    pub(crate) fn is_closed(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }

    pub(crate) fn send_one_way<F>(&self, func: F) -> Result<(), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self._send(func, |_| {})?;
        Ok(())
    }

    pub(crate) fn send<F, O>(&self, func: F) -> Result<(O, Response), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<O, Error>,
    {
        self._send(func, |inner| {
            let (tx, rx) = super::codec::request_pair();
            inner.res.push_back(tx);
            rx
        })
    }

    fn _send<F, F2, O, T>(&self, func: F, on_send: F2) -> Result<(O, T), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<O, Error>,
        F2: FnOnce(&mut State) -> T,
    {
        let mut inner = self.0.guarded.lock().unwrap();

        if inner.closed {
            return Err(DriverDown.into());
        }

        let len = inner.buf.len();

        let o = func(&mut inner.buf).inspect_err(|_| inner.buf.truncate(len))?;
        let t = on_send(&mut inner);

        inner.wake();

        Ok((o, t))
    }
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
        self.guarded.lock().unwrap().closed = true;
    }
}

pub(crate) struct SharedState {
    pub(super) guarded: Mutex<State>,
}

impl SharedState {
    pub(super) fn wait(&self) -> impl Future<Output = WaitState> + use<'_> {
        poll_fn(|cx| {
            let mut inner = self.guarded.lock().unwrap();
            if !inner.buf.is_empty() {
                Poll::Ready(WaitState::WantWrite)
            } else if inner.closed {
                Poll::Ready(WaitState::WantClose)
            } else {
                inner.register(cx.waker());
                Poll::Pending
            }
        })
    }
}

pub(super) enum WaitState {
    WantWrite,
    WantClose,
}

pub(super) struct State {
    pub(super) closed: bool,
    pub(super) buf: BytesMut,
    pub(super) res: VecDeque<ResponseSender>,
    pub(super) waker: Option<Waker>,
}

impl State {
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

enum WriteState {
    Waiting,
    WantWrite,
    WantFlush,
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
    pub(crate) fn new(io: Io) -> (Self, DriverTx) {
        let state = Arc::new(SharedState {
            guarded: Mutex::new(State {
                closed: false,
                buf: BytesMut::new(),
                res: VecDeque::new(),
                waker: None,
            }),
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

    pub(crate) fn recv(&mut self) -> impl Future<Output = Result<backend::Message, Error>> + Send + '_ {
        self.recv_with(|buf| backend::Message::parse(buf).map_err(Error::from).transpose())
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
                (ReadState::WantRead, WriteState::Closed(_)) => {
                    SelectOutput::A(self.io.ready(Interest::READABLE).await)
                }
                (ReadState::Closed(_), WriteState::WantFlush | WriteState::WantWrite) => {
                    SelectOutput::A(self.io.ready(Interest::WRITABLE).await)
                }
                (ReadState::Closed(_), WriteState::Waiting) => SelectOutput::B(self.rx.wait().await),
                (ReadState::Closed(None), WriteState::Closed(None)) => {
                    poll_fn(|cx| Pin::new(&mut self.io).poll_shutdown(cx)).await?;
                    return Ok(None);
                }
                (ReadState::Closed(read_err), WriteState::Closed(write_err)) => {
                    return Err(Error::driver_io(read_err.take(), write_err.take()));
                }
            };

            match res {
                SelectOutput::A(res) => {
                    let ready = res?;
                    if ready.is_readable() {
                        match self.try_read() {
                            Ok(Some(0)) => self.on_read_close(None),
                            Ok(_) => {}
                            Err(e) => self.on_read_close(Some(e)),
                        }
                    }

                    if ready.is_writable() {
                        if let Err(e) = self.try_write() {
                            self.on_write_err(e);
                        }
                    }
                }
                SelectOutput::B(WaitState::WantWrite) => self.write_state = WriteState::WantWrite,
                SelectOutput::B(WaitState::WantClose) => self.write_state = WriteState::Closed(None),
            }
        }
    }

    async fn recv_with<F, O>(&mut self, mut func: F) -> Result<O, Error>
    where
        F: FnMut(&mut BytesMut) -> Option<Result<O, Error>>,
    {
        loop {
            if let Some(o) = func(self.read_buf.get_mut()) {
                return o;
            }
            self.io.ready(Interest::READABLE).await?;
            if let Some(0) = self.try_read()? {
                return Err(Error::from(io::Error::from(io::ErrorKind::UnexpectedEof)));
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
            "try_write must not be called when WriteState is Wait or Closed"
        );

        if matches!(self.write_state, WriteState::WantWrite) {
            let mut inner = self.rx.guarded.lock().unwrap();

            let mut written = 0;

            loop {
                match io::Write::write(&mut self.io, &inner.buf[written..]) {
                    Ok(0) => {
                        inner.buf.advance(written);
                        return Err(io::ErrorKind::WriteZero.into());
                    }
                    Ok(n) => {
                        written += n;
                        if written == inner.buf.len() {
                            inner.buf.clear();
                            self.write_state = WriteState::WantFlush;
                            break;
                        }
                    }
                    Err(e) => {
                        inner.buf.advance(written);
                        return if matches!(e.kind(), io::ErrorKind::WouldBlock) {
                            Ok(())
                        } else {
                            Err(e)
                        };
                    }
                }
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
        self.read_state = ReadState::Closed(reason);
    }

    #[cold]
    #[inline(never)]
    fn on_write_err(&mut self, e: io::Error) {
        {
            // when write error occur the driver would go into half close state(read only).
            // clearing write_buf would drop all pending requests in it and hint the driver
            // no future Interest::WRITABLE should be passed to AsyncIo::ready method.
            let mut inner = self.rx.guarded.lock().unwrap();
            inner.buf.clear();
            // close shared state early so driver tx can observe the shutdown in first hand
            inner.closed = true;
        }
        self.write_state = WriteState::Closed(Some(e));
    }
}

impl DriverRx {
    pub(super) fn try_decode(&self, read_buf: &mut BytesMut) -> Result<Option<backend::Message>, Error> {
        let mut guard = None;

        while let Some(res) = ResponseMessage::try_from_buf(read_buf)? {
            match res {
                ResponseMessage::Normal(mut msg) => {
                    // lock the shared state only when needed and keep the lock around a bit for possible multiple messages
                    let inner = guard.get_or_insert_with(|| self.guarded.lock().unwrap());
                    let complete = msg.complete();
                    let _ = inner.res.front_mut().ok_or_else(|| msg.parse_error())?.send(msg);
                    if complete {
                        inner.res.pop_front();
                    }
                }
                ResponseMessage::Async(msg) => return Ok(Some(msg)),
            }
        }

        Ok(None)
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
