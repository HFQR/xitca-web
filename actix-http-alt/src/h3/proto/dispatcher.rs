use std::{future::Future, marker::PhantomData, rc::Rc};

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
use crate::flow::HttpFlow;
use crate::h3::RequestBody;
use crate::response::ResponseError;

/// Http/2 dispatcher
pub(crate) struct Dispatcher<'a, S, ReqB, X, U> {
    io: UdpStream,
    flow: &'a HttpFlow<S, X, U>,
    _req_body: PhantomData<ReqB>,
}

impl<'a, S, ReqB, X, U, B, E> Dispatcher<'a, S, ReqB, X, U>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: 'static,

    U: 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    ReqB: From<RequestBody> + 'static,
{
    pub(crate) fn new(io: UdpStream, flow: &'a HttpFlow<S, X, U>) -> Self {
        Self {
            io,
            flow,
            _req_body: PhantomData,
        }
    }

    pub(crate) async fn run(self) -> Result<(), HttpServiceError> {
        // wait for connecting.
        let conn = self.io.connecting().await?;

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
            let body = ReqB::from(RequestBody(Box::pin(body)));

            let req = Request::from_parts(parts, body);

            let flow = HttpFlow::clone(self.flow);
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
