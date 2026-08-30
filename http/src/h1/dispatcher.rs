use core::{
    future::poll_fn,
    marker::PhantomData,
    net::SocketAddr,
    pin::{Pin, pin},
    task::Poll,
    time::Duration,
};

use std::net::Shutdown;

use tracing::trace;
use xitca_io::io::{AsyncBufRead, AsyncBufWrite};
use xitca_service::Service;
use xitca_unsafe_collection::futures::SelectOutput;

use crate::{
    body::{Body, Frame},
    bytes::{Bytes, BytesMut},
    config::HttpServiceConfig,
    date::DateTime,
    h1::error::Error,
    http::{StatusCode, response::Response},
    util::timer::{KeepAlive, KeepAliveOutput, Timeout},
};

use super::{
    body::{RequestBody, body},
    io::{BufIo, SharedIo},
    proto::{buf_write::H1BufWrite, context::Context, error::ProtoError},
};

type ExtRequest<B> = crate::http::Request<crate::http::RequestExt<B>>;

/// Http/1 dispatcher
pub struct Dispatcher<'a, Io, S, ReqB, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize> {
    io: SharedIo<Io>,
    timer: Timer<'a>,
    ctx: Context<'a, D, H_LIMIT>,
    service: &'a S,
    _phantom: PhantomData<ReqB>,
}

impl<'a, Io, S, ReqB, ResB, BE, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize>
    Dispatcher<'a, Io, S, ReqB, D, H_LIMIT, R_LIMIT, W_LIMIT>
where
    Io: AsyncBufRead + AsyncBufWrite + 'static,
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Body<Data = Bytes, Error = BE>,
    D: DateTime,
{
    pub async fn run(
        io: Io,
        addr: SocketAddr,
        read_buf: BytesMut,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<H_LIMIT, R_LIMIT, W_LIMIT>,
        service: &'a S,
        date: &'a D,
    ) -> Result<(), Error<S::Error, BE>> {
        let mut dispatcher = Dispatcher::<_, _, _, _, H_LIMIT, R_LIMIT, W_LIMIT> {
            io: SharedIo::new(io),
            timer: Timer::new(timer, config.keep_alive_timeout, config.request_head_timeout),
            ctx: Context::with_addr(addr, date),
            service,
            _phantom: PhantomData,
        };

        let mut read_buf = read_buf;
        let mut write_buf = BytesMut::new();

        let res = loop {
            let (res, r_buf, w_buf) = dispatcher._run(read_buf, write_buf).await;
            read_buf = r_buf;
            write_buf = w_buf;

            if let Err(err) = res {
                break Err(err);
            }

            let (res, w_buf) = write_buf.write(dispatcher.io.io()).await;
            write_buf = w_buf;

            res?;

            if dispatcher.ctx.is_connection_closed() {
                break Ok(());
            }

            dispatcher.timer.update(dispatcher.ctx.date().now());

            match read_buf.read(dispatcher.io.io()).timeout(dispatcher.timer.get()).await {
                Ok((res, r_buf)) => {
                    read_buf = r_buf;

                    if res? == 0 {
                        break Ok(());
                    }
                }
                Err(KeepAliveOutput::Cancel) => break Ok(()),
                Err(KeepAliveOutput::Expire) => break Err(dispatcher.timer.map_to_err()),
            }
        };

        if let Err(err) = res {
            handle_error(&mut dispatcher.ctx, &mut write_buf, err)?;
        }

        dispatcher.shutdown(write_buf).await
    }

    async fn _run(
        &mut self,
        mut read_buf: BytesMut,
        mut write_buf: BytesMut,
    ) -> (Result<(), Error<S::Error, BE>>, BytesMut, BytesMut) {
        loop {
            let (req, decoder) = match self.ctx.decode_head::<R_LIMIT>(&mut read_buf) {
                Ok(Some(req)) => req,
                Ok(None) => break,
                Err(e) => return (Err(e.into()), read_buf, write_buf),
            };

            self.timer.reset_state();

            let (wait_for_notify, body) = if decoder.is_eof() {
                (false, RequestBody::default())
            } else {
                let body = body(
                    self.io.notifier(),
                    self.ctx.is_expect_header(),
                    R_LIMIT,
                    decoder,
                    read_buf.split(),
                );

                (true, body)
            };

            let req = req.map(|ext| ext.map_body(|_| ReqB::from(body)));

            let (parts, body) = match self.service.call(req).await {
                Ok(res) => res.into_parts(),
                Err(e) => return (Err(Error::Service(e)), read_buf, write_buf),
            };

            let mut encoder = match self.ctx.encode_head(parts, &body, &mut write_buf) {
                Ok(encoder) => encoder,
                Err(e) => return (Err(e.into()), read_buf, write_buf),
            };

            // this block is necessary. ResB has to be dropped asap as it may hold ownership of
            // Body type which if not dropped before Notifier::notify is called would prevent
            // Notifier from waking up Notify.
            {
                let mut body = pin!(body);

                let trailers = loop {
                    let res = poll_fn(|cx| match body.as_mut().poll_frame(cx) {
                        Poll::Ready(res) => Poll::Ready(SelectOutput::A(res)),
                        Poll::Pending if write_buf.is_empty() => Poll::Pending,
                        Poll::Pending => Poll::Ready(SelectOutput::B(())),
                    })
                    .await;

                    let res = match res {
                        SelectOutput::A(Some(Ok(Frame::Data(bytes)))) => {
                            encoder.encode(bytes, &mut write_buf);
                            if write_buf.len() < W_LIMIT {
                                continue;
                            }
                            Ok(())
                        }
                        SelectOutput::A(Some(Ok(Frame::Trailers(trailers)))) => break Some(trailers),
                        SelectOutput::A(Some(Err(e))) => Err(Error::Body(e)),
                        SelectOutput::A(None) => break None,
                        SelectOutput::B(_) => Ok(()),
                    };

                    let (res_io, w_buf) = write_buf.write(self.io.io()).await;
                    write_buf = w_buf;

                    if let Some(e) = res.err().or_else(|| res_io.err().map(Error::Io)) {
                        return (Err(e), read_buf, write_buf);
                    }
                };

                encoder.encode_eof(trailers, &mut write_buf);
            }

            if wait_for_notify {
                match self.io.wait().await {
                    Some(r_buf) => read_buf = r_buf,
                    None => {
                        self.ctx.set_close();
                        break;
                    }
                }
            }
        }

        (Ok(()), read_buf, write_buf)
    }

    #[cold]
    #[inline(never)]
    async fn shutdown(self, write_buf: BytesMut) -> Result<(), Error<S::Error, BE>> {
        let io = self.io.into_io();
        let (res, _) = write_buf.write(&io).await;
        res?;
        io.shutdown(Shutdown::Both).await.map_err(Into::into)
    }
}

