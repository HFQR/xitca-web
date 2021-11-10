use std::{
    cmp, fmt,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use ::h2::{
    server::{Connection, SendResponse},
    Ping, PingPong,
};
use futures_core::{ready, Stream};
use futures_util::stream::{FuturesUnordered, StreamExt};
use http::{
    header::{CONTENT_LENGTH, DATE},
    HeaderValue, Request, Response, Version,
};
use tokio::pin;
use tracing::trace;
use xitca_io::io::{AsyncRead, AsyncWrite};
use xitca_service::Service;

use crate::{
    body::{ResponseBody, ResponseBodySize},
    bytes::Bytes,
    date::{DateTime, DateTimeHandle},
    error::{BodyError, HttpServiceError},
    flow::HttpFlow,
    h2::{body::RequestBody, error::Error},
    util::{
        futures::{never, poll_fn, Select, SelectOutput},
        keep_alive::KeepAlive,
    },
};

/// Http/2 dispatcher
pub(crate) struct Dispatcher<'a, TlsSt, S, ReqB, X> {
    io: &'a mut Connection<TlsSt, Bytes>,
    keep_alive: Pin<&'a mut KeepAlive>,
    ka_dur: Duration,
    flow: &'a HttpFlow<S, X>,
    date: &'a DateTimeHandle,
    _req_body: PhantomData<ReqB>,
}

impl<'a, TlsSt, S, ReqB, X, B, E> Dispatcher<'a, TlsSt, S, ReqB, X>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: fmt::Debug,

    X: 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    TlsSt: AsyncRead + AsyncWrite + Unpin,
    ReqB: From<RequestBody> + 'static,
{
    pub(crate) fn new(
        io: &'a mut Connection<TlsSt, Bytes>,
        keep_alive: Pin<&'a mut KeepAlive>,
        ka_dur: Duration,
        flow: &'a HttpFlow<S, X>,
        date: &'a DateTimeHandle,
    ) -> Self {
        Self {
            io,
            keep_alive,
            ka_dur,
            flow,
            date,
            _req_body: PhantomData,
        }
    }

    pub(crate) async fn run(self) -> Result<(), Error<S::Error>> {
        let Self {
            io,
            mut keep_alive,
            ka_dur,
            flow,
            date,
            ..
        } = self;

        let ping_pong = io.ping_pong().unwrap();

        // reset timer to keep alive.
        let deadline = date.now() + ka_dur;
        keep_alive.as_mut().update(deadline);

        // timer for ping pong interval and keep alive.
        let mut ping_pong = H2PingPong {
            on_flight: false,
            keep_alive: keep_alive.as_mut(),
            ping_pong,
            date: &*date,
            ka_dur,
        };

        let mut queue = FuturesUnordered::new();
        let mut queued = false;

        loop {
            match io
                .accept()
                .select(try_queue(queued, &mut queue))
                .select(&mut ping_pong)
                .await
            {
                SelectOutput::A(SelectOutput::A(Some(Ok((req, tx))))) => {
                    // Convert http::Request body type to crate::h2::Body
                    // and reconstruct as HttpRequest.
                    let (parts, body) = req.into_parts();
                    let body = ReqB::from(RequestBody::from(body));
                    let req = Request::from_parts(parts, body);

                    queue.push(async move {
                        let fut = flow.service.call(req);
                        h2_handler(fut, tx, date).await
                    });
                    queued = true;
                }
                SelectOutput::A(SelectOutput::B(Some(res))) => {
                    if let Err(e) = res {
                        HttpServiceError::from(e).log("h2_dispatcher");
                    }
                }
                SelectOutput::A(SelectOutput::B(None)) => queued = false,
                SelectOutput::B(Ok(_)) => {
                    trace!("Connection keep-alive timeout. Shutting down");
                    return Ok(());
                }
                SelectOutput::A(SelectOutput::A(None)) => {
                    trace!("Connection closed by remote. Shutting down");
                    break;
                }
                SelectOutput::A(SelectOutput::A(Some(Err(e)))) | SelectOutput::B(Err(e)) => return Err(From::from(e)),
            }
        }

        while queue.next().await.is_some() {}

        io.graceful_shutdown();

        poll_fn(|cx| io.poll_closed(cx)).await.map_err(From::from)
    }
}

async fn try_queue<F: Future>(queued: bool, queue: &mut FuturesUnordered<F>) -> Option<F::Output> {
    if queued {
        queue.next().await
    } else {
        never().await
    }
}

struct H2PingPong<'a> {
    on_flight: bool,
    keep_alive: Pin<&'a mut KeepAlive>,
    ping_pong: PingPong,
    date: &'a DateTimeHandle,
    ka_dur: Duration,
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

                // Update the keep alive to 10 times the normal keep alive duration.
                // There is no particular reason for the duration choice here. as h2 connection is
                // suggested to be kept alive for a relative long time.
                let deadline = this.date.now() + (this.ka_dur * 10);

                this.keep_alive.as_mut().update(deadline);

                this.on_flight = true;
            }
        }
    }
}

async fn h2_handler<'f, 'd, Fut, B, BE, E>(
    fut: Fut,
    mut tx: SendResponse<Bytes>,
    date: &'d DateTimeHandle,
) -> Result<(), Error<E>>
where
    Fut: Future<Output = Result<Response<ResponseBody<B>>, E>> + 'f,
    B: Stream<Item = Result<Bytes, BE>>,
    BodyError: From<BE>,
{
    // split response to header and body.
    let (res, body) = fut.await.map_err(Error::Service)?.into_parts();
    let mut res = Response::from_parts(res, ());

    // set response version.
    *res.version_mut() = Version::HTTP_2;

    // check eof state of response body and make sure header is valid.
    let is_eof = match body.size() {
        ResponseBodySize::None => {
            debug_assert!(!res.headers().contains_key(CONTENT_LENGTH));
            true
        }
        ResponseBodySize::Stream => false,
        ResponseBodySize::Sized(n) => {
            // add an content-length header if there is non provided.
            if !res.headers().contains_key(CONTENT_LENGTH) {
                res.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from(n));
            }
            n == 0
        }
    };

    if !res.headers().contains_key(DATE) {
        let date = date.with_date(HeaderValue::from_bytes).unwrap();
        res.headers_mut().insert(DATE, date);
    }

    // send response and body(if there is one).
    let mut stream = tx.send_response(res, is_eof)?;

    if !is_eof {
        pin!(body);

        while let Some(res) = body.as_mut().next().await {
            let mut chunk = res?;

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

        stream.send_data(Bytes::new(), true)?;
    }

    Ok(())
}

const CHUNK_SIZE: usize = 16_384;
