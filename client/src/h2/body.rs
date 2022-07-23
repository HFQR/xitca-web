use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use xitca_http::{bytes::Bytes, error::BodyError, h2::body::RequestBody};

use futures_core::stream::Stream;

use super::error::Error;

/// Though the naming is ResponseBody. It's actually a bi-directional
/// streaming type that able to add additional stream message to server.
pub struct ResponseBody {
    // TODO: use new type and import from xitca_http?
    #[allow(dead_code)]
    pub(crate) tx: h2::SendStream<Bytes>,
    /// When used by client [RequestBody] is used as part of response body.
    pub(crate) rx: RequestBody,
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

    pub(crate) fn poll_capacity(&mut self, cap: usize, cx: &mut Context<'_>) -> Poll<Result<usize, Error>> {
        if self.want_poll_cap {
            let res = ready!(self.tx.poll_capacity(cx));
            self.want_poll_cap = false;
            let cap = res.expect("No capacity left. http2 request is dropped")?;
            Poll::Ready(Ok(cap))
        } else {
            self.tx.reserve_capacity(cap);
            self.want_poll_cap = true;
            self.poll_capacity(cap, cx)
        }
    }

    pub(crate) fn send_data(&mut self, bytes: Bytes, eof: bool) -> Result<(), Error> {
        self.tx.send_data(bytes, eof)?;
        Ok(())
    }
}

impl Stream for ResponseBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().rx).poll_next(cx)
    }
}
