use std::{
    cmp,
    future::Future,
    task::{Context, Poll},
};

use ::h2::server::{handshake, SendResponse};
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::Stream;
use futures_util::future::poll_fn;
use http::{header::CONTENT_LENGTH, HeaderValue, Request, Response, Version};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::body::{ResponseBody, ResponseBodySize};
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlowSimple;
use crate::response::ResponseError;

use super::body::RequestBody;

pub struct H2Service<S, A> {
    flow: HttpFlowSimple<S>,
    tls_acceptor: A,
}

impl<S, A> H2Service<S, A> {
    /// Construct new Http2Service.
    /// No upgrade/expect services allowed in Http/2.
    pub fn new(service: S, tls_acceptor: A) -> Self {
        Self {
            flow: HttpFlowSimple::new(service),
            tls_acceptor,
        }
    }
}

impl<St, S, B, E, A, TlsSt> Service<St> for H2Service<S, A>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    A: Service<St, Response = TlsSt> + 'static,

    S::Error: ResponseError<S::Response>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncRead + AsyncWrite + Unpin,
    TlsSt: AsyncRead + AsyncWrite + Unpin,

    HttpServiceError: From<A::Error>,
{
    type Response = ();
    type Error = HttpServiceError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.flow
            .service
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady)
    }

    fn call<'c>(&'c self, req: St) -> Self::Future<'c>
    where
        St: 'c,
    {
        async move {
            let tls_stream = self.tls_acceptor.call(req).await?;

            let mut conn = handshake(tls_stream).await?;

            while let Some(res) = conn.accept().await {
                let (req, tx) = res?;
                // Convert http::Request body type to crate::h2::Body
                // and reconstruct as HttpRequest.
                let (parts, body) = req.into_parts();
                let body = RequestBody::from(body);
                let req = Request::from_parts(parts, body);

                let flow = self.flow.clone();

                tokio::task::spawn_local(async move {
                    let fut = flow.service.call(req);
                    if let Err(e) = h2_handler(fut, tx).await {
                        e.log();
                    }
                });
            }

            Ok(())
        }
    }
}

async fn h2_handler<Fut, B, BE, E>(fut: Fut, mut tx: SendResponse<Bytes>) -> Result<(), HttpServiceError>
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
    match body.size() {
        ResponseBodySize::Sized(n) => {
            res.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from(n));
        }
        _ => {}
    }

    // send response and body(if there is one).
    if body.is_eof() {
        let _ = tx.send_response(res, true)?;
    } else {
        let mut stream = tx.send_response(res, false)?;

        tokio::pin!(body);

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
