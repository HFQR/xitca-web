use core::{
    convert::Infallible,
    future::{pending, poll_fn},
    marker::PhantomData,
    pin::{pin, Pin},
};

use std::{
    io,
    net::{Shutdown, SocketAddr},
};

use futures_core::stream::Stream;
use tokio_uring::net::TcpStream;
use tracing::trace;
use xitca_io::bytes::BytesMut;
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{
    body::NoneBody,
    bytes::Bytes,
    config::HttpServiceConfig,
    date::DateTime,
    h1::{body::RequestBody, error::Error},
    http::{response::Response, StatusCode},
    util::{
        buffered::ReadBuf,
        timer::{KeepAlive, Timeout},
    },
};

use super::{
    dispatcher::{status_only, BodyReader, Timer},
    proto::{codec::TransferCoding, context::Context, error::ProtoError},
};

type ExtRequest<B> = crate::http::Request<crate::http::RequestExt<B>>;

/// function to generic over different writer buffer types dispatcher.
pub(crate) async fn run<
    'a,
    S,
    ReqB,
    ResB,
    BE,
    D,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
>(
    io: &'a mut TcpStream,
    addr: SocketAddr,
    timer: Pin<&'a mut KeepAlive>,
    config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    service: &'a S,
    date: &'a D,
) -> Result<(), Error<S::Error, BE>>
where
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    D: DateTime,
{
    Dispatcher::new(io, addr, timer, config, service, date).run().await
}

/// Http/1 dispatcher
struct Dispatcher<'a, S, ReqB, D, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize> {
    io: &'a TcpStream,
    timer: Timer<'a>,
    ctx: Context<'a, D, HEADER_LIMIT>,
    service: &'a S,
    read_buf: ReadBuf<READ_BUF_LIMIT>,
    in_flight_read_buf: ReadBuf2,
    write_buf: WriteBuf,
    _phantom: PhantomData<ReqB>,
}

struct WriteBuf {
    buf: Option<BytesMut>,
}

struct ReadBuf2 {
    buf: Option<Vec<u8>>,
    len: usize,
}

impl ReadBuf2 {
    fn new() -> Self {
        Self {
            buf: Some(vec![0; 4096]),
            len: 0,
        }
    }

    fn get(&self) -> &[u8] {
        self.buf
            .as_ref()
            .map(|b| &b[..self.len])
            .expect("ReadBuf2::read_io is dropped before polling to complete")
    }

    async fn read_io(&mut self, io: &TcpStream) -> io::Result<()> {
        let (res, buf) = io.read(self.buf.take().unwrap()).await;
        match res {
            Ok(n) => {
                self.len = n;

                if n == 0 {
                    self.buf.replace(buf);
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }

                // TODO: extend in flight buffer size?
                // if n == buf.capacity() {
                //     buf.reserve_exact(n);
                // }

                self.buf.replace(buf);
                Ok(())
            }
            Err(e) => {
                self.buf.replace(buf);
                Err(e)
            }
        }
    }
}

impl WriteBuf {
    fn new() -> Self {
        Self {
            buf: Some(BytesMut::new()),
        }
    }

    fn get_mut(&mut self) -> &mut BytesMut {
        self.buf
            .as_mut()
            .expect("WriteBuf::write_io is dropped before polling to complete")
    }

    async fn write_io(&mut self, io: &TcpStream) -> io::Result<()> {
        let (res, mut buf) = io.write_all(self.buf.take().unwrap()).await;
        buf.clear();
        self.buf.replace(buf);
        res
    }
}

impl<'a, S, ReqB, ResB, BE, D, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize>
    Dispatcher<'a, S, ReqB, D, HEADER_LIMIT, READ_BUF_LIMIT>
