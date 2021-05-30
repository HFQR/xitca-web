use std::{io, marker::PhantomData};

use actix_server_alt::net::TcpStream;
use actix_service_alt::Service;
use bytes::{Buf, Bytes, BytesMut};
use futures_core::stream::Stream;
use http::{response::Parts, Request, Response};

use crate::body::ResponseBody;
use crate::error::BodyError;
use crate::flow::HttpFlow;
use crate::h1::{
    body::{RequestBody, RequestBodySender},
    error::Error,
};
use crate::response::ResponseError;
use crate::util::date::DateTask;

use super::context::Context;
use super::decode::RequestBodyDecoder;
use super::state::State;

pub(crate) struct Dispatcher<'a, S, B, X, U> {
    io: &'a mut TcpStream,
    state: State,
    ctx: Context<'a>,
    read_buf: ReadBuffer,
    write_buf: BytesMut,
    error: Option<Error>,
    flow: &'a HttpFlow<S, X, U>,
    _phantom: PhantomData<B>,
}

impl<'a, S, B, E, X, U> Dispatcher<'a, S, B, X, U>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    pub(crate) fn new(io: &'a mut TcpStream, flow: &'a HttpFlow<S, X, U>, date: &'a DateTask) -> Self {
        Self {
            io,
            state: State::new(),
            ctx: Context::new(date.get()),
            read_buf: ReadBuffer::new(),
            write_buf: BytesMut::new(),
            error: None,
            flow,
            _phantom: PhantomData,
        }
    }

    pub(crate) async fn run(&mut self) -> Result<(), Error> {
        loop {
            while let Some((req, body_handle)) = self.decode_head()? {
                let (parts, body) = self
                    .flow
                    .service
                    .call(req)
                    .await
                    .unwrap_or_else(ResponseError::response_error)
                    .into_parts();

                self.encode_head(parts, &body)?;

                let mut encoder = body.encoder();

                tokio::pin!(body);
                let buf = &mut self.write_buf;
                while let Some(bytes) = body.as_mut().next().await {
                    let bytes = bytes.unwrap();
                    encoder.encode(&bytes, buf)?;
                }
                encoder.encode_eof(buf)?;
            }

            while self.write_buf.has_remaining() {
                self.io.writable().await?;
                self.try_write()?;
            }

            self.io.readable().await?;
            self.try_read()?;
        }
    }

    fn try_read(&mut self) -> Result<(), Error> {
        self.read_buf.advance(false);

        loop {
            match self.io.try_read_buf(self.read_buf.buf()) {
                Ok(0) => return Err(Error::Closed),
                Ok(_) => self.read_buf.advance(true),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(ref e) if e.kind() == io::ErrorKind::ConnectionReset => {
                    self.set_read_close();
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn try_write(&mut self) -> Result<(), Error> {
        loop {
            match self.io.try_write(&mut self.write_buf) {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => {
                    self.write_buf.advance(n);
                    if self.write_buf.remaining() == 0 {
                        return Ok(());
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn decode_head(&mut self) -> Result<Option<(Request<RequestBody>, Option<RequestBodyHandle>)>, Error> {
        // Do not try when nothing new read.
        if self.read_buf.advanced() {
            let buf = self.read_buf.buf();

            if let Some((req, decoder)) = self.ctx.decode_head(buf)? {
                let (body_handle, body) = RequestBodyHandle::new_pair(decoder);

                let (parts, _) = req.into_parts();
                let req = Request::from_parts(parts, body);

                return Ok(Some((req, body_handle)));
            }
        }

        Ok(None)
    }

    fn encode_head(&mut self, parts: Parts, body: &ResponseBody<B>) -> Result<(), Error> {
        let size = body.size();
        self.ctx.encode_head(parts, size, &mut self.write_buf)?;
        Ok(())
    }

    fn set_read_close(&mut self) {
        self.state.set_read_close();
    }

    fn set_write_close(&mut self) {
        self.state.set_write_close();
    }
}

struct ReadBuffer {
    advanced: bool,
    buf: BytesMut,
}

impl ReadBuffer {
    fn new() -> Self {
        Self {
            advanced: false,
            buf: BytesMut::new(),
        }
    }

    fn buf(&mut self) -> &mut BytesMut {
        &mut self.buf
    }

    #[inline(always)]
    fn advanced(&self) -> bool {
        self.advanced
    }

    #[inline(always)]
    fn advance(&mut self, advanced: bool) {
        self.advanced = advanced;
    }
}

struct RequestBodyHandle {
    decoder: RequestBodyDecoder,
    sender: RequestBodySender,
}

impl RequestBodyHandle {
    fn new_pair(decoder: RequestBodyDecoder) -> (Option<RequestBodyHandle>, RequestBody) {
        if decoder.is_eof() {
            let (_, body) = RequestBody::create(true);

            (None, body)
        } else {
            let (sender, body) = RequestBody::create(false);

            let body_handle = RequestBodyHandle { decoder, sender };

            (Some(body_handle), body)
        }
    }

    fn close(&mut self) {
        self.sender.feed_eof();
    }
}
