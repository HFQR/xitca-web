use std::{
    cmp,
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::Stream;
use futures_util::future::poll_fn;
use h2::server::{handshake, SendResponse};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::http::{
    body::ResponseBody, error::BodyError, error::HttpServiceError, flow::HttpFlow,
    request::HttpRequest, response::HttpResponse,
};
use crate::service::Service;

use super::body::RequestBody;

pub struct H2Service<St, S, B, A, TlsSt> {
    tls_acceptor: A,
    flow: HttpFlow<S>,
    _stream: PhantomData<(St, B, TlsSt)>,
}

impl<St, S, B, A, TlsSt> H2Service<St, S, B, A, TlsSt> {
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
impl<St, S, B, E, A, TlsSt> Service for H2Service<St, S, B, A, TlsSt>
where
    S: for<'r> Service<Request<'r> = HttpRequest<RequestBody>, Response = HttpResponse<ResponseBody<B>>> + 'static,
    A: for<'r> Service<Request<'r> = St, Response = TlsSt> + 'static,

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
    B: Stream<Item = Result<Bytes, BE>>,
    BodyError: From<BE>,
{
    let res = fut.await.unwrap_or_else(|_| {
        HttpResponse::builder()
            .status(500)
            .body(ResponseBody::None)
            .unwrap()
    });

    let (res, body) = res.into_parts();
    let res = HttpResponse::from_parts(res, ());

    if body.is_eof() {
        let _ = tx.send_response(res, true)?;
        Ok(())
    } else {
        let mut stream = tx.send_response(res, false)?;

        tokio::pin!(body);

        // TODO: remove dependent on futures_util
        use futures_util::StreamExt;
        while let Some(res) = body.next().await {
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
