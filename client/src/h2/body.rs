use core::{
    cmp,
    pin::Pin,
    task::{Context, Poll, ready},
};

use xitca_http::body::{Body, Frame, SizeHint};

use crate::{
    body::BodyError,
    bytes::{Buf, Bytes, BytesMut},
};

type Tx = h2::SendStream<Bytes>;
type Rx = h2::RecvStream;

#[allow(dead_code)]
/// Though the naming is ResponseBody. It's actually a bi-directional
/// streaming type that able to add additional stream message to server.
pub struct ResponseBody {
    rx: Rx,
    // TODO: use new type and import from xitca_http?
    pub(crate) tx: Tx,
    want_poll_cap: bool,
}

impl ResponseBody {
    pub(super) fn new(tx: Tx, rx: Rx) -> Self {
        Self {
            tx,
            rx,
            want_poll_cap: false,
        }
    }

    pub(crate) fn poll_send_buf(
        &mut self,
        bytes: &mut BytesMut,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), crate::h2::Error>> {
        let chunk = bytes.chunk();
        if self.want_poll_cap {
            let res = ready!(self.tx.poll_capacity(cx));
            self.want_poll_cap = false;
            let cap = res.expect("No capacity left. http2 request is dropped")?;

            let len = cmp::min(cap, chunk.len());
            let bytes = bytes.split_to(len).freeze();
            self.tx.send_data(bytes, false)?;

            Poll::Ready(Ok(()))
        } else {
            self.tx.reserve_capacity(chunk.len());
            self.want_poll_cap = true;
            self.poll_send_buf(bytes, cx)
        }
    }

    pub(crate) fn send_data(&mut self, bytes: Bytes, eof: bool) -> Result<(), crate::h2::Error> {
        self.tx.send_data(bytes, eof).map_err(Into::into)
    }
}

impl Body for ResponseBody {
    type Data = Bytes;
    type Error = BodyError;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        self.get_mut()
            .rx
            .poll_data(cx)
            .map_ok(Frame::Data)
            .map_err(|e| e.into())
    }

    fn is_end_stream(&self) -> bool {
        self.rx.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}
