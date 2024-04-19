use core::{
    convert::Infallible,
    future::{poll_fn, Future},
    pin::Pin,
};

use std::{collections::VecDeque, io};

use postgres_protocol::message::backend;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::{
    bytes::{BufInterest, BufRead, BufWrite, BytesMut, WriteBuf},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::error::Error;

use super::codec::{Request, Response, ResponseMessage, ResponseSender, SenderState};

type PagedBytesMut = xitca_unsafe_collection::bytes::PagedBytesMut<4096>;

pub(crate) type GenericDriverRx = UnboundedReceiver<Request>;

#[derive(Debug)]
pub(crate) struct DriverTx(UnboundedSender<Request>);

impl DriverTx {
    pub(crate) fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    pub(crate) fn send(&self, msg: BytesMut) -> impl Future<Output = Result<Response, Error>> + '_ {
        self.send_multi(1, msg)
    }

    pub(crate) async fn send_multi(&self, msg_count: usize, msg: BytesMut) -> Result<Response, Error> {
        let (tx, rx) = unbounded_channel();
        self.0.send(Request::new(tx, msg_count, msg))?;
        Ok(Response::new(rx))
    }

    pub(crate) fn do_send(&self, msg: BytesMut) {
        let (tx, _) = unbounded_channel();
        let _ = self.0.send(Request::new(tx, 1, msg));
    }
}

pub struct GenericDriver<Io> {
    pub(crate) io: Io,
    pub(crate) write_buf: WriteBuf,
    pub(crate) read_buf: PagedBytesMut,
    pub(crate) state: DriverState,
    pub(crate) res: VecDeque<ResponseSender>,
}

pub(crate) enum DriverState {
    Running(GenericDriverRx),
    Closing(Option<io::Error>),
}

#[cfg(feature = "io-uring")]
impl DriverState {
    pub(crate) fn take_rx(self) -> GenericDriverRx {
        match self {
            Self::Running(rx) => rx,
            _ => panic!("driver is closing. no rx can be handed out"),
        }
    }
}

impl<Io> GenericDriver<Io>
where
    Io: AsyncIo + Send,
{
    pub(crate) fn new(io: Io) -> (Self, DriverTx) {
        let (tx, rx) = unbounded_channel();
        (
            Self {
                io,
                write_buf: WriteBuf::new(),
                read_buf: PagedBytesMut::new(),
                res: VecDeque::new(),
                state: DriverState::Running(rx),
            },
            DriverTx(tx),
        )
    }

    pub(crate) async fn send(&mut self, msg: BytesMut) -> Result<(), Error> {
        self.write_buf_extend(&msg);
        loop {
            self.try_write()?;
            if self.write_buf.is_empty() {
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

            let interest = if self.write_buf.want_write_io() {
                Interest::READABLE.add(Interest::WRITABLE)
            } else {
                Interest::READABLE
            };

            let select = match self.state {
                DriverState::Running(ref mut rx) => rx.recv().select(self.io.ready(interest)).await,
                DriverState::Closing(ref mut e) => {
                    if !interest.is_writable() && self.res.is_empty() {
                        // no interest to write to io and all response have been finished so
                        // shutdown io and exit.
                        // if there is a better way to exhaust potential remaining backend message
                        // please file an issue.
                        poll_fn(|cx| Pin::new(&mut self.io).poll_shutdown(cx)).await?;
                        return e.take().map(|e| Err(e.into())).transpose();
                    }
                    SelectOutput::B(self.io.ready(interest).await)
                }
            };

            match select {
                // batch message and keep polling.
                SelectOutput::A(Some(req)) => {
                    self.write_buf_extend(req.msg.as_ref());
                    self.res.push_back(req.tx);
                }
                SelectOutput::B(ready) => {
                    let ready = ready?;
                    if ready.is_readable() {
                        self.try_read()?;
                    }
                    if ready.is_writable() {
                        if let Err(e) = self.try_write() {
                            // when write error occur the driver would go into half close state(read only).
                            // clearing write_buf would drop all pending requests in it and hint the driver
                            // no future Interest::WRITABLE should be passed to AsyncIo::ready method.
                            self.write_buf.clear();

                            // enter closed state and no more request would be received from channel.
                            // requests inside it would eventually be dropped after shutdown completed.
                            self.state = DriverState::Closing(Some(e));
                        }
                    }
                }
                SelectOutput::A(None) => self.state = DriverState::Closing(None),
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

    fn write_buf_extend(&mut self, buf: &[u8]) {
        let _ = self.write_buf.write_buf(|w| {
            w.extend_from_slice(buf);
            Ok::<_, Infallible>(())
        });
    }

    fn try_read(&mut self) -> Result<(), Error> {
        self.read_buf.do_io(&mut self.io).map_err(Into::into)
    }

    fn try_write(&mut self) -> io::Result<()> {
        self.write_buf.do_io(&mut self.io)
    }

    fn try_decode(&mut self) -> Result<Option<backend::Message>, Error> {
        while let Some(res) = ResponseMessage::try_from_buf(self.read_buf.get_mut())? {
            match res {
                ResponseMessage::Normal { buf, complete } => {
                    let front = self.res.front_mut().expect("server respond out of bound");
                    match front.send(buf, complete) {
                        SenderState::Finish => {
                            self.res.pop_front();
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
