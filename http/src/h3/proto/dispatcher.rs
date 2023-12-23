use core::{
    fmt,
    future::{poll_fn, Future},
    marker::PhantomData,
    pin::{pin, Pin},
    task::{ready, Context, Poll},
};

use std::net::SocketAddr;

use ::h3::{
    quic::SendStream,
    server::{self, RequestStream},
};
use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use xitca_io::net::UdpStream;
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::{
    bytes::{Buf, Bytes},
    error::HttpServiceError,
    h3::{body::RequestBody, error::Error},
    http::{Extension, Request, RequestExt, Response},
    util::futures::Queue,
};

/// Http/3 dispatcher
pub(crate) struct Dispatcher<'a, S, ReqB> {
    io: UdpStream,
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
    pub(crate) fn new(io: UdpStream, addr: SocketAddr, service: &'a S) -> Self {
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

                    let body = Box::pin(AsyncStream::new(rx, |mut stream| async move {
                        // What the fuck is this API? We need to find another http3 implementation on quinn
                        // that actually make sense. This is plain stupid.
                        let res = stream.recv_data().await?;
                        Ok(res.map(|bytes| (Bytes::copy_from_slice(bytes.chunk()), stream)))
                    }));

                    let req = http_0dot2_to_1(req.into_parts().0);

                    // Reconstruct Request to attach crate body type.
                    let req = req.map(|_| {
                        let body = ReqB::from(RequestBody(body));
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
    let (res, body) = fut.await.map_err(Error::Service)?.into_parts();

    let res = http_1_to_0dot2(res);

    stream.send_response(res).await?;

    let mut body = pin!(body);

    while let Some(res) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
        let bytes = res.map_err(Error::Body)?;
        stream.send_data(bytes).await?;
    }

    stream.finish().await?;

    Ok(())
}

fn http_1_to_0dot2(mut res: crate::http::response::Parts) -> http_0_dot_2::Response<()> {
    use http_0_dot_2::{Response, StatusCode, Version};

    let mut builder = Response::builder()
        .status(StatusCode::from_u16(res.status.as_u16()).unwrap())
        .version(Version::HTTP_3);

    let mut last = None;
    for (k, v) in res.headers.drain() {
        if k.is_some() {
            last = k;
        }
        let name = last.as_ref().unwrap();
        builder = builder.header(name.as_str(), v.as_bytes());
    }

    builder.body(()).unwrap()
}

fn http_0dot2_to_1(mut parts: http_0_dot_2::request::Parts) -> Request<()> {
    use crate::http::{Method, Uri};

    let mut builder = Request::builder()
        .method(Method::from_bytes(parts.method.as_str().as_bytes()).unwrap())
        .uri(Uri::try_from(parts.uri.to_string().as_str()).unwrap());

    let mut last = None;
    for (k, v) in parts.headers.drain() {
        if k.is_some() {
            last = k;
        }
        let name = last.as_ref().unwrap();
        builder = builder.header(name.as_str(), v.as_bytes());
    }

    builder.body(()).unwrap()
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
