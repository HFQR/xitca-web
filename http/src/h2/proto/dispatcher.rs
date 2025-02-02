use core::{
    cmp, fmt,
    future::{poll_fn, Future},
    marker::PhantomData,
    net::SocketAddr,
    pin::{pin, Pin},
    task::{ready, Context, Poll},
    time::Duration,
};

use ::h2::{
    server::{Connection, SendResponse},
    Ping, PingPong,
};
use futures_core::stream::Stream;
use tracing::trace;
use xitca_io::io::{AsyncRead, AsyncWrite};
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{
    body::BodySize,
    bytes::Bytes,
    date::{DateTime, DateTimeHandle},
    error::HttpServiceError,
    h2::{body::RequestBody, error::Error},
    http::{
        header::{HeaderMap, HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRAILER},
        Extension, Request, RequestExt, Response, Version,
    },
    util::{futures::Queue, timer::KeepAlive},
};

/// Http/2 dispatcher
pub(crate) struct Dispatcher<'a, TlsSt, S, ReqB> {
    io: &'a mut Connection<TlsSt, Bytes>,
    addr: SocketAddr,
    keep_alive: Pin<&'a mut KeepAlive>,
    ka_dur: Duration,
    pong_timeout: Duration,
    service: &'a S,
    date: &'a DateTimeHandle,
    _req_body: PhantomData<ReqB>,
}

impl<'a, TlsSt, S, ReqB, ResB, BE> Dispatcher<'a, TlsSt, S, ReqB>
where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,

    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,

    TlsSt: AsyncRead + AsyncWrite + Unpin,
    ReqB: From<RequestBody>,
{
    pub(crate) fn new(
        io: &'a mut Connection<TlsSt, Bytes>,
        addr: SocketAddr,
        keep_alive: Pin<&'a mut KeepAlive>,
        ka_dur: Duration,
        pong_timeout: Duration,
        service: &'a S,
        date: &'a DateTimeHandle,
    ) -> Self {
        Self {
            io,
            addr,
            keep_alive,
            ka_dur,
            pong_timeout,
            service,
            date,
            _req_body: PhantomData,
        }
    }

    pub(crate) async fn run(self) -> Result<(), Error<S::Error, BE>> {
        let Self {
            io,
            addr,
            mut keep_alive,
            ka_dur,
            pong_timeout,
            service,
            date,
            ..
        } = self;

        let ping_pong = io.ping_pong().expect("first call to ping_pong should never fail");

        // reset timer to keep alive.
        let deadline = date.now() + ka_dur;
        keep_alive.as_mut().update(deadline);

        // timer for ping pong interval and keep alive.
        let mut ping_pong = H2PingPong {
            on_flight: false,
            keep_alive: keep_alive.as_mut(),
            ping_pong,
            date,
            ka_dur,
            pong_timeout,
        };

        let mut queue = Queue::new();

        loop {
            match io.accept().select(try_poll_queue(&mut queue, &mut ping_pong)).await {
                SelectOutput::A(Some(Ok((req, tx)))) => {
                    // Convert http::Request body type to crate::h2::Body
                    // and reconstruct as HttpRequest.
                    let req = req.map(|body| {
                        let body = ReqB::from(RequestBody::from(body));
                        RequestExt::from_parts(body, Extension::new(addr))
                    });

                    queue.push(async move {
                        let fut = service.call(req);
                        h2_handler(fut, tx, date).await
                    });
                }
                SelectOutput::B(SelectOutput::A(_)) => io.graceful_shutdown(),
                SelectOutput::B(SelectOutput::B(Ok(_))) => {
                    trace!("Connection keep-alive timeout. Shutting down");
                    return Ok(());
                }
                SelectOutput::A(None) => {
                    trace!("Connection closed by remote. Shutting down");
                    break;
                }
                SelectOutput::A(Some(Err(e))) | SelectOutput::B(SelectOutput::B(Err(e))) => return Err(From::from(e)),
            }
        }

        queue.drain().await;

        Ok(())
    }
}

async fn try_poll_queue<F, E, S, B>(
    queue: &mut Queue<F>,
    ping_ping: &mut H2PingPong<'_>,
) -> SelectOutput<(), Result<(), ::h2::Error>>
where
    F: Future<Output = Result<ConnectionState, E>>,
    HttpServiceError<S, B>: From<E>,
    S: fmt::Debug,
    B: fmt::Debug,
{
    loop {
        if queue.is_empty() {
            return SelectOutput::B(ping_ping.await);
        }

        match queue.next2().await {
            Ok(ConnectionState::KeepAlive) => {}
            Ok(ConnectionState::Close) => return SelectOutput::A(()),
            Err(e) => HttpServiceError::from(e).log("h2_dispatcher"),
        }
    }
}

