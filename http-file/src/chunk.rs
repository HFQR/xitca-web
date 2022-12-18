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
    pub struct ChunkedReader<F>
    where
        F: ChunkRead,
    {
        chunk_size: usize,
        size: u64,
        #[pin]
        on_flight: F::Future
    }
}

pub(super) fn chunk_read_stream<F>(file: F, chunk_size: usize) -> ChunkedReader<F>
where
    F: ChunkRead,
{
    ChunkedReader {
        chunk_size,
        size: file.len(),
        on_flight: file.next(BytesMut::with_capacity(chunk_size)),
    }
}

impl<F> Stream for ChunkedReader<F>
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
