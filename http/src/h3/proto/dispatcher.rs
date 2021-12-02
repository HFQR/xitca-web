use std::{fmt, future::Future, marker::PhantomData, rc::Rc};

use futures_core::Stream;
use futures_intrusive::sync::LocalMutex;
use h3::{
    quic::BidiStream,
    server::{self, RequestStream},
};
use xitca_io::net::UdpStream;
use xitca_service::Service;

use crate::{
    body::ResponseBody,
    bytes::Bytes,
    error::{BodyError, HttpServiceError},
    h3::{body::RequestBody, error::Error},
    http::{Request, Response},
    util::futures::{Queue, Select, SelectOutput},
};

/// Http/3 dispatcher
pub(crate) struct Dispatcher<'a, S, ReqB> {
    io: UdpStream,
    service: &'a S,
    _req_body: PhantomData<ReqB>,
}

impl<'a, S, ReqB, B, E> Dispatcher<'a, S, ReqB>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: fmt::Debug,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    ReqB: From<RequestBody> + 'static,
{
    pub(crate) fn new(io: UdpStream, service: &'a S) -> Self {
        Self {
            io,
            service,
            _req_body: PhantomData,
        }
    }

    pub(crate) async fn run(self) -> Result<(), Error<S::Error>> {
        // wait for connecting.
        let conn = self.io.connecting().await?;

        // construct h3 connection from quinn connection.
        let conn = h3_quinn::Connection::new(conn);
        let mut conn = server::Connection::new(conn).await?;

        let mut queue = Queue::new();

        // accept loop
        loop {
            match conn.accept().select(queue.next()).await {
                SelectOutput::A(Ok(Some((req, stream)))) => {
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

                    queue.push(async move {
                        let fut = self.service.call(req);
                        h3_handler(fut, stream).await
                    });
                }
                SelectOutput::A(Ok(None)) => break,
                SelectOutput::A(Err(e)) => return Err(e.into()),
                SelectOutput::B(res) => {
                    if let Err(e) = res {
                        HttpServiceError::from(e).log("h3_dispatcher");
                    }
                }
            }
        }

        queue.drain().await;

        Ok(())
    }
}

async fn h3_handler<'a, Fut, C, B, BE, E>(fut: Fut, stream: Rc<LocalMutex<RequestStream<C>>>) -> Result<(), Error<E>>
where
    Fut: Future<Output = Result<Response<ResponseBody<B>>, E>> + 'a,
    C: BidiStream<Bytes>,
    B: Stream<Item = Result<Bytes, BE>>,
    BodyError: From<BE>,
{
    let (res, body) = fut.await.map_err(Error::Service)?.into_parts();
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
