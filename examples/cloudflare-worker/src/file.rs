/// use `rust_embed` crate to generate in memory static file.
///
/// *. `debug-assertions` must be set to false in profile to enable in memory file.
use core::{future::Future, time::Duration};

use std::{
    io::{self, SeekFrom},
    path::PathBuf,
    time::SystemTime,
};

use rust_embed::{EmbeddedFile, RustEmbed};

use http_file::runtime::{AsyncFs, ChunkRead, Meta};
use xitca_http::bytes::BytesMut;

#[derive(RustEmbed)]
#[folder = "../file/static"]
pub struct Files;

impl AsyncFs for Files {
    type File = StaticFile;
    type OpenFuture = impl Future<Output = io::Result<Self::File>> + Send;

    fn open(&self, path: PathBuf) -> Self::OpenFuture {
        async move {
            let path = path
                .to_str()
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            let file = Files::get(path).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;

            Ok(StaticFile { pos: 0, file })
        }
    }
}

pub struct StaticFile {
    pos: u64,
    file: EmbeddedFile,
}

impl Meta for StaticFile {
    fn modified(&mut self) -> Option<SystemTime> {
        self.file
            .metadata
            .last_modified()
            .map(|l| SystemTime::UNIX_EPOCH + Duration::from_secs(l))
    }

    fn len(&self) -> u64 {
        self.file.data.len() as u64
    }
}

impl ChunkRead for StaticFile {
    type SeekFuture<'f> = impl Future<Output = io::Result<()>> + Send + 'f where Self: 'f;
    type Future = impl Future<Output = io::Result<Option<(Self, BytesMut, usize)>>> + Send;

    fn seek(&mut self, pos: SeekFrom) -> Self::SeekFuture<'_> {
        let SeekFrom::Start(pos) = pos else { unreachable!("") };
        self.pos += pos;
        async { Ok(()) }
    }

    fn next(mut self, mut buf: BytesMut) -> Self::Future {
        async {
            if self.pos == self.len() {
                return Ok(None);
            }

            let end = core::cmp::min(self.pos + 4096, self.len());
            let start = self.pos;
            let len = end - start;
            buf.extend_from_slice(&self.file.data[start as usize..end as usize]);
            self.pos += len;

            Ok(Some((self, buf, len as usize)))
        }
    }
}
