use core::{
    future::{poll_fn, Future},
    pin::Pin,
    task::Poll,
};

use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
};

use futures_core::task::__internal::AtomicWaker;
use postgres_protocol::message::{backend, frontend};
use xitca_io::{
    bytes::{Buf, BufRead, BytesMut},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::error::{DriverDown, Error};

use super::codec::{Response, ResponseMessage, ResponseSender, SenderState};

type PagedBytesMut = xitca_unsafe_collection::bytes::PagedBytesMut<4096>;

const INTEREST_READ_WRITE: Interest = Interest::READABLE.add(Interest::WRITABLE);

pub(crate) struct DriverTx(Arc<SharedState>);

impl Drop for DriverTx {
    fn drop(&mut self) {
        let mut state = self.0.guarded.lock().unwrap();
        frontend::terminate(&mut state.buf);
        state.closed = true;
        self.0.waker.wake();
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
        self._send(func, |_| {})
    }

    pub(crate) fn send<F>(&self, func: F) -> Result<Response, Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self.send_multi(func, 1)
    }

    pub(crate) fn send_multi<F>(&self, func: F, msg_count: usize) -> Result<Response, Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self._send(func, |inner| {
            let (tx, rx) = super::codec::request_pair(msg_count);
            inner.res.push_back(tx);
            rx
        })
    }

    fn _send<F, F2, T>(&self, func: F, on_send: F2) -> Result<T, Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
        F2: FnOnce(&mut State) -> T,
    {
        let mut inner = self.0.guarded.lock().unwrap();

        if inner.closed {
            return Err(DriverDown.into());
        }

        let len = inner.buf.len();

        func(&mut inner.buf).inspect_err(|_| inner.buf.truncate(len))?;

        let res = on_send(&mut inner);

        self.0.waker.wake();

        Ok(res)
    }
}

pub(crate) struct SharedState {
    guarded: Mutex<State>,
    waker: AtomicWaker,
}

impl SharedState {
    async fn wait(&self) -> WaitState {
        poll_fn(|cx| {
            let inner = self.guarded.lock().unwrap();
            if !inner.buf.is_empty() {
                Poll::Ready(WaitState::WantWrite)
            } else if inner.closed {
                Poll::Ready(WaitState::WantClose)
            } else {
                self.waker.register(cx.waker());
                Poll::Pending
            }
        })
        .await
    }
}

enum WaitState {
    WantWrite,
    WantClose,
}

struct State {
    closed: bool,
    buf: BytesMut,
    res: VecDeque<ResponseSender>,
}

pub struct GenericDriver<Io> {
    io: Io,
    read_buf: PagedBytesMut,
    shared_state: Arc<SharedState>,
    read_state: ReadState,
    write_state: WriteState,
}

