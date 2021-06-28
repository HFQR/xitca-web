use std::{
    cmp,
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
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::{ready, Stream};
use http::{
    header::{CONTENT_LENGTH, DATE},
    HeaderValue, Request, Response, Version,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    pin, select,
};
use tracing::trace;

use crate::body::{ResponseBody, ResponseBodySize};
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlow;
use crate::h2::{body::RequestBody, error::Error};
use crate::response::ResponseError;
use crate::util::{
    date::{Date, SharedDate},
    futures::poll_fn,
    keep_alive::KeepAlive,
};

/// Http/2 dispatcher
pub(crate) struct Dispatcher<'a, TlsSt, S, ReqB, X, U> {
    io: &'a mut Connection<TlsSt, Bytes>,
    keep_alive: Pin<&'a mut KeepAlive>,
    ka_dur: Duration,
    flow: &'a HttpFlow<S, X, U>,
    date: &'a SharedDate,
    _req_body: PhantomData<ReqB>,
}

impl<'a, TlsSt, S, ReqB, X, U, B, E> Dispatcher<'a, TlsSt, S, ReqB, X, U>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: 'static,

    U: 'static,

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
        flow: &'a HttpFlow<S, X, U>,
        date: &'a SharedDate,
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

    pub(crate) async fn run(self) -> Result<(), Error> {
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
        let deadline = date.borrow().now() + ka_dur;
        keep_alive.as_mut().update(deadline);

        // timer for ping pong interval and keep alive.
        let mut ping_pong = H2PingPong {
            on_flight: false,
            keep_alive: keep_alive.as_mut(),
            ping_pong,
            date: &*date,
            ka_dur,
        };

        loop {
            select! {
                biased;
                opt = io.accept() => match opt {
                    Some(res) => {
                        let (req, tx) = res?;
                        // Convert http::Request body type to crate::h2::Body
                        // and reconstruct as HttpRequest.
                        let (parts, body) = req.into_parts();
                        let body = ReqB::from(RequestBody::from(body));
                        let req = Request::from_parts(parts, body);

                        let flow = HttpFlow::clone(flow);
                        let date = SharedDate::clone(self.date);

                        tokio::task::spawn_local(async move {
                            let fut = flow.service.call(req);
                            if let Err(e) = h2_handler(fut, tx, date).await {
                                HttpServiceError::from(e).log("h2_dispatcher");
                            }
                        });
                    },
                    None => return Ok(())
                },
                res = &mut ping_pong => {
                    res?;

                    trace!("Connection keep-alive timeout. Shutting down");

                    io.graceful_shutdown();

                    poll_fn(|cx| io.poll_closed(cx)).await?;

                    return Ok(())
                }
            }
        }
    }
}

struct H2PingPong<'a> {
    on_flight: bool,
    keep_alive: Pin<&'a mut KeepAlive>,
    ping_pong: PingPong,
    date: &'a Date,
    ka_dur: Duration,
}

impl Future for H2PingPong<'_> {
    type Output = Result<(), Error>;

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

                        let deadline = this.date.borrow().now() + this.ka_dur;

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
                let deadline = this.date.borrow().now() + (this.ka_dur * 10);

                this.keep_alive.as_mut().update(deadline);

                this.on_flight = true;
            }
        }
    }
}

async fn h2_handler<Fut, B, BE, E>(fut: Fut, mut tx: SendResponse<Bytes>, date: SharedDate) -> Result<(), Error>
where
    Fut: Future<Output = Result<Response<ResponseBody<B>>, E>>,
    E: ResponseError<Response<ResponseBody<B>>>,
    B: Stream<Item = Result<Bytes, BE>>,
    BodyError: From<BE>,
{
    // resolve service call. map error to response.
    let res = fut.await.unwrap_or_else(|ref mut e| ResponseError::response_error(e));

    // split response to header and body.
    let (res, body) = res.into_parts();
    let mut res = Response::from_parts(res, ());

    // set response version.
    *res.version_mut() = Version::HTTP_2;

    // set content length header when it's absent.
    if !res.headers().contains_key(CONTENT_LENGTH) {
        if let ResponseBodySize::Sized(n) = body.size() {
            res.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from(n));
        }
    }

    if !res.headers().contains_key(DATE) {
        let date = HeaderValue::from_bytes(date.borrow().date()).unwrap();
        res.headers_mut().insert(DATE, date);
    }

    // send response and body(if there is one).
    if body.is_eof() {
        let _ = tx.send_response(res, true)?;
    } else {
        let mut stream = tx.send_response(res, false)?;

        pin!(body);

        while let Some(res) = body.as_mut().next().await {
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
    }

    Ok(())
}

const CHUNK_SIZE: usize = 16_384;
