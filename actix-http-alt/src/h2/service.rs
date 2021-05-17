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
use tokio::io::{AsyncRead, AsyncWrite};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlowSimple;
use crate::request::HttpRequest;
use crate::response::{HttpResponse, ResponseError};

use super::body::RequestBody;

pub struct H2Service<S, A> {
    flow: HttpFlowSimple<S>,
    tls_acceptor: A,
}

impl<S, A> H2Service<S, A>
where
    S: Service,
{
    /// Construct new Http2Service.
    /// No upgrade/expect services allowed in Http/2.
    pub fn new(service: S, tls_acceptor: A) -> Self {
        Self {
            flow: HttpFlowSimple::new(service),
            tls_acceptor,
        }
    }
}

#[rustfmt::skip]
impl<St, S, B, E, A, TlsSt> Service for H2Service<S, A>
where
    S: for<'r> Service<
            Request<'r> = HttpRequest<RequestBody>,
            Response = HttpResponse<ResponseBody<B>>,
        > + 'static,
    A: for<'r> Service<Request<'r> = St, Response = TlsSt> + 'static,

    S::Error: ResponseError<S::Response>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncRead + AsyncWrite + Unpin + 'static,
    TlsSt: AsyncRead + AsyncWrite + Unpin + 'static,

    HttpServiceError: From<A::Error>,
{
    type Request<'r> = St;
    type Response = ();
    type Error = HttpServiceError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.flow.poll_ready(cx).map_err(|_| HttpServiceError::ServiceReady)
    }

    fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
        async move {
            let tls_stream = self.tls_acceptor.call(req).await?;

            let mut conn = handshake(tls_stream).await?;

            while let Some(res) = conn.accept().await {
                let (req, tx) = res?;
                // Convert http::Request body type to crate::h2::Body
                // and reconstruct as HttpRequest.
                let (parts, body) = req.into_parts();
                let body = RequestBody::from(body);
                let req = HttpRequest::from_parts(parts, body);

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

async fn h2_handler<Fut, B, BE, E>(
    fut: Fut,
    mut tx: SendResponse<Bytes>,
) -> Result<(), HttpServiceError>
where
    Fut: Future<Output = Result<HttpResponse<ResponseBody<B>>, E>>,
    E: ResponseError<HttpResponse<ResponseBody<B>>>,
    B: Stream<Item = Result<Bytes, BE>>,
    BodyError: From<BE>,
{
    let res = fut.await.unwrap_or_else(ResponseError::response_error);

    let (res, body) = res.into_parts();
    let res = HttpResponse::from_parts(res, ());

    if body.is_eof() {
        let _ = tx.send_response(res, true)?;
        Ok(())
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

        Ok(())
    }
}

const CHUNK_SIZE: usize = 16_384;
