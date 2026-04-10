use std::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use ::h3::{client::RequestStream, quic::RecvStream};

use crate::{
    body::{Body, BodyError, Frame},
    bytes::{Buf, Bytes},
};

pub type ResponseBody = Pin<Box<dyn Body<Data = Bytes, Error = BodyError> + Send + Sync + 'static>>;

pub(super) struct MapBody<S, B>
where
    S: RecvStream,
{
    pub(super) body: RequestStream<S, B>,
}

impl<S, B> Body for MapBody<S, B>
where
    S: RecvStream,
{
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let opt = match ready!(this.body.poll_recv_data(cx)) {
            Ok(Some(bytes)) => Some(Ok(Frame::Data(Bytes::copy_from_slice(bytes.chunk())))),
            Err(e) => Some(Err(e.into())),
            Ok(None) => match ready!(this.body.poll_recv_trailers(cx)) {
                Ok(Some(trailers)) => Some(Ok(Frame::Trailers(trailers))),
                Err(e) => Some(Err(e.into())),
                Ok(None) => None,
            },
        };
        Poll::Ready(opt)
    }
}
