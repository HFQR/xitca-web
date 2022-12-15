//! local file serving with http.

use core::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::path::{Component, Path, PathBuf};

use bytes::{Bytes, BytesMut};
use futures_core::stream::Stream;
use http::{Method, Request, Response};
use percent_encoding::percent_decode;
use pin_project_lite::pin_project;
use tokio_uring::fs::File;

pub struct ServeDir {
    base_path: PathBuf,
}

impl ServeDir {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { base_path: path.into() }
    }

    pub async fn serve<Ext>(&self, req: Request<Ext>) -> Result<Response<()>, ()> {
        if matches!(*req.method(), Method::HEAD | Method::GET) {
            return Err(());
        }

        let path = self.path_check(req.uri().path()).unwrap();

        assert!(!path.is_dir());

        let file = File::open(path).await.unwrap();

        async fn callback(bytes: BytesMut, file: File, offset: usize) -> (BytesMut, File, usize) {
            let (res, buf) = file.read_at(bytes, offset as _).await;
            let n = res.unwrap();
            todo!()
        }

        let body = ChunkedRead {
            callback,
            fut: callback(BytesMut::new(), file, 0),
        };

        Ok(Response::new(()))
    }
}

impl ServeDir {
    fn path_check(&self, path: &str) -> Option<PathBuf> {
        let path = path.trim_start_matches('/').as_bytes();

        let path_decoded = percent_decode(path).decode_utf8().ok()?;
        let path_decoded = Path::new(&*path_decoded);

        let mut path = self.base_path.clone();

        for component in path_decoded.components() {
            match component {
                Component::Normal(comp) => {
                    if Path::new(&comp)
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                    {
                        return None;
                    }
                    path.push(comp)
                }
                Component::CurDir => {}
                Component::Prefix(_) | Component::RootDir | Component::ParentDir => return None,
            }
        }

        Some(path)
    }
}

pin_project! {
    pub struct ChunkedRead<F, Fut> {
        callback: F,
        #[pin]
        fut: Fut
    }
}
impl<F, Fut> Stream for ChunkedRead<F, Fut>
where
    F: FnMut(BytesMut, File, usize) -> Fut + Unpin,
    Fut: Future<Output = (BytesMut, File)>,
{
    type Item = Bytes;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        let (mut bytes, file) = ready!(this.fut.as_mut().poll(cx));

        let chunk = bytes.split().freeze();

        this.fut.set((this.callback)(bytes, file, 0));

        Poll::Ready(Some(chunk))
    }
}
