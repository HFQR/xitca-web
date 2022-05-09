use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use futures_core::{ready, Stream};
use futures_intrusive::{
    sync::{GenericMutex, LocalMutex},
    NoopLock,
};
use h3::{
    quic::{RecvStream, SendStream},
    server::{self, RequestStream},
};
use pin_project_lite::pin_project;
use xitca_io::net::UdpStream;
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::{
    body::ResponseBody,
    bytes::{Buf, Bytes},
    error::HttpServiceError,
    h3::{body::RequestBody, error::Error},
    http::Response,
    request::{RemoteAddr, Request},
    util::futures::Queue,
};

/// Http/3 dispatcher
pub(crate) struct Dispatcher<'a, S, ReqB> {
    io: UdpStream,
    service: &'a S,
    _req_body: PhantomData<ReqB>,
}

impl<'a, S, ReqB, ResB, BE> Dispatcher<'a, S, ReqB>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<ResB>>>,
    S::Error: fmt::Debug,

    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,

    ReqB: From<RequestBody>,
{
    pub(crate) fn new(io: UdpStream, service: &'a S) -> Self {
        Self {
            io,
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
                    // a hack to split read/write of request stream.
                    // TODO: may deadlock?
                    let stream = Rc::new(LocalMutex::new(stream, true));
                    let sender = stream.clone();

                    let body = Box::pin(AsyncStream::new(sender, |stream| async move {
                        // What the fuck is this API? We need to find another http3 implementation on quinn
                        // that actually make sense. This is plain stupid.
                        let res = stream.lock().await.recv_data().await?;
                        Ok(res.map(|bytes| (Bytes::copy_from_slice(bytes.chunk()), stream)))
                    }));

                    // Reconstruct Request to attach crate body type.
                    let req =
                        Request::from_http(req, RemoteAddr::None).map_body(move |_| ReqB::from(RequestBody(body)));

                    queue.push(async move {
                        let fut = self.service.call(req);
                        h3_handler(fut, &*stream).await
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

async fn h3_handler<Fut, C, ResB, SE, BE>(
    fut: Fut,
    stream: &GenericMutex<NoopLock, RequestStream<C, Bytes>>,
) -> Result<(), Error<SE, BE>>
where
    Fut: Future<Output = Result<Response<ResponseBody<ResB>>, SE>>,
    C: RecvStream + SendStream<Bytes>,
    ResB: Stream<Item = Result<Bytes, BE>>,
{
    let (res, body) = fut.await.map_err(Error::Service)?.into_parts();
    let res = Response::from_parts(res, ());

    stream.lock().await.send_response(res).await?;

    tokio::pin!(body);

    while let Some(res) = body.as_mut().next().await {
        let bytes = res.map_err(Error::Body)?;
        stream.lock().await.send_data(bytes).await?;
    }

    stream.lock().await.finish().await?;

    Ok(())
}

pin_project! {
    struct AsyncStream<F, Arg, Fut>{
        callback: F,
        #[pin]
        inner: _AsyncStream<Arg, Fut>
    }
}

pin_project! {
    #[project = AsyncStreamProj]
    #[project_replace = AsyncStreamReplaceProj]
    enum _AsyncStream<Arg, Fut> {
        Arg {
            arg: Arg
        },
        Next {
            #[pin]
            fut: Fut
        },
        Empty
    }
}

impl<F, Arg, Fut> AsyncStream<F, Arg, Fut> {
    fn new(arg: Arg, callback: F) -> Self
    where
        F: Fn(Arg) -> Fut,
    {
        let fut = callback(arg);

        Self {
            callback,
            inner: _AsyncStream::Next { fut },
        }
    }
}

impl<F, Arg, Fut, Res, Err> Stream for AsyncStream<F, Arg, Fut>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Option<(Res, Arg)>, Err>>,
{
    type Item = Result<Res, Err>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match this.inner.as_mut().project() {
                AsyncStreamProj::Next { fut } => {
                    return match ready!(fut.poll(cx)) {
                        Ok(Some((bytes, arg))) => {
                            this.inner.set(_AsyncStream::Arg { arg });
                            Poll::Ready(Some(Ok(bytes)))
                        }
                        Ok(None) => {
                            this.inner.set(_AsyncStream::Empty);
                            Poll::Ready(None)
                        }
                        Err(e) => {
                            this.inner.set(_AsyncStream::Empty);
                            Poll::Ready(Some(Err(e)))
                        }
                    }
                }
                AsyncStreamProj::Arg { .. } => match this.inner.as_mut().project_replace(_AsyncStream::Empty) {
                    AsyncStreamReplaceProj::Arg { arg } => {
                        this.inner.set(_AsyncStream::Next {
                            fut: (this.callback)(arg),
                        });
                    }
                    _ => unreachable!("Never gonna happen"),
                },
                AsyncStreamProj::Empty => unreachable!("StreamRequest polled after finis"),
            }
        }
    }
}
