use std::{cmp, future::Future, marker::PhantomData};

use ::h2::server::{Connection, SendResponse};
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::Stream;
use http::{header::CONTENT_LENGTH, HeaderValue, Request, Response, Version};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    pin,
};

use crate::body::{ResponseBody, ResponseBodySize};
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlow;
use crate::h2::{body::RequestBody, error::Error};
use crate::response::ResponseError;
use crate::util::poll_fn::poll_fn;

/// Http/2 dispatcher
pub(crate) struct Dispatcher<'a, TlsSt, S, ReqB, X, U> {
    io: &'a mut Connection<TlsSt, Bytes>,
    flow: &'a HttpFlow<S, X, U>,
    _req_body: PhantomData<ReqB>,
}

impl<'a, TlsSt, S, ReqB, X, U, B, E> Dispatcher<'a, TlsSt, S, ReqB, X, U>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: 'static,

    U: 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    TlsSt: AsyncRead + AsyncWrite + Unpin,
    ReqB: From<RequestBody> + 'static,
{
    pub(crate) fn new(io: &'a mut Connection<TlsSt, Bytes>, flow: &'a HttpFlow<S, X, U>) -> Self {
        Self {
            io,
            flow,
            _req_body: PhantomData,
        }
    }

    pub(crate) async fn run(self) -> Result<(), Error> {
        while let Some(res) = self.io.accept().await {
            let (req, tx) = res?;
            // Convert http::Request body type to crate::h2::Body
            // and reconstruct as HttpRequest.
            let (parts, body) = req.into_parts();
            let body = ReqB::from(RequestBody::from(body));
            let req = Request::from_parts(parts, body);

            let flow = HttpFlow::clone(self.flow);

            tokio::task::spawn_local(async move {
                let fut = flow.service.call(req);
                if let Err(e) = h2_handler(fut, tx).await {
                    HttpServiceError::from(e).log();
                }
            });
        }

        Ok(())
    }
}

async fn h2_handler<Fut, B, BE, E>(fut: Fut, mut tx: SendResponse<Bytes>) -> Result<(), Error>
where
    Fut: Future<Output = Result<Response<ResponseBody<B>>, E>>,
    E: ResponseError<Response<ResponseBody<B>>>,
    B: Stream<Item = Result<Bytes, BE>>,
    BodyError: From<BE>,
{
    // resolve service call. map error to response.
    let res = fut.await.unwrap_or_else(ResponseError::response_error);

    // split response to header and body.
    let (res, body) = res.into_parts();
    let mut res = Response::from_parts(res, ());

    // set response version.
    *res.version_mut() = Version::HTTP_2;

    // set content length header.
    if let ResponseBodySize::Sized(n) = body.size() {
        res.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from(n));
    }

    // send response and body(if there is one).
    if body.is_eof() {
        let _ = tx.send_response(res, true)?;
    } else {
        let mut stream = tx.send_response(res, false)?;

        pin!(body);

        while let Some(res) = body.as_mut().next().await {
            let mut chunk = res?;

            'send: loop {
                stream.reserve_capacity(cmp::min(chunk.len(), CHUNK_SIZE));

                match poll_fn(|cx| stream.poll_capacity(cx)).await {
                    // No capacity left. drop body and return.
                    None => return Ok(()),
                    Some(res) => {
                        // Split chuck to writeable size and send to client.
                        let cap = res?;

                        let len = chunk.len();
                        let bytes = chunk.split_to(cmp::min(cap, len));

                        stream.send_data(bytes, false)?;

                        if chunk.is_empty() {
                            break 'send;
                        }
                    }
                }
            }
        }

        stream.send_data(Bytes::new(), true)?;
    }

    Ok(())
}

const CHUNK_SIZE: usize = 16_384;
