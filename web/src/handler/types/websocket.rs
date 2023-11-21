//! type extractor and responder for websocket

use core::{
    future::{poll_fn, Future},
    pin::{pin, Pin},
    time::Duration,
};

use futures_core::stream::Stream;
use http_ws::{
    stream::{RequestStream, ResponseSender, WsError},
    HandshakeError, Item, Message as WsMessage, ProtocolError, WsOutput,
};
use tokio::time::{sleep, Instant};
use xitca_unsafe_collection::{
    bytes::BytesStr,
    futures::{Select, SelectOutput},
};

use crate::{
    body::{BodyStream, RequestBody, ResponseBody},
    bytes::Bytes,
    handler::{error::ExtractError, FromRequest, Responder},
    http::header::{CONNECTION, SEC_WEBSOCKET_VERSION, UPGRADE},
    request::WebRequest,
    response::WebResponse,
};

/// simplified websocket message type.
/// for more variant of message please reference [WsMessage] type.
#[derive(Debug, Eq, PartialEq)]
pub enum Message {
    Text(BytesStr),
    Binary(Bytes),
    Continuation(Item),
}

type BoxFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

type OnMsgCB = Box<dyn for<'a> FnMut(Message, &'a mut Context) -> BoxFuture<'a>>;

type OnErrCB<E> = Box<dyn FnMut(WsError<E>) -> BoxFuture<'static>>;

type OnCloseCB = Box<dyn FnOnce() -> BoxFuture<'static>>;

/// type extractor and responder for WebSocket.
///
/// upon extraction the WebSocket type is supposed to be returned as response type
/// after possible configuration.
///
/// dropping the websocket after extraction and return other response type(error)
/// is allowed.
pub struct WebSocket<B = RequestBody>
where
    B: BodyStream,
{
    ws: WsOutput<B, B::Error>,
    ping_interval: Duration,
    max_unanswered_ping: u8,
    on_msg: OnMsgCB,
    on_err: OnErrCB<B::Error>,
    on_close: OnCloseCB,
}

impl<B> WebSocket<B>
where
    B: BodyStream,
{
    /// Set interval duration of server side ping message to client.
    pub fn set_ping_interval(&mut self, dur: Duration) -> &mut Self {
        self.ping_interval = dur;
        self
    }

    /// Set max number of consecutive server side ping messages that are not
    /// answered by client.
    ///
    /// # Panic:
    /// when 0 is passed as argument.
    pub fn set_max_unanswered_ping(&mut self, size: u8) -> &mut Self {
        assert!(size > 0, "max_unanswered_ping MUST be none 0");
        self.max_unanswered_ping = size;
        self
    }

    /// Get a reference of sender part of a websocket connection.
    /// Used for sending message to client.
    pub fn msg_sender(&self) -> &ResponseSender {
        &self.ws.2
    }

    /// Async function that would be called when new message arrived from client.
    pub fn on_msg<F>(&mut self, mut func: F) -> &mut Self
    where
        F: for<'a> MsgFn<(Message, &'a mut Context)> + 'static,
    {
        self.on_msg = Box::new(move |msg, ctx| Box::pin(func.call_mut((msg, ctx))));
        self
    }

    /// Async function that would be called when error occurred.
    pub fn on_err<F, Fut>(&mut self, mut func: F) -> &mut Self
    where
        F: FnMut(WsError<B::Error>) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        self.on_err = Box::new(move |e| Box::pin(func(e)));
        self
    }

    /// Async function that would be called when closing the websocket connection.
    pub fn on_close<F, Fut>(&mut self, func: F) -> &mut Self
    where
        F: FnOnce() -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        self.on_close = Box::new(|| Box::pin(func()));
        self
    }

    fn new(ws: WsOutput<B, B::Error>) -> Self {
        #[cold]
        #[inline(never)]
        fn boxed_future() -> BoxFuture<'static> {
            Box::pin(async {})
        }

        Self {
            ws,
            ping_interval: Duration::from_secs(15),
            max_unanswered_ping: 3,
            on_msg: Box::new(|_, _| boxed_future()),
            on_err: Box::new(|_| boxed_future()),
            on_close: Box::new(|| boxed_future()),
        }
    }
}

/// websocket connection context.
/// used for interacting with connection internal states.
pub struct Context {
    tx: ResponseSender,
}

impl Context {
    /// send [WsMessage] to client.
    #[inline]
    pub fn send(&self, msg: WsMessage) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.tx.send(msg)
    }

    /// send text message to client.
    #[inline]
    pub fn text(&self, txt: impl Into<String>) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.send(WsMessage::Text(Bytes::from(txt.into())))
    }

    /// send binary message to client.
    #[inline]
    pub fn binary(&self, bin: impl Into<Bytes>) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.send(WsMessage::Binary(bin.into()))
    }
}

