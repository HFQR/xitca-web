use std::{
    future::Future,
    task::{Context, Poll},
};

use actix_server_alt::net::UdpStream;
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::Stream;
use h3::{
    quic::SendStream,
    server::{self, RequestStream},
};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlowSimple;
use crate::request::HttpRequest;
use crate::response::{HttpResponse, ResponseError};

use super::body::RequestBody;

pub struct H3Service<S> {
    flow: HttpFlowSimple<S>,
}

impl<S> H3Service<S>
where
    S: Service,
{
    /// Construct new Http3Service.
    /// No upgrade/expect services allowed in Http/3.
    pub fn new(service: S) -> Self {
        Self {
            flow: HttpFlowSimple::new(service),
        }
    }
}

#[rustfmt::skip]
impl<S, B, E> Service for H3Service<S>
where
    S: for<'r> Service<
            Request<'r> = HttpRequest<RequestBody>,
            Response = HttpResponse<ResponseBody<B>>,
        > + 'static,

    S::Error: ResponseError<S::Response>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    type Request<'r> = UdpStream;
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

    fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
        async move {
            // wait for connecting.
            let conn = req.connecting().await?;

            // construct h3 connection from quinn connection.
            let conn = h3_quinn::Connection::new(conn);
            let mut conn = server::Connection::new(conn).await?;

            // accept loop
            while let Some(res) = conn.accept().await.transpose() {
                // TODO: pass stream receiver to HttpRequest.
                let (req, stream) = res?;

                // Reconstruct HttpRequest to attach crate body type.
                let (parts, _) = req.into_parts();
                let body = RequestBody;
                let req = HttpRequest::from_parts(parts, body);

                let flow = self.flow.clone();
                tokio::task::spawn_local(async move {
                    let fut = flow.service.call(req);
                    if let Err(e) = h3_handler(fut, stream).await {
                        e.log();
                    }
                });
            }

            Ok(())
        }
    }
}

async fn h3_handler<Fut, C, B, BE, E>(
    fut: Fut,
    mut stream: RequestStream<C>,
) -> Result<(), HttpServiceError>
where
    Fut: Future<Output = Result<HttpResponse<ResponseBody<B>>, E>>,
    C: SendStream<Bytes>,
    E: ResponseError<HttpResponse<ResponseBody<B>>>,
    B: Stream<Item = Result<Bytes, BE>>,
    BodyError: From<BE>,
{
    let res = fut.await.unwrap_or_else(ResponseError::response_error);

    let (res, body) = res.into_parts();
    let res = HttpResponse::from_parts(res, ());

    stream.send_response(res).await?;

    tokio::pin!(body);

    while let Some(res) = body.as_mut().next().await {
        let bytes = res?;
        stream.send_data(bytes).await?;
    }

    Ok(())
}