// in case driver is dropped without closing the shared state
impl<Io> Drop for GenericDriver<Io> {
    fn drop(&mut self) {
        self.shared_state.guarded.lock().unwrap().closed = true;
    }
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
            }),
            waker: AtomicWaker::new(),
        });

        (
            Self {
                io,
                read_buf: PagedBytesMut::new(),
                shared_state: state.clone(),
                read_state: ReadState::WantRead,
                write_state: WriteState::Waiting,
            },
            DriverTx(state),
        )
    }

    pub(crate) async fn send(&mut self, msg: BytesMut) -> Result<(), Error> {
        self.shared_state.guarded.lock().unwrap().buf.extend_from_slice(&msg);
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

    pub(crate) async fn try_next(&mut self) -> Result<Option<backend::Message>, Error> {
        loop {
            if let Some(msg) = self.try_decode()? {
                return Ok(Some(msg));
            }

            let ready = match (&mut self.read_state, &mut self.write_state) {
                (ReadState::WantRead, WriteState::Waiting) => {
                    match self.shared_state.wait().select(self.io.ready(Interest::READABLE)).await {
                        SelectOutput::A(WaitState::WantWrite) => {
                            self.write_state = WriteState::WantWrite;
                            self.io.ready(INTEREST_READ_WRITE).await?
                        }
                        SelectOutput::A(WaitState::WantClose) => {
                            self.write_state = WriteState::Closed(None);
                            continue;
                        }
                        SelectOutput::B(ready) => ready?,
                    }
                }
                (ReadState::WantRead, WriteState::WantWrite) => self.io.ready(INTEREST_READ_WRITE).await?,
                (ReadState::WantRead, WriteState::WantFlush) => {
                    // before flush io do a quick buffer check and go into write io state if possible.
                    if !self.shared_state.guarded.lock().unwrap().buf.is_empty() {
                        self.write_state = WriteState::WantWrite;
                    }
                    self.io.ready(INTEREST_READ_WRITE).await?
                }
                (ReadState::WantRead, WriteState::Closed(_)) => self.io.ready(Interest::READABLE).await?,
                (ReadState::Closed(_), WriteState::WantFlush | WriteState::WantWrite) => {
                    self.io.ready(Interest::WRITABLE).await?
                }
                (ReadState::Closed(_), WriteState::Waiting) => match self.shared_state.wait().await {
                    WaitState::WantWrite => {
                        self.write_state = WriteState::WantWrite;
                        self.io.ready(Interest::WRITABLE).await?
                    }
                    WaitState::WantClose => {
                        self.write_state = WriteState::Closed(None);
                        continue;
                    }
                },
                (ReadState::Closed(None), WriteState::Closed(None)) => {
                    poll_fn(|cx| Pin::new(&mut self.io).poll_shutdown(cx)).await?;
                    return Ok(None);
                }
                (ReadState::Closed(read_err), WriteState::Closed(write_err)) => {
                    return Err(Error::driver_io(read_err.take(), write_err.take()))
                }
            };

            if ready.is_readable() {
                if let Err(e) = self.try_read() {
                    self.on_read_err(e);
                };
            }

            if ready.is_writable() {
                if let Err(e) = self.try_write() {
                    self.on_write_err(e);
                }
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
            self.try_read()?;
        }
    }

    fn try_read(&mut self) -> io::Result<()> {
        self.read_buf.do_io(&mut self.io)
    }

    fn try_write(&mut self) -> io::Result<()> {
        loop {
            match self.write_state {
                WriteState::WantFlush => {
                    match io::Write::flush(&mut self.io) {
                        Ok(_) => self.write_state = WriteState::Waiting,
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) => return Err(e),
                    }
                    break;
                }
                WriteState::WantWrite => {
                    let mut inner = self.shared_state.guarded.lock().unwrap();

                    match io::Write::write(&mut self.io, &inner.buf) {
                        Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                        Ok(n) => {
                            inner.buf.advance(n);

                            if inner.buf.is_empty() {
                                self.write_state = WriteState::WantFlush;
                            }
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) => return Err(e),
                    }
                }
                _ => unreachable!("try_write must be called when WriteState is waiting"),
            }
        }

        Ok(())
    }

    #[cold]
    fn on_read_err(&mut self, e: io::Error) {
        let reason = (e.kind() != io::ErrorKind::UnexpectedEof).then_some(e);
        self.read_state = ReadState::Closed(reason);
    }

    #[cold]
    fn on_write_err(&mut self, e: io::Error) {
        {
            // when write error occur the driver would go into half close state(read only).
            // clearing write_buf would drop all pending requests in it and hint the driver
            // no future Interest::WRITABLE should be passed to AsyncIo::ready method.
            let mut inner = self.shared_state.guarded.lock().unwrap();
            inner.buf.clear();
            // close shared state early so driver tx can observe the shutdown in first hand
            inner.closed = true;
        }
        self.write_state = WriteState::Closed(Some(e));
    }

    fn try_decode(&mut self) -> Result<Option<backend::Message>, Error> {
        while let Some(res) = ResponseMessage::try_from_buf(self.read_buf.get_mut())? {
            match res {
                ResponseMessage::Normal { mut buf, complete } => {
                    let mut inner = self.shared_state.guarded.lock().unwrap();
                    let front = inner.res.front_mut().ok_or_else(|| parse_error(&mut buf))?;
                    match front.send(buf, complete) {
                        SenderState::Finish => {
                            inner.res.pop_front();
                        }
                        SenderState::Continue => {}
                    }
                }
                ResponseMessage::Async(msg) => return Ok(Some(msg)),
            }
        }
        Ok(None)
    }
}

#[cold]
#[inline(never)]
fn parse_error(buf: &mut BytesMut) -> Error {
    match backend::Message::parse(buf) {
        Err(e) => Error::from(e),
        Ok(Some(backend::Message::ErrorResponse(body))) => Error::db(body.fields()),
        _ => Error::unexpected(),
    }
}
