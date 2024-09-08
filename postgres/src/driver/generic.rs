use core::{
    future::{poll_fn, Future},
    pin::Pin,
};

use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
};

use postgres_protocol::message::backend;
use tokio::sync::Notify;
use xitca_io::{
    bytes::{Buf, BufRead, BytesMut},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::error::{DriverDown, Error};

use super::codec::{Response, ResponseMessage, ResponseSender, SenderState};

type PagedBytesMut = xitca_unsafe_collection::bytes::PagedBytesMut<4096>;

pub(crate) struct DriverTx(Arc<SharedState>);

impl Drop for DriverTx {
    fn drop(&mut self) {
        self.0.guarded.lock().unwrap().closed = true;
        self.0.notify.notify_one();
    }
}

impl DriverTx {
    pub(crate) fn is_closed(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }

    pub(crate) fn send_with<F>(&self, func: F) -> Result<Response, Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self.send_multi_with(func, 1)
    }

    pub(crate) fn send_multi_with<F>(&self, func: F, msg_count: usize) -> Result<Response, Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        let mut inner = self.0.guarded.lock().unwrap();

        if inner.closed {
            return Err(DriverDown.into());
        }

        let len = inner.buf.len();

        func(&mut inner.buf).inspect_err(|_| inner.buf.truncate(len))?;
        let (tx, rx) = super::codec::request_pair(msg_count);
        inner.res.push_back(tx);
        self.0.notify.notify_one();

        Ok(rx)
    }
}

pub(crate) struct SharedState {
    guarded: Mutex<State>,
    notify: Notify,
}

struct State {
    closed: bool,
    buf: BytesMut,
    res: VecDeque<ResponseSender>,
}

pub struct GenericDriver<Io> {
    pub(crate) io: Io,
    pub(crate) read_buf: PagedBytesMut,
    pub(crate) state: DriverState,
    pub(crate) shared_state: Arc<SharedState>,
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
}

pub(crate) enum DriverState {
    Running,
    Closing(Option<io::Error>),
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
            notify: Notify::new(),
        });

        (
            Self {
                io,
                read_buf: PagedBytesMut::new(),
                state: DriverState::Running,
                shared_state: state.clone(),
                write_state: WriteState::Waiting,
            },
            DriverTx(state),
        )
    }

    fn want_write(&self) -> bool {
        !matches!(self.write_state, WriteState::Waiting)
    }

    pub(crate) async fn send(&mut self, msg: BytesMut) -> Result<(), Error> {
        self.shared_state.guarded.lock().unwrap().buf.extend_from_slice(&msg);
        self.write_state = WriteState::WantWrite;
        loop {
            self.try_write()?;
            if !self.want_write() {
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

            let mut interest = if self.want_write() {
                const { Interest::READABLE.add(Interest::WRITABLE) }
            } else {
                Interest::READABLE
            };

            let ready = match self.state {
                DriverState::Running => 'inner: loop {
                    match self
                        .shared_state
                        .notify
                        .notified()
                        .select(self.io.ready(interest))
                        .await
                    {
                        SelectOutput::A(_) => {
                            self.write_state = WriteState::WantWrite;
                            interest = interest.add(Interest::WRITABLE);
                            continue 'inner;
                        }
                        SelectOutput::B(ready) => break ready?,
                    }
                },
                DriverState::Closing(ref mut e) => {
                    if !interest.is_writable() && self.shared_state.guarded.lock().unwrap().res.is_empty() {
                        // no interest to write to io and all response have been finished so
                        // shutdown io and exit.
                        // if there is a better way to exhaust potential remaining backend message
                        // please file an issue.
                        poll_fn(|cx| Pin::new(&mut self.io).poll_shutdown(cx)).await?;
                        return e.take().map(|e| Err(e.into())).transpose();
                    }
                    self.io.ready(interest).await?
                }
            };

            if ready.is_readable() {
                self.try_read()?;
            }

            if ready.is_writable() {
                if let Err(e) = self.try_write() {
                    // when write error occur the driver would go into half close state(read only).
                    // clearing write_buf would drop all pending requests in it and hint the driver
                    // no future Interest::WRITABLE should be passed to AsyncIo::ready method.
                    let mut inner = self.shared_state.guarded.lock().unwrap();
                    inner.buf.clear();
                    // close shared state early so driver tx can observe the shutdown in first hand
                    inner.closed = true;
                    self.write_state = WriteState::Waiting;
                    self.state = DriverState::Closing(Some(e));
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

    fn try_read(&mut self) -> Result<(), Error> {
        self.read_buf.do_io(&mut self.io).map_err(Into::into)
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

                    if inner.closed {
                        if matches!(self.state, DriverState::Running) {
                            self.state = DriverState::Closing(None);
                        }

                        if inner.buf.is_empty() {
                            self.write_state = WriteState::WantFlush;
                            continue;
                        }
                    }

                    match io::Write::write(&mut self.io, &inner.buf) {
                        Ok(0) => {
                            // spurious write can happen if ClientTx release locked buffer before writer
                            // obtain it's lock. The buffer can be written to IO before Notified get polled.
                            // Notified will cause another write and at that point the buffer could be emptied
                            // already.
                            if inner.buf.is_empty() {
                                break;
                            }

                            return Err(io::ErrorKind::WriteZero.into());
                        }
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
                WriteState::Waiting => unreachable!("try_write must be called when WriteState is waiting"),
            }
        }

        Ok(())
    }

    fn try_decode(&mut self) -> Result<Option<backend::Message>, Error> {
        while let Some(res) = ResponseMessage::try_from_buf(self.read_buf.get_mut())? {
            match res {
                ResponseMessage::Normal { buf, complete } => {
                    let mut inner = self.shared_state.guarded.lock().unwrap();
                    let front = inner.res.front_mut().expect("server respond out of bound");
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
