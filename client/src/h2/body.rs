use core::{
    cmp,
    pin::Pin,
    task::{ready, Context, Poll},
};

use xitca_http::h2::body::RequestBody;

use futures_core::stream::Stream;

use crate::{
    body::BodyError,
    bytes::{Buf, Bytes, BytesMut},
};

type Tx = h2::SendStream<Bytes>;

#[allow(dead_code)]
/// Though the naming is ResponseBody. It's actually a bi-directional
/// streaming type that able to add additional stream message to server.
pub struct ResponseBody {
    /// When used by client [RequestBody] is used as part of response body.
    rx: RequestBody,
    // TODO: use new type and import from xitca_http?
    pub(crate) tx: Tx,
    want_poll_cap: bool,
}

impl ResponseBody {
    pub(super) fn new(tx: Tx, rx: RequestBody) -> Self {
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

impl Stream for ResponseBody {
    type Item = Result<Bytes, BodyError>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().rx).poll_next(cx)
    }
}
