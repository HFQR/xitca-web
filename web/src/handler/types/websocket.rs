use std::{
    future::{poll_fn, Future},
    pin::Pin,
    time::Duration,
};

pub use http_ws::Message;

use futures_core::stream::Stream;
use http_ws::{stream::DecodeStream, WsOutput};
use tokio::{sync::mpsc::Sender, time::Instant};
use xitca_unsafe_collection::{
    futures::{Select, SelectOutput},
    pin,
};

use crate::{
    dev::bytes::Bytes,
    handler::{error::ExtractError, FromRequest, Responder},
    request::{RequestBody, WebRequest},
    response::{StreamBody, WebResponse},
    stream::WebStream,
};

pub type WsSender = Sender<Message>;

type OnMsgCB = Box<dyn for<'a> FnMut(&'a mut WsSender, Message) -> Pin<Box<dyn Future<Output = ()> + 'a>>>;

pub struct WebSocket<B = RequestBody>
where
    B: WebStream,
{
    ws: WsOutput<B, B::Error>,
    ping_interval: Duration,
    max_unanswered_ping: u8,
    on_msg: OnMsgCB,
}

impl<B> WebSocket<B>
where
    B: WebStream,
{
    fn new(ws: WsOutput<B, B::Error>) -> Self {
        Self {
            ws,
            ping_interval: Duration::from_secs(15),
            max_unanswered_ping: 3,
            on_msg: Box::new(|_, _| Box::pin(async {})),
        }
    }

    pub fn set_ping_interval(&mut self, dur: Duration) -> &mut Self {
        self.ping_interval = dur;
        self
    }

    pub fn set_max_unanswered_ping(&mut self, size: u8) -> &mut Self {
        assert!(size > 0, "max_unanswered_ping MUST be none 0");
        self.max_unanswered_ping = size;
        self
    }

    pub fn msg_sender(&self) -> &Sender<Message> {
        &self.ws.2
    }

    pub fn on_msg<F>(&mut self, func: F) -> &mut Self
    where
        F: for<'a> FnMut(&'a mut WsSender, Message) -> Pin<Box<dyn Future<Output = ()> + 'a>> + 'static,
    {
        self.on_msg = Box::new(func);
        self
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for WebSocket<B>
where
    C: 'static,
    B: WebStream + Default + 'static,
{
    type Type<'b> = WebSocket<B>;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        let body = req.take_body_ref();

        async {
            let ws = http_ws::ws(req.req(), body).unwrap();
            Ok(WebSocket::new(ws))
        }
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for WebSocket<B>
where
    B: WebStream + 'static,
{
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, _: WebRequest<'r, C, B>) -> Self::Future {
        let Self {
            ws,
            ping_interval,
            max_unanswered_ping,
            on_msg,
        } = self;

        let (decode, res, tx) = ws;

        tokio::task::spawn_local(async move {
            let _ = spawn_task(ping_interval, max_unanswered_ping, decode, tx, on_msg).await;
        });

        let res = res.map(|body| StreamBody::new(body).into());

        async { res }
    }
}

async fn spawn_task<B>(
    ping_interval: Duration,
    max_unanswered_ping: u8,
    decode: DecodeStream<B, B::Error>,
    mut tx: WsSender,
    mut on_msg: OnMsgCB,
) -> Result<(), Box<dyn std::error::Error>>
where
    B: WebStream,
{
    let sleep = tokio::time::sleep(ping_interval);

    pin!(sleep);
    pin!(decode);

    let mut un_answered_ping = 0u8;

    let on_msg = &mut *on_msg;

    loop {
        match poll_fn(|cx| decode.as_mut().poll_next(cx)).select(sleep.as_mut()).await {
            SelectOutput::A(Some(Ok(msg))) => match msg {
                Message::Pong(_) => {
                    if let Some(num) = un_answered_ping.checked_sub(1) {
                        un_answered_ping = num;
                    }
                }
                Message::Ping(ping) => {
                    tx.send(Message::Pong(ping)).await?;
                }
                Message::Close(reason) => {
                    tx.send(Message::Close(reason)).await?;
                    break;
                }
                msg => on_msg(&mut tx, msg).await,
            },
            SelectOutput::A(Some(Err(_))) => break,
            SelectOutput::A(None) => break,
            SelectOutput::B(_) => {
                if un_answered_ping > max_unanswered_ping {
                    tx.send(Message::Close(None)).await?;
                    break;
                } else {
                    un_answered_ping += 1;
                    tx.send(Message::Ping(Bytes::new())).await?;
                    sleep.as_mut().reset(Instant::now() + ping_interval);
                }
            }
        }
    }

    Ok(())
}