where
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    D: DateTime,
{
    fn new<const WRITE_BUF_LIMIT: usize>(
        io: &'a mut TcpStream,
        addr: SocketAddr,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
        service: &'a S,
        date: &'a D,
    ) -> Self {
        Self {
            io,
            timer: Timer::new(timer, config.keep_alive_timeout, config.request_head_timeout),
            ctx: Context::with_addr(addr, date),
            service,
            read_buf: ReadBuf::new(),
            in_flight_read_buf: ReadBuf2::new(),
            write_buf: WriteBuf::new(),
            _phantom: PhantomData,
        }
    }

    async fn run(mut self) -> Result<(), Error<S::Error, BE>> {
        loop {
            match self._run().await {
                Ok(_) => {}
                Err(Error::KeepAliveExpire) => {
                    trace!(target: "h1_dispatcher", "Connection keep-alive expired. Shutting down");
                    return Ok(());
                }
                Err(Error::RequestTimeout) => self.request_error(|| status_only(StatusCode::REQUEST_TIMEOUT)),
                Err(Error::Proto(ProtoError::HeaderTooLarge)) => {
                    self.request_error(|| status_only(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE))
                }
                Err(Error::Proto(_)) => self.request_error(|| status_only(StatusCode::BAD_REQUEST)),
                Err(e) => return Err(e),
            }

            if !self.write_buf.get_mut().is_empty() {
                self.write_buf.write_io(self.io).await?;
            }

            if self.ctx.is_connection_closed() {
                return self.io.shutdown(Shutdown::Both).map_err(Into::into);
            }
        }
    }

    async fn _run(&mut self) -> Result<(), Error<S::Error, BE>> {
        self.timer.update_timer_with(|dur| self.ctx.date().now() + dur);
        self.in_flight_read_buf
            .read_io(self.io)
            .timeout(self.timer.get())
            .await
            .map_err(|_| self.timer.map_to_err())??;

        self.read_buf.get_mut().extend_from_slice(self.in_flight_read_buf.get());

        while let Some((req, decoder)) = self.ctx.decode_head::<READ_BUF_LIMIT>(&mut self.read_buf)? {
            self.timer.reset_state();

            let (mut body_reader, body) = BodyReader::from_coding(decoder);
            let req = req.map(|ext| ext.map_body(|_| ReqB::from(body)));

            {
                let is_expect = self.ctx.is_expect_header();
                let read_task = read_body(
                    is_expect,
                    &mut body_reader,
                    &mut self.read_buf,
                    &mut self.in_flight_read_buf,
                    self.io,
                );

                let mut read_task = pin!(read_task);

                let (parts, body) = match self.service.call(req).select(read_task.as_mut()).await {
                    SelectOutput::A(Ok(res)) => res.into_parts(),
                    SelectOutput::A(Err(e)) => return Err(Error::Service(e)),
                    SelectOutput::B(Err(e)) => return Err(e.into()),
                    SelectOutput::B(Ok(i)) => match i {},
                };

                let encoder = self.ctx.encode_head(parts, &body, self.write_buf.get_mut())?;

                match write_body::<S::Error, BE>(self.io, body, encoder, &mut self.write_buf)
                    .select(read_task)
                    .await
                {
                    SelectOutput::A(Ok(_)) => {}
                    SelectOutput::A(Err(e)) => return Err(e),
                    SelectOutput::B(Err(e)) => return Err(e.into()),
                    SelectOutput::B(Ok(i)) => match i {},
                }
            }

            if !body_reader.decoder.is_eof() {
                self.ctx.set_close();
                break;
            }
        }

        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn request_error(&mut self, func: impl FnOnce() -> Response<NoneBody<Bytes>>) {
        self.ctx.set_close();
        let (parts, body) = func().into_parts();
        self.ctx
            .encode_head(parts, &body, self.write_buf.get_mut())
            .expect("request_error must be correct");
    }
}

async fn read_body<const READ_BUF_LIMIT: usize>(
    is_expect: bool,
    body_reader: &mut BodyReader,
    read_buf: &mut ReadBuf<READ_BUF_LIMIT>,
    read_buf2: &mut ReadBuf2,
    io: &TcpStream,
) -> io::Result<Infallible> {
    if is_expect {
        // wait for service future to start polling RequestBody.
        if body_reader.wait_for_poll().await.is_ok() {
            let buf = b"HTTP/1.1 100 Continue\r\n\r\n".to_vec();
            let (res, _) = io.write_all(buf).await;
            res?;
        }
    }

    loop {
        body_reader.ready(read_buf).await;
        read_buf2.read_io(io).await?;
        read_buf.get_mut().extend_from_slice(read_buf2.get());
    }
}

async fn write_body<SE, BE>(
    io: &TcpStream,
    mut body: impl Stream<Item = Result<Bytes, BE>>,
    mut encoder: TransferCoding,
    write_buf: &mut WriteBuf,
) -> Result<(), Error<SE, BE>> {
    let mut body = pin!(body);

    let mut task = pin!(None);
    let mut err = None;

    loop {
        match poll_fn(|cx| body.as_mut().poll_next(cx))
            .select(async {
                // make sure only one in-flight write operation.
                loop {
                    match task.as_mut().as_pin_mut() {
                        Some(task) => return task.await,
                        None => {
                            if write_buf.get_mut().is_empty() {
                                // pending when buffer is empty. wait for body to make progress
                                // with more bytes. (or exit with error)
                                pending::<()>().await;
                            }
                            let buf = write_buf.get_mut().split();
                            task.as_mut().set(Some(io.write_all(buf)));
                        }
                    }
                }
            })
            .await
        {
            SelectOutput::A(None) => {
                encoder.encode_eof(write_buf.get_mut());
                break;
            }
            SelectOutput::A(Some(Ok(bytes))) => {
                encoder.encode(bytes, write_buf.get_mut());
            }
            SelectOutput::A(Some(Err(e))) => {
                err = Some(Error::Body(e));
                break;
            }
            SelectOutput::B((res, _)) => {
                task.as_mut().set(None);
                res?
            }
        }
    }

    if let Some(task) = task.as_pin_mut() {
        let (res, _) = task.await;
        res?;
    }

    err.take().map(Err).unwrap_or_else(|| Ok(()))
}
