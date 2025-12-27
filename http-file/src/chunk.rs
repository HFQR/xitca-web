use core::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::io;

use bytes::{Bytes, BytesMut};
use futures_core::stream::Stream;
use pin_project_lite::pin_project;

use super::runtime::ChunkRead;

pin_project! {
    /// chunked file reader with async [Stream]
    #[project = ChunkReaderProj]
    pub enum ChunkReader<F>
    where
        F: ChunkRead,
    {
        Empty,
        Reader {
            #[pin]
            reader:  _ChunkReader<F>
        }
    }
}

impl<F> ChunkReader<F>
where
    F: ChunkRead,
{
    pub(super) fn empty() -> Self {
        Self::Empty
    }

    pub(super) fn reader(file: F, size: u64, chunk_size: usize) -> Self {
        Self::Reader {
            reader: _ChunkReader {
                chunk_size,
                size,
                on_flight: file.next(BytesMut::with_capacity(chunk_size)),
            },
        }
    }
}

impl<F> Stream for ChunkReader<F>
where
    F: ChunkRead,
{
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.project() {
            ChunkReaderProj::Empty => Poll::Ready(None),
            ChunkReaderProj::Reader { reader } => reader.poll_next(cx),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            // see xitca_http::body::none_body_hint for reason. this is a library hack.
            Self::Empty => (usize::MAX, Some(0)),
            Self::Reader { reader } => reader.size_hint(),
        }
    }
}

pin_project! {
    #[doc(hidden)]
    pub struct _ChunkReader<F>
    where
        F: ChunkRead,
    {
        chunk_size: usize,
        size: u64,
        #[pin]
        on_flight: F::Future
    }
}

impl<F> Stream for _ChunkReader<F>
where
    F: ChunkRead,
{
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if *this.size == 0 {
            return Poll::Ready(None);
        }

        Poll::Ready(ready!(this.on_flight.as_mut().poll(cx))?.map(|(file, mut bytes, n)| {
            let mut chunk = bytes.split_to(n);

            let n = n as u64;

            if *this.size <= n {
                if *this.size < n {
                    // an unlikely case happen when someone append data to file while it's being
                    // read.
                    // drop the extra part. only self.size bytes of data were promised to client.
                    chunk.truncate(*this.size as usize);
                }
                *this.size = 0;
                return Ok(chunk.freeze());
            }

            *this.size -= n;

            // TODO: better handling additional memory alloc?
            // the goal should be linear growth targeting page size.
            bytes.reserve(*this.chunk_size);
            this.on_flight.set(file.next(bytes));

            Ok(chunk.freeze())
        }))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.size as usize;
        (size, Some(size))
    }
}
