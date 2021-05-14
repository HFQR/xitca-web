use std::{
    cmp,
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::future::poll_fn;
use h2::server::{handshake, SendResponse};
use http::Response;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::http::error::HttpServiceError;
use crate::http::flow::HttpFlow;
use crate::http::request::HttpRequest;
use crate::service::Service;

use super::body::RequestBody;

pub struct H2Service<St, S, A, TlsSt> {
    tls_acceptor: A,
    flow: HttpFlow<S>,
    _stream: PhantomData<(St, TlsSt)>,
}

impl<St, S, A, TlsSt> H2Service<St, S, A, TlsSt> {
    /// Construct new Http2Service.
    /// No upgrade/expect services allowed in Http/2.
    pub fn new(service: S, tls_acceptor: A) -> Self {
        Self {
            tls_acceptor,
            flow: HttpFlow::new(service),
            _stream: PhantomData,
        }
    }
}

#[rustfmt::skip]
impl<St, S, A, TlsSt> Service for H2Service<St, S, A, TlsSt>
where
    S: for<'r> Service<Request<'r> = HttpRequest<RequestBody>, Response = Response<Bytes>> + 'static,
    A: for<'r> Service<Request<'r> = St, Response = TlsSt> + 'static,

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
        self.flow
            .service
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady)
    }

    fn call<'s, 'r, 'f>(&'s self, req: Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
        async move {
            let tls_stream = self.tls_acceptor.call(req).await?;

            let mut conn = handshake(tls_stream).await?;

            while let Some(Ok((req, tx))) = conn.accept().await {
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

async fn h2_handler<Fut, E>(fut: Fut, mut tx: SendResponse<Bytes>) -> Result<(), HttpServiceError>
where
    Fut: Future<Output = Result<Response<Bytes>, E>>,
{
    let res = fut
        .await
        .unwrap_or_else(|_| Response::builder().status(500).body(Bytes::new()).unwrap());

    let (res, mut body) = res.into_parts();
    let res = Response::from_parts(res, ());

    let mut stream = tx.send_response(res, false)?;

    while !body.is_empty() {
        stream.reserve_capacity(cmp::min(body.len(), CHUNK_SIZE));

        match poll_fn(|cx| stream.poll_capacity(cx)).await {
            // No capacity left. drop body and return.
            None => return Ok(()),
            Some(res) => {
                // Split chuck to writeable size and send to client.
                let cap = res.unwrap();

                let len = body.len();
                let bytes = body.split_to(cmp::min(cap, len));

                stream.send_data(bytes, false)?;
            }
        }
    }

    stream.send_data(Bytes::new(), true)?;

    Ok(())
}

const CHUNK_SIZE: usize = 16_384;
