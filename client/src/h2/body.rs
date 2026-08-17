use core::{
    cmp,
    future::Future,
    pin::Pin,
    task::{Context, Poll, ready},
};

use h2::client::ResponseFuture;

use crate::{
    body::{Body, BodyError, Frame},
    bytes::{Buf, Bytes, BytesMut},
    http::response::Parts,
};

type Tx = h2::SendStream<Bytes>;
type Rx = h2::RecvStream;

/// The receiving half, which may not exist yet.
///
/// A response head normally arrives before this body does. It does not for a
/// stream the caller is given *before* the server has answered — see
/// [`ResponseBody::new`] — so the head is resolved on the first read
/// instead.
enum Recv {
    Pending(ResponseFuture),
    Ready { rx: Rx, head: Option<Parts> },
}

#[allow(dead_code)]
/// Though the naming is ResponseBody. It's actually a bi-directional
/// streaming type that able to add additional stream message to server.
pub struct ResponseBody {
    recv: Recv,
    // TODO: use new type and import from xitca_http?
    pub(crate) tx: Tx,
    want_poll_cap: bool,
}

impl ResponseBody {
    /// A body for a request whose response head has not arrived.
    ///
    /// **Every h2 body starts here**, because at the point one is built the
    /// head genuinely does not exist yet. Whoever wants it awaits
    /// [`ResponseBody::poll_head`]; whoever wants to write first simply does
    /// not, which is what makes a client-speaks-first stream possible.
    pub(super) fn new(tx: Tx, fut: ResponseFuture) -> Self {
        Self {
            tx,
            recv: Recv::Pending(fut),
            want_poll_cap: false,
        }
    }

    /// Resolve the response head if it has not arrived yet, keeping it for
    /// [`ResponseBody::take_head`]. A no-op for a body constructed with one.
    pub(crate) fn poll_head(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), BodyError>> {
        if let Recv::Pending(ref mut fut) = self.recv {
            let res = ready!(Pin::new(fut).poll(cx))?;
            let (head, rx) = res.into_parts();
            self.recv = Recv::Ready { rx, head: Some(head) };
        }
        Poll::Ready(Ok(()))
    }

    /// Take the head resolved by [`ResponseBody::poll_head`]. `None` when it
    /// has already been taken.
    ///
    /// Taken once, by whoever awaited it.
    pub(crate) fn take_head(&mut self) -> Option<Parts> {
        match self.recv {
            Recv::Pending(_) => None,
            Recv::Ready { ref mut head, .. } => head.take(),
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

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        let this = self.get_mut();

        // There is nothing to read from until the response head has arrived.
        if let Err(e) = ready!(this.poll_head(cx)) {
            return Poll::Ready(Some(Err(e)));
        }

        let Recv::Ready { ref mut rx, .. } = this.recv else {
            unreachable!("poll_head leaves the receiving half ready or errors")
        };

        // Try to read data frames first.
        match rx.poll_data(cx) {
            Poll::Ready(Some(Ok(data))) => {
                // TODO: less aggressive window update
                rx.flow_control().release_capacity(data.len())?;
                return Poll::Ready(Some(Ok(Frame::Data(data))));
            }
            Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e.into()))),
            Poll::Ready(None) => {}
            Poll::Pending => return Poll::Pending,
        }

        // Data stream is exhausted — try to read trailers.
        match rx.poll_trailers(cx) {
            Poll::Ready(Ok(Some(trailers))) => Poll::Ready(Some(Ok(Frame::Trailers(trailers)))),
            Poll::Ready(Ok(None)) => Poll::Ready(None),
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e.into()))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        match self.recv {
            // A head that has not arrived cannot be followed by an end of
            // stream that has.
            Recv::Pending(_) => false,
            Recv::Ready { ref rx, .. } => rx.is_end_stream(),
        }
    }
}
