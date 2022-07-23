use std::{
    pin::Pin,
    task::{Context, Poll},
};

use xitca_http::{bytes::Bytes, error::BodyError, h2::body::RequestBody};

use futures_core::stream::Stream;

#[allow(dead_code)]
/// Though the naming is ResponseBody. It's actually a bi-directional
/// streaming type that able to add additional stream message to server.
pub struct ResponseBody {
    /// When used by client [RequestBody] is used as part of response body.
    rx: RequestBody,
    // TODO: use new type and import from xitca_http?
    tx: h2::SendStream<Bytes>,
    want_poll_cap: bool,
}

impl ResponseBody {
    pub(super) fn new(tx: h2::SendStream<Bytes>, rx: RequestBody) -> Self {
        Self {
            tx,
            rx,
            want_poll_cap: false,
        }
    }
}

#[cfg(feature = "websocket")]
mod impl_websocket {
    use super::*;

    use xitca_http::bytes::{Buf, BytesMut};

    use crate::h2::Error;

    impl ResponseBody {
        pub(crate) fn poll_send_buf(&mut self, bytes: &mut BytesMut, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
            let chunk = bytes.chunk();
            if self.want_poll_cap {
                let res = std::task::ready!(self.tx.poll_capacity(cx));
                self.want_poll_cap = false;
                let cap = res.expect("No capacity left. http2 request is dropped")?;

                let len = std::cmp::min(cap, chunk.len());
                let bytes = bytes.split_to(len).freeze();
                self.tx.send_data(bytes, false)?;

                Poll::Ready(Ok(()))
            } else {
                self.tx.reserve_capacity(chunk.len());
                self.want_poll_cap = true;
                self.poll_send_buf(bytes, cx)
            }
        }

        pub(crate) fn send_data(&mut self, bytes: Bytes, eof: bool) -> Result<(), Error> {
            self.tx.send_data(bytes, eof)?;
            Ok(())
        }
    }
}

impl Stream for ResponseBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().rx).poll_next(cx)
    }
}