struct H2PingPong<'a> {
    on_flight: bool,
    keep_alive: Pin<&'a mut KeepAlive>,
    ping_pong: PingPong,
    date: &'a DateTimeHandle,
    ka_dur: Duration,
    pong_timeout: Duration,
}

impl Future for H2PingPong<'_> {
    type Output = Result<(), ::h2::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            if this.on_flight {
                // When have on flight ping pong. poll pong and and keep alive timer.
                // on success pong received update keep alive timer to determine the next timing of
                // ping pong.
                match this.ping_pong.poll_pong(cx)? {
                    Poll::Ready(_) => {
                        this.on_flight = false;

                        let deadline = this.date.now() + this.ka_dur;

                        this.keep_alive.as_mut().update(deadline);
                        this.keep_alive.as_mut().reset();
                    }
                    Poll::Pending => return this.keep_alive.as_mut().poll(cx).map(|_| Ok(())),
                }
            } else {
                // When there is no on flight ping pong. keep alive timer is used to wait for next
                // timing of ping pong. Therefore at this point it serves as an interval instead.

                ready!(this.keep_alive.as_mut().poll(cx));

                this.ping_pong.send_ping(Ping::opaque())?;

                // Update the keep alive to the pong timeout, if pong is not received within this
                // time. The connection is considered dead.
                let deadline = this.date.now() + this.ka_dur + this.pong_timeout;

                this.keep_alive.as_mut().update(deadline);

                this.on_flight = true;
            }
        }
    }
}

enum ConnectionState {
    KeepAlive,
    Close,
}

// handle request/response and return if connection should go into graceful shutdown.
async fn h2_handler<Fut, B, SE, BE>(
    fut: Fut,
    mut tx: SendResponse<Bytes>,
    date: &DateTimeHandle,
) -> Result<ConnectionState, Error<SE, BE>>
where
    Fut: Future<Output = Result<Response<B>, SE>>,
    B: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
{
    // split response to header and body.
    let (res, body) = fut.await.map_err(Error::Service)?.into_parts();
    let mut res = Response::from_parts(res, ());

    // set response version.
    *res.version_mut() = Version::HTTP_2;

    // check eof state of response body and make sure header is valid.
    let is_eof = match BodySize::from_stream(&body) {
        BodySize::None => {
            debug_assert!(!res.headers().contains_key(CONTENT_LENGTH));
            true
        }
        BodySize::Stream => {
            debug_assert!(!res.headers().contains_key(CONTENT_LENGTH));
            false
        }
        BodySize::Sized(n) => {
            // add an content-length header if there is non provided.
            if !res.headers().contains_key(CONTENT_LENGTH) {
                res.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from(n));
            }
            n == 0
        }
    };

    let mut trailers = HeaderMap::with_capacity(0);

    while let Some(value) = res.headers_mut().remove(TRAILER) {
        let name = HeaderName::from_bytes(value.as_bytes()).unwrap();
        let value = res.headers_mut().remove(name.clone()).unwrap();
        trailers.append(name, value);
    }

    if !res.headers().contains_key(DATE) {
        let date = date.with_date(HeaderValue::from_bytes).unwrap();
        res.headers_mut().insert(DATE, date);
    }

    // check response header to determine if user want connection be closed.
    let state = res
        .headers_mut()
        .remove(CONNECTION)
        .and_then(|v| {
            v.as_bytes()
                .eq_ignore_ascii_case(b"close")
                .then_some(ConnectionState::Close)
        })
        .unwrap_or(ConnectionState::KeepAlive);

    // send response and body(if there is one).
    let mut stream = tx.send_response(res, is_eof)?;

    if !is_eof {
        let mut body = pin!(body);

        while let Some(res) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
            let mut chunk = res.map_err(Error::Body)?;

            while !chunk.is_empty() {
                let len = chunk.len();

                stream.reserve_capacity(cmp::min(len, CHUNK_SIZE));

                let cap = poll_fn(|cx| stream.poll_capacity(cx))
                    .await
                    .expect("No capacity left. http2 response is dropped")?;

                // Split chuck to writeable size and send to client.
                let bytes = chunk.split_to(cmp::min(cap, len));

                stream.send_data(bytes, false)?;
            }
        }
    }

    stream.send_trailers(trailers)?;

    Ok(state)
}

const CHUNK_SIZE: usize = 16_384;
