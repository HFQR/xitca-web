use std::{io, marker::PhantomData};

use actix_server_alt::net::TcpStream;
use actix_service_alt::Service;
use bytes::{Bytes, BytesMut};
use futures_core::stream::Stream;
use http::{Request, Response};
use pin_project_lite::pin_project;
use tokio::io::{Interest, Ready};

use super::context::Context;
use super::decode::{RequestBodyDecoder, RequestBodyItem};
use super::state::State;

use crate::body::ResponseBody;
use crate::error::BodyError;
use crate::flow::HttpFlow;
use crate::h1::{
    body::{RequestBody, RequestBodySender},
    error::Error,
};
use crate::response::ResponseError;

pub(crate) struct Dispatcher<'a, S, B, X, U> {
    io: TcpStream,
    state: State,
    context: Context,
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
    pub(crate) fn new(io: TcpStream, flow: &'a HttpFlow<S, X, U>) -> Self {
        Self {
            io,
            state: State::new(),
            context: Context::new(),
            read_buf: ReadBuffer::new(),
            write_buf: BytesMut::new(),
            error: None,
            flow,
            _phantom: PhantomData,
        }
    }

    pub(crate) async fn run(&mut self) -> Result<(), Error> {
        while !self.state.read_closed() {
            self.io.readable().await?;
            self.try_read()?;
            match self.decode_head()? {
                Some((req, body_handle)) => {
                    log::trace!("New Request with headers: {:?}", req.headers());

                    let res = self.flow.service.call(req).await;

                    self.try_encode(res)?;
                    self.io.writable().await;
                    // self.try_write()?;
                }
                None => continue,
            }
        }

        Ok(())
    }

    async fn handle_request(&mut self) -> Result<(), Error> {
        // while let Some(req) = self.queue.pop_front() {
        //     let res = self.flow.service.call(req).await;
        //     self.try_encode(res)?;
        // }

        Ok(())
    }

    fn try_read(&mut self) -> Result<(), Error> {
        self.read_buf.advance(false);

        loop {
            match self.io.try_read_buf(self.read_buf.buf()) {
                Ok(0) => {
                    self.set_read_close();
                    return Ok(());
                }
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

    fn running(&self) -> bool {
        !(self.state.read_closed() && self.state.write_closed())
    }

    fn decode_head(&mut self) -> Result<Option<(Request<RequestBody>, Option<RequestBodyHandle>)>, Error> {
        // Do not try when nothing new read.
        if self.read_buf.advanced() {
            let buf = self.read_buf.buf();

            match self.context.decode_head(buf)? {
                Some((req, decoder)) => {
                    let (body_handle, body) = RequestBodyHandle::new_pair(decoder);

                    let (parts, _) = req.into_parts();
                    let req = Request::from_parts(parts, body);

                    return Ok(Some((req, body_handle)));
                }
                // Nothing new decoded.
                None => {
                    // should disable write interest checking.
                }
            }
        }

        Ok(None)
    }

    // fn decode_body(&mut self) -> Result<(), Error> {
    //     // Do not try when nothing new read.
    //     if self.read_buf.advanced() {
    //         let buf = self.read_buf.buf();
    //
    //         let body_handle = self.body_handle.as_mut().unwrap();
    //
    //         while let Some(item) = body_handle.decoder.decode(buf)? {
    //             match item {
    //                 RequestBodyItem::Chunk(chunk) => body_handle.sender.feed_data(chunk),
    //                 RequestBodyItem::Eof => {
    //                     body_handle.sender.feed_eof();
    //                 }
    //             }
    //         }
    //     }
    //
    //     Ok(())
    // }

    fn try_encode(&mut self, res: Result<S::Response, S::Error>) -> Result<(), Error> {
        let res = res.unwrap_or_else(ResponseError::response_error);

        let (parts, body) = res.into_parts();
        use futures_core::Stream;
        let size = body.size_hint();

        Ok(())
    }

    fn set_read_close(&mut self) {
        self.state.set_read_close();
    }

    fn set_write_close(&mut self) {
        self.state.set_write_close();
    }
}

pin_project! {
    struct Task<Fut> {
        fut: Fut
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
