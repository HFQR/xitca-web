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
use xitca_io::bytes::{Buf, BytesMut};
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{
    body::NoneBody,
    bytes::Bytes,
    config::HttpServiceConfig,
    date::DateTime,
    h1::{
        body::{RequestBody, RequestBodySender},
        error::Error,
    },
    http::{
        response::{Parts, Response},
        StatusCode,
    },
    util::{
        buffered::ReadBuf,
        timer::{KeepAlive, Timeout},
    },
};

use super::{
    dispatcher::Timer,
    proto::{
        codec::{ChunkResult, TransferCoding},
        context::Context,
        error::ProtoError,
    },
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
        let (res, mut buf) = io.read(self.buf.take().unwrap()).await;
        match res {
            Ok(n) => {
                self.len = n;

                if n == 0 {
                    self.buf.replace(buf);
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }

                if n == buf.capacity() {
                    buf.reserve_exact(n);
                }

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

            self.write_buf.write_io(self.io).await?;

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

            let (parts, body) = match self
                .service
                .call(req)
                .select(self.request_body_handler(&mut body_reader))
                .await
            {
                SelectOutput::A(Ok(res)) => res.into_parts(),
                SelectOutput::A(Err(e)) => return Err(Error::Service(e)),
                SelectOutput::B(Err(e)) => return Err(e),
                SelectOutput::B(Ok(i)) => match i {},
            };

            let encoder = self.encode_head(parts, &body)?;
            match write_body::<S::Error, BE>(self.io, body, encoder, &mut self.write_buf)
                .select(read_body(
                    &mut body_reader,
                    &mut self.read_buf,
                    &mut self.in_flight_read_buf,
                    self.io,
                    &mut self.ctx,
                ))
                .await
            {
                SelectOutput::A(Ok(_)) => {}
                SelectOutput::A(Err(e)) => return Err(e),
                SelectOutput::B(Err(e)) => return Err(e.into()),
                SelectOutput::B(Ok(i)) => match i {},
            }

            if !body_reader.decoder.is_eof() {
                self.ctx.set_close();
                break;
            }
        }

        Ok(())
    }

    fn encode_head(&mut self, parts: Parts, body: &impl Stream) -> Result<TransferCoding, ProtoError> {
        self.ctx.encode_head(parts, body, self.write_buf.get_mut())
    }

    // an associated future of self.service that runs until service is resolved or error produced.
    async fn request_body_handler(&mut self, body_reader: &mut BodyReader) -> Result<Infallible, Error<S::Error, BE>> {
        if self.ctx.is_expect_header() {
            // wait for service future to start polling RequestBody.
            if body_reader.wait_for_poll().await.is_ok() {
                // encode continue as service future want a body.
                self.ctx.encode_continue(self.write_buf.get_mut());
                // use drain write to make sure continue is sent to client.
                self.write_buf.write_io(self.io).await?;
            }
        }

        read_body(
            body_reader,
            &mut self.read_buf,
            &mut self.in_flight_read_buf,
            self.io,
            &mut self.ctx,
        )
        .await
        .map_err(Into::into)
    }

    #[cold]
    #[inline(never)]
    fn request_error(&mut self, func: impl FnOnce() -> Response<NoneBody<Bytes>>) {
        self.ctx.set_close();
        let (parts, body) = func().into_parts();
        self.encode_head(parts, &body).expect("request_error must be correct");
    }
}

async fn read_body<D, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize>(
    body_reader: &mut BodyReader,
    read_buf: &mut ReadBuf<READ_BUF_LIMIT>,
    read_buf2: &mut ReadBuf2,
    io: &TcpStream,
    ctx: &mut Context<'_, D, HEADER_LIMIT>,
) -> io::Result<Infallible> {
    loop {
        body_reader.ready(read_buf, ctx).await;
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

    loop {
        if write_buf.get_mut().remaining() > 65535 {
            write_buf.write_io(io).await?;
        }

        match poll_fn(|cx| body.as_mut().poll_next(cx)).await {
            Some(chunk) => {
                let bytes = chunk.map_err(Error::Body)?;
                encoder.encode(bytes, write_buf.get_mut());
            }
            None => {
                encoder.encode_eof(write_buf.get_mut());
                break;
            }
        };
    }
    Ok(())
}

struct BodyReader {
    decoder: TransferCoding,
    tx: RequestBodySender,
}

impl BodyReader {
    fn from_coding(decoder: TransferCoding) -> (Self, RequestBody) {
        let (tx, body) = RequestBody::channel(decoder.is_eof());
        let body_reader = BodyReader { decoder, tx };
        (body_reader, body)
    }

    // dispatcher MUST call this method before do any io reading.
    // a none ready state means the body consumer either is in backpressure or don't expect body.
    async fn ready<D, const READ_BUF_LIMIT: usize, const HEADER_LIMIT: usize>(
        &mut self,
        read_buf: &mut ReadBuf<READ_BUF_LIMIT>,
        ctx: &mut Context<'_, D, HEADER_LIMIT>,
    ) {
        loop {
            match self.decoder.decode(&mut *read_buf) {
                ChunkResult::Ok(bytes) => self.tx.feed_data(bytes),
                ChunkResult::InsufficientData => match self.tx.ready().await {
                    Ok(_) => return,
                    // service future drop RequestBody half way so notify Context to close
                    // connection afterwards.
                    Err(_) => self.set_close(ctx),
                },
                ChunkResult::Eof => self.tx.feed_eof(),
                ChunkResult::AlreadyEof => pending().await,
                ChunkResult::Err(e) => self.feed_error(e, ctx),
            }
        }
    }

    // feed error to body sender and prepare for close connection.
    #[cold]
    #[inline(never)]
    fn feed_error<D, const HEADER_LIMIT: usize>(&mut self, e: io::Error, ctx: &mut Context<'_, D, HEADER_LIMIT>) {
        self.tx.feed_error(e);
        self.set_close(ctx);
    }

    // prepare for close connection by end decoder and set context to closed connection regardless their current states.
    #[cold]
    #[inline(never)]
    fn set_close<D, const HEADER_LIMIT: usize>(&mut self, ctx: &mut Context<'_, D, HEADER_LIMIT>) {
        self.decoder.set_eof();
        ctx.set_close();
    }

    // wait for service start to consume RequestBody.
    async fn wait_for_poll(&mut self) -> io::Result<()> {
        self.tx.wait_for_poll().await.map_err(|e| {
            // IMPORTANT: service future drop RequestBody so set decoder to eof.
            self.decoder.set_eof();
            e
        })
    }
}

#[cold]
#[inline(never)]
fn status_only(status: StatusCode) -> Response<NoneBody<Bytes>> {
    Response::builder().status(status).body(NoneBody::default()).unwrap()
}
