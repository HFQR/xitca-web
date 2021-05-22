use std::{
    future::Future,
    rc::Rc,
    task::{Context, Poll},
};

use actix_server_alt::net::UdpStream;
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::Stream;
use futures_intrusive::sync::LocalMutex;
use h3::{
    quic::SendStream,
    server::{self, RequestStream},
};
use http::{Request, Response};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlowSimple;
use crate::response::ResponseError;

use super::body::RequestBody;

pub struct H3Service<S> {
    flow: HttpFlowSimple<S>,
}

impl<S> H3Service<S> {
    /// Construct new Http3Service.
    /// No upgrade/expect services allowed in Http/3.
    pub fn new(service: S) -> Self {
        Self {
            flow: HttpFlowSimple::new(service),
        }
    }
}

impl<S, B, E> Service<UdpStream> for H3Service<S>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,

    S::Error: ResponseError<S::Response>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
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

    fn call<'c>(&'c self, req: UdpStream) -> Self::Future<'c>
    where
        UdpStream: 'c,
    {
        async move {
            // wait for connecting.
            let conn = req.connecting().await?;

            // construct h3 connection from quinn connection.
            let conn = h3_quinn::Connection::new(conn);
            let mut conn = server::Connection::new(conn).await?;

            // accept loop
            while let Some((req, stream)) = conn.accept().await? {
                // Reconstruct HttpRequest to attach crate body type.
                let (parts, _) = req.into_parts();

                // a hack to split read/write of request stream.
                // TODO: may deadlock?
                let stream = Rc::new(LocalMutex::new(stream, true));
                let sender = stream.clone();
                let body = async_stream::stream! {
                    while let Some(res) = sender.lock().await.recv_data().await.transpose() {
                        yield res;
                    }
                };
                let body = RequestBody(Box::pin(body));

                let req = Request::from_parts(parts, body);

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
    stream: Rc<LocalMutex<RequestStream<C>>>,
) -> Result<(), HttpServiceError>
where
    Fut: Future<Output = Result<Response<ResponseBody<B>>, E>>,
    C: SendStream<Bytes>,
    E: ResponseError<Response<ResponseBody<B>>>,
    B: Stream<Item = Result<Bytes, BE>>,
    BodyError: From<BE>,
{
    let res = fut.await.unwrap_or_else(ResponseError::response_error);

    let (res, body) = res.into_parts();
    let res = Response::from_parts(res, ());

    stream.lock().await.send_response(res).await?;

    tokio::pin!(body);

    while let Some(res) = body.as_mut().next().await {
        let bytes = res?;
        stream.lock().await.send_data(bytes).await?;
    }

    stream.lock().await.finish().await?;

    Ok(())
}
