use core::{
    mem,
    pin::Pin,
    task::{Context, Poll, ready},
};

use std::io;

use pin_project_lite::pin_project;
use xitca_io::io::{AsyncBufRead, AsyncBufWrite};

use crate::{
    body::{Body, BoxBody, Frame, SizeHint},
    bytes::{Bytes, BytesMut},
    error::BodyError,
};

use super::{
    io::{BufIo, NotifierIo},
    proto::{
        encode::CONTINUE_BYTES,
        trasnder_coding::{ChunkResult, TransferCoding},
    },
};

pub use crate::body::RequestBody;

pub(super) fn body<Io>(
    io: NotifierIo<Io>,
    is_expect: bool,
    limit: usize,
    decoder: TransferCoding,
    read_buf: BytesMut,
) -> RequestBody
where
    Io: AsyncBufRead + AsyncBufWrite + 'static,
{
    let size = match decoder {
        TransferCoding::Length(len) => SizeHint::Exact(len),
        TransferCoding::Eof => SizeHint::None,
        _ => SizeHint::Unknown,
    };

    let body = BodyInner {
        io,
        decoder: Decoder {
            decoder,
            limit,
            read_buf,
        },
    };

    let state = if is_expect {
        State::ExpectWrite {
            fut: async {
                let (res, _) = body.io.io().write_all(CONTINUE_BYTES).await;
                res.map(|_| body)
            },
        }
    } else {
        State::Body { body }
    };

    RequestBody::Boxed(BoxBody::new(BodyReader {
        size,
        chunk_read,
        state,
    }))
}

pin_project! {
    #[project = StateProj]
    #[project_replace = StateProjReplace]
    enum State<Io, FutC, FutE> {
        Body {
            body: BodyInner<Io>
        },
        ChunkRead {
            #[pin]
            fut: FutC
        },
        ExpectWrite {
            #[pin]
            fut: FutE,
        },
        None,
    }
}

pin_project! {
    struct BodyReader<Io, F, FutC, FutE> {
        size: SizeHint,
        chunk_read: F,
        #[pin]
        state: State<Io, FutC, FutE>
    }
}

struct BodyInner<Io> {
    io: NotifierIo<Io>,
    decoder: Decoder,
}

async fn chunk_read<Io>(mut body: BodyInner<Io>) -> io::Result<(usize, BodyInner<Io>)>
where
    Io: AsyncBufRead,
{
    let (res, r_buf) = body.decoder.read_buf.split().read(body.io.io()).await;
    body.decoder.read_buf.unsplit(r_buf);
    let read = res?;
    Ok((read, body))
}

impl<Io, F, FutC, FutE> Body for BodyReader<Io, F, FutC, FutE>
where
    Io: AsyncBufRead,
    F: Fn(BodyInner<Io>) -> FutC,
    FutC: Future<Output = io::Result<(usize, BodyInner<Io>)>>,
    FutE: Future<Output = io::Result<BodyInner<Io>>>,
{
    type Data = Bytes;
    type Error = BodyError;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let mut this = self.project();
        loop {
            match this.state.as_mut().project() {
                StateProj::Body { body } => {
                    match body.decoder.decode() {
                        ChunkResult::Ok(bytes) => return Poll::Ready(Some(Ok(Frame::Data(bytes)))),
                        ChunkResult::Trailers(trailers) => return Poll::Ready(Some(Ok(Frame::Trailers(trailers)))),
                        ChunkResult::Err(e) => return Poll::Ready(Some(Err(e.into()))),
                        ChunkResult::InsufficientData => body.decoder.limit_check()?,
                        _ => return Poll::Ready(None),
                    }

                    let StateProjReplace::Body { body } = this.state.as_mut().project_replace(State::None) else {
                        unreachable!()
                    };
                    this.state.as_mut().project_replace(State::ChunkRead {
                        fut: (this.chunk_read)(body),
                    });
                }
                StateProj::ChunkRead { fut } => {
                    let (read, body) = ready!(fut.poll(cx))?;
                    if read == 0 {
                        this.state.as_mut().project_replace(State::None);
                        return Poll::Ready(None);
                    }
                    this.state.as_mut().project_replace(State::Body { body });
                }
                StateProj::ExpectWrite { fut } => {
                    let body = ready!(fut.poll(cx))?;
                    this.state.as_mut().project_replace(State::ChunkRead {
                        fut: (this.chunk_read)(body),
                    });
                }
                StateProj::None => return Poll::Ready(None),
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.size
    }
}

impl<Io> Drop for BodyInner<Io> {
    fn drop(&mut self) {
        if self.decoder.decoder.is_eof() {
            let buf = mem::take(&mut self.decoder.read_buf);
            self.io.notify(buf);
        }
    }
}

struct Decoder {
    decoder: TransferCoding,
    limit: usize,
    read_buf: BytesMut,
}

impl Decoder {
    fn decode(&mut self) -> ChunkResult {
        self.decoder.decode(&mut self.read_buf)
    }

    fn limit_check(&self) -> io::Result<()> {
        if self.read_buf.len() < self.limit {
            return Ok(());
        }

        let msg = format!(
            "READ_BUF_LIMIT reached: {{ limit: {}, length: {} }}",
            self.limit,
            self.read_buf.len()
        );
        Err(io::Error::other(msg))
    }
}
