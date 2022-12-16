use core::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::io;

use bytes::{Bytes, BytesMut};
use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use tokio::{fs::File, io::AsyncReadExt};

pin_project! {
    struct ChunkedReader<F, Fut> {
        size: u64,
        chunk_size: usize,
        read: F,
        #[pin]
        on_flight: Fut
    }
}

pub type ChunkReadStream = impl Stream<Item = io::Result<Bytes>> + Send;

pub(super) fn chunk_read_stream(file: File, size: u64, chunk_size: usize) -> ChunkReadStream {
    ChunkedReader {
        size,
        chunk_size,
        read,
        on_flight: read(file, BytesMut::with_capacity(chunk_size)),
    }
}

async fn read(mut file: File, mut bytes: BytesMut) -> io::Result<Option<(BytesMut, File, usize)>> {
    let n = file.read_buf(&mut bytes).await?;

    if n == 0 {
        Ok(None)
    } else {
        Ok(Some((bytes, file, n)))
    }
}

impl<F, Fut> Stream for ChunkedReader<F, Fut>
where
    F: FnMut(File, BytesMut) -> Fut,
    Fut: Future<Output = io::Result<Option<(BytesMut, File, usize)>>>,
{
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if *this.size == 0 {
            return Poll::Ready(None);
        }

        Poll::Ready(ready!(this.on_flight.as_mut().poll(cx))?.map(|(mut bytes, file, n)| {
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
            this.on_flight.set((this.read)(file, bytes));

            Ok(chunk.freeze())
        }))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.size as usize;
        (size, Some(size))
    }
}