// timer state is transformed in following order:
//
// Idle (expecting keep-alive duration)           <--
//  |                                               |
//  --> Wait (expecting request head duration)      |
//       |                                          |
//       --> Throttle (expecting manually set to Idle again)
enum TimerState {
    Idle,
    Wait,
    Throttle,
}

struct Timer<'a> {
    timer: Pin<&'a mut KeepAlive>,
    state: TimerState,
    ka_dur: Duration,
    req_dur: Duration,
}

impl<'a> Timer<'a> {
    fn new(timer: Pin<&'a mut KeepAlive>, ka_dur: Duration, req_dur: Duration) -> Self {
        Self {
            timer,
            state: TimerState::Idle,
            ka_dur,
            req_dur,
        }
    }

    fn reset_state(&mut self) {
        self.state = TimerState::Idle;
    }

    fn get(&mut self) -> Pin<&mut KeepAlive> {
        self.timer.as_mut()
    }

    // update timer with a given base instant value. the final deadline is calculated base on it.
    fn update(&mut self, now: tokio::time::Instant) {
        let dur = match self.state {
            TimerState::Idle => {
                self.state = TimerState::Wait;
                self.ka_dur
            }
            TimerState::Wait => {
                self.state = TimerState::Throttle;
                self.req_dur
            }
            TimerState::Throttle => return,
        };
        self.timer.as_mut().update(now + dur)
    }

    #[cold]
    #[inline(never)]
    fn map_to_err<SE, BE>(&self) -> Error<SE, BE> {
        match self.state {
            TimerState::Wait => Error::KeepAliveExpire,
            TimerState::Throttle => Error::RequestTimeout,
            TimerState::Idle => unreachable!(),
        }
    }
}

#[cold]
#[inline(never)]
fn handle_error<D, W, S, B, const H_LIMIT: usize>(
    ctx: &mut Context<'_, D, H_LIMIT>,
    buf: &mut W,
    err: Error<S, B>,
) -> Result<(), Error<S, B>>
where
    D: DateTime,
    W: H1BufWrite,
{
    ctx.set_close();
    match err {
        Error::KeepAliveExpire => {
            trace!(target: "h1_dispatcher", "Connection keep-alive expired. Shutting down")
        }
        e => {
            let status = match e {
                Error::RequestTimeout => StatusCode::REQUEST_TIMEOUT,
                Error::Proto(ProtoError::HeaderTooLarge) => StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                Error::Proto(_) => StatusCode::BAD_REQUEST,
                e => return Err(e),
            };
            let (parts, body) = Response::builder()
                .status(status)
                .body(crate::body::Empty::<Bytes>::new())
                .unwrap()
                .into_parts();
            ctx.encode_head(parts, &body, buf)
                .expect("request_error must be correct");
        }
    }
    Ok(())
}
