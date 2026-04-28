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

use super::proto::{
    Header, MAX_HEADER_BLOCK_BYTES, decode_stateless,
    frame::{Frame as WireFrame, FrameError, PayloadLen},
};

/// Request body type for the homebrew Http/3 dispatcher (v2).
///
/// Wraps a bidirectional QUIC stream's receive half and decodes HTTP/3 DATA
/// and trailer (HEADERS) frames off the wire.
pub struct RequestBodyV2 {
    inner: Inner,
}

enum Inner {
    Active {
        recv: RecvStream,
        buf: BytesMut,
        data_remaining: usize,
        /// Content-Length advertised by the peer, if any. Sum of DATA frame
        /// payloads MUST equal this on clean end (RFC 9114 §4.1.2).
        expected_len: Option<u64>,
        /// Cumulative DATA payload bytes yielded so far.
        seen_len: u64,
    },
    Ended,
}

impl RequestBodyV2 {
    pub(crate) fn new(recv: RecvStream, leftover: BytesMut, expected_len: Option<u64>) -> Self {
        Self {
            inner: Inner::Active {
                recv,
                buf: leftover,
                data_remaining: 0,
                expected_len,
                seen_len: 0,
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
                expected_len,
                seen_len,
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
                    *seen_len = seen_len.saturating_add(take as u64);
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
                    // RFC 9114 §4.1.2: sum of DATA frame payloads must equal
                    // Content-Length. Reject overflow as soon as we see it
                    // rather than after streaming.
                    if let Some(exp) = *expected_len {
                        if seen_len.saturating_add(len.0 as u64) > exp {
                            this.inner = Inner::Ended;
                            return Poll::Ready(Some(Err(invalid_data("DATA bytes exceed Content-Length"))));
                        }
                    }
                    *data_remaining = len.0;
                    // loop to yield the first chunk of payload.
                }
                Ok(WireFrame::Headers(block)) => {
                    buf.advance(consumed);
                    // Trailers terminate the body — Content-Length must already match.
                    if let Some(exp) = *expected_len {
                        if *seen_len != exp {
                            this.inner = Inner::Ended;
                            return Poll::Ready(Some(Err(invalid_data(
                                "DATA bytes mismatch Content-Length before trailers",
                            ))));
                        }
                    }
                    // RFC 9114 §4.1: a HEADERS frame after DATA carries trailers.
                    let decoded = match decode_stateless(&mut block.clone(), MAX_HEADER_BLOCK_BYTES) {
                        Ok(d) => d,
                        Err(_) => {
                            this.inner = Inner::Ended;
                            return Poll::Ready(Some(Err(invalid_data("qpack decode trailer"))));
                        }
                    };
                    if decoded.dyn_ref {
                        this.inner = Inner::Ended;
                        return Poll::Ready(Some(Err(invalid_data("trailer referenced qpack dynamic table"))));
                    }
                    let header = match Header::try_from(decoded.fields) {
                        Ok(h) => h,
                        Err(_) => {
                            this.inner = Inner::Ended;
                            return Poll::Ready(Some(Err(invalid_data("malformed trailer"))));
                        }
                    };
                    let trailers = match header.try_into_trailers() {
                        Ok(t) => t,
                        Err(_) => {
                            this.inner = Inner::Ended;
                            return Poll::Ready(Some(Err(invalid_data("pseudo-header in trailer"))));
                        }
                    };
                    // After trailers, no more frames are legal on a request stream.
                    this.inner = Inner::Ended;
                    return Poll::Ready(Some(Ok(Frame::Trailers(trailers))));
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
                            // Clean end-of-stream between frames. Verify the
                            // body length matches Content-Length before we
                            // signal a successful end.
                            if let Some(exp) = *expected_len {
                                if *seen_len != exp {
                                    this.inner = Inner::Ended;
                                    return Poll::Ready(Some(Err(invalid_data(
                                        "DATA bytes mismatch Content-Length at end-of-stream",
                                    ))));
                                }
                            }
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
        Poll::Ready(Err(e)) => Poll::Ready(Err(Box::new(io::Error::other(e.to_string())))),
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
