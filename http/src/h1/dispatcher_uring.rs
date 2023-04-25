use core::{
    convert::Infallible,
    future::{pending, poll_fn, Future},
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

/// Http/1 dispatcher
pub(super) struct Dispatcher<'a, S, ReqB, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize> {
    io: &'a TcpStream,
    timer: Timer<'a>,
    ctx: Context<'a, D, H_LIMIT>,
    service: &'a S,
    read_buf: ReadBuf<R_LIMIT>,
    in_flight_read_buf: ReadBuf2,
    write_buf: WriteBuf<W_LIMIT>,
    _phantom: PhantomData<ReqB>,
}

struct WriteBuf<const LIMIT: usize> {
    buf: Option<BytesMut>,
}

impl<const LIMIT: usize> WriteBuf<LIMIT> {
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

impl<'a, S, ReqB, ResB, BE, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize>
    Dispatcher<'a, S, ReqB, D, H_LIMIT, R_LIMIT, W_LIMIT>
where
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    D: DateTime,
{
    pub(super) fn new(
        io: &'a mut TcpStream,
        addr: SocketAddr,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<H_LIMIT, R_LIMIT, W_LIMIT>,
        service: &'a S,
        date: &'a D,
    ) -> Self {
        Self {
            io,
            timer: Timer::new(timer, config.keep_alive_timeout, config.request_head_timeout),
            ctx: Context::<_, H_LIMIT>::with_addr(addr, date),
            service,
            read_buf: ReadBuf::<R_LIMIT>::new(),
            in_flight_read_buf: ReadBuf2::new(),
            write_buf: WriteBuf::<W_LIMIT>::new(),
            _phantom: PhantomData,
        }
    }

    pub(super) async fn run(mut self) -> Result<(), Error<S::Error, BE>> {
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
        self.timer.update(self.ctx.date().now());

        self.in_flight_read_buf
            .read_io(self.io)
            .timeout(self.timer.get())
            .await
            .map_err(|_| self.timer.map_to_err())??;

        self.read_buf.get_mut().extend_from_slice(self.in_flight_read_buf.get());

        while let Some((req, decoder)) = self.ctx.decode_head::<R_LIMIT>(&mut self.read_buf)? {
            self.timer.reset_state();

            let (mut body_reader, body) = BodyReader::from_coding(decoder);
            let req = req.map(|ext| ext.map_body(|_| ReqB::from(body)));

            {
                let read_task = read_body(
                    &self.ctx,
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

                match write_body::<S::Error, BE, W_LIMIT>(self.io, body, encoder, &mut self.write_buf)
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

fn read_body<'a, D, const H_LIMIT: usize, const R_LIMIT: usize>(
    ctx: &Context<'_, D, H_LIMIT>,
    body_reader: &'a mut BodyReader,
    read_buf: &'a mut ReadBuf<R_LIMIT>,
    read_buf2: &'a mut ReadBuf2,
    io: &'a TcpStream,
) -> impl Future<Output = io::Result<Infallible>> + 'a {
    let is_expect = ctx.is_expect_header();
    async move {
        if is_expect {
            // wait for service future to start polling RequestBody.
            if body_reader.wait_for_poll().await.is_ok() {
                let mut buf = BytesMut::new();
                Context::<D, H_LIMIT>::encode_continue(&mut buf);
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
}

async fn write_body<SE, BE, const W_LIMIT: usize>(
    io: &TcpStream,
    mut body: impl Stream<Item = Result<Bytes, BE>>,
    mut encoder: TransferCoding,
    write_buf: &mut WriteBuf<W_LIMIT>,
) -> Result<(), Error<SE, BE>> {
    let mut body = pin!(body);

    let mut task = pin!(None);
    let mut err = None;

    loop {
        let want_body = write_buf.get_mut().len() < W_LIMIT;

        let poll_body = async {
            if want_body {
                poll_fn(|cx| body.as_mut().poll_next(cx)).await
            } else {
                pending().await
            }
        };

        let poll_write = async {
            if task.is_none() {
                if write_buf.get_mut().is_empty() {
                    // pending when buffer is empty. wait for body to make progress
                    // with more bytes. (or exit with error)
                    return pending().await;
                }
                let buf = write_buf.get_mut().split();
                task.as_mut().set(Some(io.write_all(buf)));
            }
            task.as_mut().as_pin_mut().unwrap().await
        };

        match poll_body.select(poll_write).await {
            SelectOutput::A(Some(Ok(bytes))) => {
                encoder.encode(bytes, write_buf.get_mut());
            }
            SelectOutput::A(None) => {
                encoder.encode_eof(write_buf.get_mut());
                break;
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
