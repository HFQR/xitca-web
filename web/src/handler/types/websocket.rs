use core::{
    cmp::Ordering,
    convert::Infallible,
    future::{Future, poll_fn},
    pin::{Pin, pin},
    time::Duration,
};

use std::io;

use futures_core::stream::Stream;
use http_ws::{
    HandshakeError, Item, Message as WsMessage, ProtocolError, WsOutput,
    stream::{RequestStream, WsError},
};
use tokio::time::{Instant, sleep};
use xitca_unsafe_collection::{
    bytes::BytesStr,
    futures::{Select, SelectOutput},
};

use crate::{
    body::{BodyStream, RequestBody, ResponseBody},
    bytes::Bytes,
    context::WebContext,
    error::{Error, HeaderNotFound},
    handler::{FromRequest, Responder},
    http::{
        StatusCode, WebResponse,
        header::{CONNECTION, SEC_WEBSOCKET_VERSION, UPGRADE},
    },
    service::Service,
};

pub use http_ws::{ResponseSender, ResponseWeakSender};

/// simplified websocket message type.
/// for more variant of message please reference [http_ws::Message] type.
#[derive(Debug, Eq, PartialEq)]
pub enum Message {
    Text(BytesStr),
    Binary(Bytes),
    Continuation(Item),
}

type BoxFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

type OnMsgCB = Box<dyn for<'a> FnMut(&'a mut ResponseSender, Message) -> BoxFuture<'a>>;

type OnErrCB<E> = Box<dyn FnMut(WsError<E>) -> BoxFuture<'static>>;

type OnCloseCB<B> = Box<dyn for<'a> FnOnce(Pin<&'a mut RequestStream<B>>) -> BoxFuture<'a>>;

pub struct WebSocket<B = RequestBody>
where
    B: BodyStream,
{
    ws: WsOutput<B>,
    ping_interval: Duration,
    max_unanswered_ping: u8,
    on_msg: OnMsgCB,
    on_err: OnErrCB<B::Error>,
    on_close: OnCloseCB<B>,
}

impl<B> WebSocket<B>
where
    B: BodyStream,
{
    fn new(ws: WsOutput<B>) -> Self {
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
            on_close: Box::new(|_| boxed_future()),
        }
    }

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

    /// Get a reference of Websocket message sender.
    /// Can be used to send message to client.
    pub fn msg_sender(&self) -> &ResponseSender {
        &self.ws.2
    }

    /// Async function that would be called when new message arrived from client.
    pub fn on_msg<F>(&mut self, func: F) -> &mut Self
    where
        F: for<'a> FnMut(&'a mut ResponseSender, Message) -> BoxFuture<'a> + 'static,
    {
        self.on_msg = Box::new(func);
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
        F: FnOnce(Pin<&mut RequestStream<B>>) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        self.on_close = Box::new(|stream| Box::pin(func(stream)));
        self
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for HandshakeError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let e = match self {
            HandshakeError::NoConnectionUpgrade => HeaderNotFound(CONNECTION),
            HandshakeError::NoVersionHeader => HeaderNotFound(SEC_WEBSOCKET_VERSION),
            HandshakeError::NoWebsocketUpgrade => HeaderNotFound(UPGRADE),
            // TODO: refine error mapping of the remaining branches.
            _ => return StatusCode::INTERNAL_SERVER_ERROR.call(ctx).await,
        };

        e.call(ctx).await
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for WebSocket<B>
where
    C: 'static,
    B: BodyStream + Default + 'static,
{
    type Type<'b> = WebSocket<B>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let body = ctx.take_body_ref();
        let ws = http_ws::ws(ctx.req(), body).map_err(Error::from_service)?;
        Ok(WebSocket::new(ws))
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for WebSocket<B>
where
    B: BodyStream + 'static,
{
    type Response = WebResponse;
    type Error = Infallible;

    async fn respond(self, _: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
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

        Ok(res.map(ResponseBody::box_stream))
    }
}

async fn spawn_task<B>(
    ping_interval: Duration,
    max_unanswered_ping: u8,
    decode: RequestStream<B>,
    mut tx: ResponseSender,
    mut on_msg: OnMsgCB,
    mut on_err: OnErrCB<B::Error>,
    on_close: OnCloseCB<B>,
) where
    B: BodyStream,
{
    let on_msg = &mut *on_msg;
    let on_err = &mut *on_err;

    let mut decode = pin!(decode);

    let spawn_inner = async {
        let mut sleep = pin!(sleep(ping_interval));

        let mut un_answered_ping = 0u8;

        loop {
            match poll_fn(|cx| decode.as_mut().poll_next(cx)).select(sleep.as_mut()).await {
                SelectOutput::A(Some(Ok(msg))) => {
                    let msg = match msg {
                        WsMessage::Text(txt) => Message::Text(BytesStr::try_from(txt).unwrap()),
                        WsMessage::Binary(bin) => Message::Binary(bin),
                        WsMessage::Continuation(item) => Message::Continuation(item),
                        WsMessage::Nop => continue,
                        WsMessage::Pong(_) => {
                            if let Some(num) = un_answered_ping.checked_sub(1) {
                                un_answered_ping = num;
                            }
                            continue;
                        }
                        WsMessage::Ping(ping) => {
                            tx.send(WsMessage::Pong(ping)).await?;
                            continue;
                        }
                        WsMessage::Close(reason) => {
                            match tx.send(WsMessage::Close(reason)).await {
                                // ProtocolError::Closed error means someone already sent close message
                                // so just ignore it and treat as success.
                                Ok(_) | Err(ProtocolError::Closed) => return Ok(()),
                                Err(e) => return Err(e.into()),
                            }
                        }
                    };

                    on_msg(&mut tx, msg).await
                }
                SelectOutput::A(Some(Err(e))) => on_err(e).await,
                SelectOutput::A(None) => return Ok(()),
                SelectOutput::B(_) => match un_answered_ping.cmp(&max_unanswered_ping) {
                    Ordering::Less => {
                        if let Err(e) = tx.send(WsMessage::Ping(Bytes::new())).await {
                            // continue ping timer when websocket is closed.
                            // client may be lagging behind and not respond to close message immediately.
                            if !matches!(e, ProtocolError::Closed) {
                                return Err(e.into());
                            }
                        }
                        un_answered_ping += 1;
                        sleep.as_mut().reset(Instant::now() + ping_interval);
                    }
                    // on last interval try to send close message to client to inform it connection
                    // is going away.
                    Ordering::Equal => match tx.send(WsMessage::Close(None)).await {
                        Ok(_) => un_answered_ping += 1,
                        // ProtocolError::Closed error means someone already sent close message
                        // so just ignore it and end connection right away.
                        Err(ProtocolError::Closed) => return Ok(()),
                        Err(e) => return Err(e.into()),
                    },
                    // this will only happen when client fail to respond to the close message on last
                    // interval in time and at this point just closed the connection with an io error.
                    Ordering::Greater => {
                        let _ = tx.send_error(io::ErrorKind::UnexpectedEof.into()).await;
                        return Ok(());
                    }
                },
            }
        }
    };

    if let Err(e) = spawn_inner.await {
        on_err(e).await;
    }

    on_close(decode).await;
}
