use core::{
    future::{Future, poll_fn},
    pin::Pin,
};

use std::{collections::VecDeque, io};

use postgres_protocol::message::{backend, frontend};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{
    error::{DriverDown, Error},
    iter::AsyncLendingIterator,
};

use super::codec::{Request, Response, ResponseMessage, ResponseSender};

type PagedBytesMut = xitca_unsafe_collection::bytes::PagedBytesMut<4096>;

const INTEREST_READ_WRITE: Interest = Interest::READABLE.add(Interest::WRITABLE);

pub(crate) struct DriverTx(UnboundedSender<Request>);

impl Drop for DriverTx {
    fn drop(&mut self) {
        let mut buf = BytesMut::new();
        frontend::terminate(&mut buf);
        let (tx, _) = super::codec::request_pair(buf);
        let _ = self.0.send(tx);
    }
}

impl DriverTx {
    pub(crate) fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    pub(crate) fn send_one_way<F>(&self, func: F) -> Result<(), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        todo!()
    }

    pub(crate) fn send(&self, buf: BytesMut) -> Result<Response, Error> {
        let (req, res) = super::codec::request_pair(buf);
        self.0.send(req).map_err(|_| DriverDown)?;
        Ok(res)
    }
}

pub struct GenericDriver<Io> {
    pub(super) io: Io,
    pub(super) req: VecDeque<ResponseSender>,
    pub(super) read_buf: PagedBytesMut,
    pub(super) rx: UnboundedReceiver<Request>,
    read_state: ReadState,
    write_state: WriteState,
}

enum WriteState {
    Waiting,
    WantWrite(BytesMut),
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
        let (tx, rx) = unbounded_channel();

        (
            Self {
                io,
                rx,
                req: VecDeque::new(),
                read_buf: PagedBytesMut::new(),
                read_state: ReadState::WantRead,
                write_state: WriteState::Waiting,
            },
            DriverTx(tx),
        )
    }

    pub(crate) async fn send(&mut self, msg: BytesMut) -> Result<(), Error> {
        self.write_state = WriteState::WantWrite(msg);
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
            if let Some(msg) = try_decode(&mut self.req, &mut self.read_buf.get_mut())? {
                return Ok(Some(msg));
            }

            let res = match (&mut self.read_state, &mut self.write_state) {
                (ReadState::WantRead, WriteState::Waiting) => {
                    self.io.ready(Interest::READABLE).select(self.rx.recv()).await
                }
                (ReadState::WantRead, WriteState::WantWrite(_) | WriteState::WantFlush) => {
                    SelectOutput::A(self.io.ready(INTEREST_READ_WRITE).await)
                }
                (ReadState::WantRead, WriteState::Closed(_)) => {
                    SelectOutput::A(self.io.ready(Interest::READABLE).await)
                }
                (ReadState::Closed(_), WriteState::WantFlush | WriteState::WantWrite(_)) => {
                    SelectOutput::A(self.io.ready(Interest::WRITABLE).await)
                }
                (ReadState::Closed(_), WriteState::Waiting) => SelectOutput::B(self.rx.recv().await),
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
                SelectOutput::B(Some(msg)) => {
                    self.write_state = WriteState::WantWrite(msg.buf);
                    self.req.push_back(msg.tx);
                }
                SelectOutput::B(None) => self.write_state = WriteState::Closed(None),
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
            matches!(self.write_state, WriteState::WantWrite(_) | WriteState::WantFlush),
            "try_write must not be called when WriteState is Wait or Closed"
        );

        if let WriteState::WantWrite(ref mut buf) = self.write_state {
            let mut written = 0;
            loop {
                match io::Write::write(&mut self.io, &buf[written..]) {
                    Ok(0) => {
                        buf.advance(written);
                        return Err(io::ErrorKind::WriteZero.into());
                    }
                    Ok(n) => {
                        written += n;
                        if written == buf.len() {
                            buf.clear();
                            self.write_state = WriteState::WantFlush;
                            break;
                        }
                    }
                    Err(e) => {
                        buf.advance(written);
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
        // when write error occur the driver would go into half close state(read only).
        // clearing all pending requests in channel to notify task waiting for response
        self.rx.close();
        self.write_state = WriteState::Closed(Some(e));
    }
}

pub(super) fn try_decode(
    queue: &mut VecDeque<ResponseSender>,
    read_buf: &mut BytesMut,
) -> Result<Option<backend::Message>, Error> {
    while let Some(res) = ResponseMessage::try_from_buf(read_buf)? {
        match res {
            ResponseMessage::Normal(mut msg) => {
                let req = queue.pop_front().ok_or_else(|| msg.parse_error())?;
                let _ = req.send(msg.buf);
                if !msg.complete {
                    queue.push_front(req);
                }
            }
            ResponseMessage::Async(msg) => return Ok(Some(msg)),
        }
    }

    Ok(None)
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
