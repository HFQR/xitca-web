use core::{
    pin::Pin,
    task::{Context, Poll, ready},
};
use std::io;

use quinn::RecvStream;

use crate::{
    body::{Body, Frame, SizeHint},
    bytes::{Buf, Bytes, BytesMut},
    error::BodyError,
};

use super::proto::frame::{Frame as WireFrame, FrameError, PayloadLen};

/// Request body type for the homebrew Http/3 dispatcher (v2).
///
/// Wraps a bidirectional QUIC stream's receive half and decodes HTTP/3 DATA
/// frames off the wire. Trailer (HEADERS) frames after the initial request
/// headers are not yet supported — they're treated as end-of-body.
pub struct RequestBodyV2 {
    inner: Inner,
}

enum Inner {
    Active {
        recv: RecvStream,
        buf: BytesMut,
        data_remaining: usize,
    },
    Ended,
}

impl RequestBodyV2 {
    pub(crate) fn new(recv: RecvStream, leftover: BytesMut) -> Self {
        Self {
            inner: Inner::Active {
                recv,
                buf: leftover,
                data_remaining: 0,
            },
        }
    }
}

impl Body for RequestBodyV2 {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        let this = self.get_mut();
        loop {
            let Inner::Active {
                recv,
                buf,
                data_remaining,
            } = &mut this.inner
            else {
                return Poll::Ready(None);
            };

            // Still inside a DATA frame — drain buffered bytes first.
            if *data_remaining > 0 {
                if !buf.is_empty() {
                    let take = (*data_remaining).min(buf.len());
                    let bytes = buf.split_to(take).freeze();
                    *data_remaining -= take;
                    return Poll::Ready(Some(Ok(Frame::Data(bytes))));
                }
                match ready!(poll_fill(cx, recv, buf)) {
                    Ok(true) => continue,
                    Ok(false) => {
                        this.inner = Inner::Ended;
                        return Poll::Ready(Some(Err(unexpected_eof())));
                    }
                    Err(e) => {
                        this.inner = Inner::Ended;
                        return Poll::Ready(Some(Err(e)));
                    }
                }
            }

            // Between frames — try decoding the next wire frame.
            let (res, consumed) = {
                let mut cursor = io::Cursor::new(&buf[..]);
                let res = WireFrame::<PayloadLen>::decode(&mut cursor);
                (res, cursor.position() as usize)
            };

            match res {
                Ok(WireFrame::Data(len)) => {
                    buf.advance(consumed);
                    *data_remaining = len.0;
                    // loop to yield the first chunk of payload.
                }
                Ok(WireFrame::Headers(_)) => {
                    // Trailers on the request body — treat as end-of-body for now.
                    this.inner = Inner::Ended;
                    return Poll::Ready(None);
                }
                Ok(_) => {
                    // Any non-Data/Headers frame on a request stream is a protocol
                    // error per RFC 9114 §4.1. Surface as malformed body.
                    this.inner = Inner::Ended;
                    return Poll::Ready(Some(Err(invalid_data("unexpected h3 frame on request stream"))));
                }
                Err(FrameError::Incomplete(_)) => {
                    match ready!(poll_fill(cx, recv, buf)) {
                        Ok(true) => continue,
                        Ok(false) => {
                            // Clean end-of-stream between frames.
                            this.inner = Inner::Ended;
                            return Poll::Ready(None);
                        }
                        Err(e) => {
                            this.inner = Inner::Ended;
                            return Poll::Ready(Some(Err(e)));
                        }
                    }
                }
                Err(FrameError::UnknownFrame(_)) => {
                    // Frame::decode already advanced past unknown payload.
                    buf.advance(consumed);
                }
                Err(_) => {
                    this.inner = Inner::Ended;
                    return Poll::Ready(Some(Err(invalid_data("malformed h3 frame"))));
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self.inner, Inner::Ended)
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

/// Pulls a chunk from `recv` into `buf`. Returns Ok(true) if bytes were read,
/// Ok(false) on clean stream end, Err on IO error.
fn poll_fill(cx: &mut Context<'_>, recv: &mut RecvStream, buf: &mut BytesMut) -> Poll<Result<bool, BodyError>> {
    let mut tmp = [0u8; 8 * 1024];
    match recv.poll_read(cx, &mut tmp) {
        Poll::Pending => Poll::Pending,
        Poll::Ready(Ok(n)) => {
            if n == 0 {
                Poll::Ready(Ok(false))
            } else {
                buf.extend_from_slice(&tmp[..n]);
                Poll::Ready(Ok(true))
            }
        }
        Poll::Ready(Err(e)) => Poll::Ready(Err(Box::new(io::Error::new(io::ErrorKind::Other, e.to_string())))),
    }
}

fn unexpected_eof() -> BodyError {
    Box::new(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "h3 request stream ended mid-frame",
    ))
}

fn invalid_data(msg: &'static str) -> BodyError {
    Box::new(io::Error::new(io::ErrorKind::InvalidData, msg))
}
