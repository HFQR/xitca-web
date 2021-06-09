use std::{
    future::Future,
    io::{self, Write},
};

use bytes::Bytes;
use tokio::task::spawn_blocking;

use super::decoder::AsyncDecode;
use super::writer::{Writer, MAX_CHUNK_SIZE_DECODE_IN_PLACE};

pub(super) use brotli2::write::BrotliDecoder;

impl<Item> AsyncDecode<Item> for BrotliDecoder<Writer>
where
    Item: AsRef<[u8]> + Send + 'static,
{
    type Item = Bytes;
    type Future = impl Future<Output = io::Result<(Self, Option<Self::Item>)>>;

    fn decode(self, item: Item) -> Self::Future {
        async move {
            let buf = item.as_ref();
            if buf.len() < MAX_CHUNK_SIZE_DECODE_IN_PLACE {
                decode(self, buf)
            } else {
                spawn_blocking(move || decode(self, item.as_ref()))
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            }
        }
    }

    fn decode_eof(mut self) -> io::Result<Option<Self::Item>> {
        let b = self.finish()?.take();

        if !b.is_empty() {
            Ok(Some(b))
        } else {
            Ok(None)
        }
    }
}

fn decode(mut this: BrotliDecoder<Writer>, buf: &[u8]) -> io::Result<(BrotliDecoder<Writer>, Option<Bytes>)> {
    this.write_all(buf)?;
    this.flush()?;
    let b = this.get_mut().take();
    if !b.is_empty() {
        Ok((this, Some(b)))
    } else {
        Ok((this, None))
    }
}
