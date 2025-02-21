use core::{
    fmt,
    future::{Future, poll_fn},
    marker::PhantomData,
    net::SocketAddr,
    pin::pin,
};

use ::h3::{
    quic::SendStream,
    server::{self, RequestStream},
};
use futures_core::stream::Stream;
use xitca_io::net::QuicStream;
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::{
    bytes::Bytes,
    error::HttpServiceError,
    h3::{body::RequestBody, error::Error},
    http::{Extension, Request, RequestExt, Response},
    util::futures::Queue,
};

/// Http/3 dispatcher
pub(crate) struct Dispatcher<'a, S, ReqB> {
    io: QuicStream,
    addr: SocketAddr,
    service: &'a S,
    _req_body: PhantomData<ReqB>,
}

impl<'a, S, ReqB, ResB, BE> Dispatcher<'a, S, ReqB>
where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
    ReqB: From<RequestBody>,
{
    pub(crate) fn new(io: QuicStream, addr: SocketAddr, service: &'a S) -> Self {
        Self {
            io,
            addr,
            service,
            _req_body: PhantomData,
        }
    }

    pub(crate) async fn run(self) -> Result<(), Error<S::Error, BE>> {
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
                    let (tx, rx) = stream.split();

                    // Reconstruct Request to attach crate body type.
                    let req = req.map(|_| {
                        let body = ReqB::from(RequestBody(rx));
                        RequestExt::from_parts(body, Extension::new(self.addr))
                    });

                    queue.push(async move {
                        let fut = self.service.call(req);
                        h3_handler(fut, tx).await
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

async fn h3_handler<'a, Fut, C, ResB, SE, BE>(
    fut: Fut,
    mut stream: RequestStream<C, Bytes>,
) -> Result<(), Error<SE, BE>>
where
    Fut: Future<Output = Result<Response<ResB>, SE>> + 'a,
    C: SendStream<Bytes>,
    ResB: Stream<Item = Result<Bytes, BE>>,
{
    let (parts, body) = fut.await.map_err(Error::Service)?.into_parts();
    let res = Response::from_parts(parts, ());
    stream.send_response(res).await?;

    let mut body = pin!(body);

    while let Some(res) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
        let bytes = res.map_err(Error::Body)?;
        stream.send_data(bytes).await?;
    }

    stream.finish().await?;

    Ok(())
}