impl<E> From<HandshakeError> for ExtractError<E> {
    fn from(e: HandshakeError) -> Self {
        match e {
            HandshakeError::NoConnectionUpgrade => ExtractError::HeaderNotFound(CONNECTION),
            HandshakeError::NoVersionHeader => ExtractError::HeaderNotFound(SEC_WEBSOCKET_VERSION),
            HandshakeError::NoWebsocketUpgrade => ExtractError::HeaderNotFound(UPGRADE),
            // TODO: refine error mapping of the remaining branches.
            e => ExtractError::Boxed(Box::new(e)),
        }
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for WebSocket<B>
where
    C: 'static,
    B: BodyStream + Default + 'static,
{
    type Type<'b> = WebSocket<B>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebRequest<'r, C, B>) -> Result<Self, Self::Error> {
        let body = req.take_body_ref();
        let ws = http_ws::ws(req.req(), body)?;
        Ok(WebSocket::new(ws))
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for WebSocket<B>
where
    B: BodyStream + 'static,
{
    type Output = WebResponse;

    async fn respond_to(self, _: WebRequest<'r, C, B>) -> Self::Output {
        let Self {
            ws,
            ping_interval,
            max_unanswered_ping,
            on_msg,
            on_err,
            on_close,
        } = self;

        let (decode, res, tx) = ws;

        tokio::task::spawn_local(spawn_task(
            ping_interval,
            max_unanswered_ping,
            decode,
            tx,
            on_msg,
            on_err,
            on_close,
        ));

        res.map(ResponseBody::box_stream)
    }
}

async fn spawn_task<B>(
    ping_interval: Duration,
    max_unanswered_ping: u8,
    decode: RequestStream<B, B::Error>,
    tx: ResponseSender,
    mut on_msg: OnMsgCB,
    mut on_err: OnErrCB<B::Error>,
    on_close: OnCloseCB,
) where
    B: BodyStream,
{
    let on_msg = &mut *on_msg;
    let on_err = &mut *on_err;

    let spawn_inner = || async {
        let mut sleep = pin!(sleep(ping_interval));
        let mut decode = pin!(decode);

        let mut un_answered_ping = 0u8;

        let mut ctx = Context { tx };

        loop {
            match poll_fn(|cx| decode.as_mut().poll_next(cx)).select(sleep.as_mut()).await {
                SelectOutput::A(Some(Ok(msg))) => {
                    let msg = match msg {
                        WsMessage::Pong(_) => {
                            if let Some(num) = un_answered_ping.checked_sub(1) {
                                un_answered_ping = num;
                            }
                            continue;
                        }
                        WsMessage::Ping(ping) => {
                            ctx.tx.send(WsMessage::Pong(ping)).await?;
                            continue;
                        }
                        WsMessage::Close(reason) => {
                            ctx.tx.send(WsMessage::Close(reason)).await?;
                            break;
                        }
                        WsMessage::Text(txt) => Message::Text(BytesStr::try_from(txt).unwrap()),
                        WsMessage::Binary(bin) => Message::Binary(bin),
                        WsMessage::Continuation(item) => Message::Continuation(item),
                        WsMessage::Nop => continue,
                    };

                    on_msg(msg, &mut ctx).await
                }
                SelectOutput::A(Some(Err(e))) => on_err(e).await,
                SelectOutput::A(None) => break,
                SelectOutput::B(_) => {
                    if un_answered_ping > max_unanswered_ping {
                        ctx.tx.send(WsMessage::Close(None)).await?;
                        break;
                    } else {
                        un_answered_ping += 1;
                        ctx.tx.send(WsMessage::Ping(Bytes::new())).await?;
                        sleep.as_mut().reset(Instant::now() + ping_interval);
                    }
                }
            }
        }

        Ok(())
    };

    if let Err(e) = spawn_inner().await {
        on_err(e).await;
    }

    on_close().await;
}

#[doc(hidden)]
/// implementation detail of async function as callback
pub trait MsgFn<Arg> {
    type Future: Future<Output = ()>;

    fn call_mut(&mut self, arg: Arg) -> Self::Future;
}

impl<Arg1, Arg2, F, Fut> MsgFn<(Arg1, Arg2)> for F
where
    F: FnMut(Arg1, Arg2) -> Fut,
    Fut: Future<Output = ()>,
{
    type Future = Fut;

    fn call_mut(&mut self, (arg1, arg2): (Arg1, Arg2)) -> Self::Future {
        self(arg1, arg2)
    }
}
