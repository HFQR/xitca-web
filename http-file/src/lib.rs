//! local file serving with http.

mod chunk;

pub use chunk::ChunkReadStream;

use std::path::{Component, Path, PathBuf};

use http::{Method, Request, Response};
use percent_encoding::percent_decode;

pub struct ServeDir {
    chunk_size: usize,
    base_path: PathBuf,
}

impl ServeDir {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            chunk_size: 4096,
            base_path: path.into(),
        }
    }

    pub fn chunk_size(&mut self, size: usize) -> &mut Self {
        self.chunk_size = size;
        self
    }

    pub async fn serve<Ext>(&self, req: Request<Ext>) -> Result<Response<ChunkReadStream>, ()> {
        if !matches!(*req.method(), Method::HEAD | Method::GET) {
            return Err(());
        }

        let path = self.path_check(req.uri().path()).unwrap();

        assert!(!path.is_dir());

        let file = tokio::fs::File::open(path).await.unwrap();

        let stream = chunk::chunk_read_stream(file, self.chunk_size);

        Ok(Response::new(stream))
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

#[cfg(test)]
mod test {
    use core::future::poll_fn;

    use super::*;

    #[tokio::test]
    async fn basic() {
        let dir = ServeDir::new("sample");
        let req = Request::builder().uri("/test.txt").body(()).unwrap();

        let mut file = dir.serve(req).await.unwrap().into_body();

        let mut res = String::new();

        while let Some(Ok(bytes)) = poll_fn(|cx| file.as_mut().poll_next(cx)).await {
            res.push_str(std::str::from_utf8(bytes.as_ref()).unwrap());
        }

        assert_eq!(res, "hello, world!");
    }

    #[tokio::test]
    async fn method() {
        let dir = ServeDir::new("sample");
        let req = Request::builder()
            .method(Method::POST)
            .uri("/test.txt")
            .body(())
            .unwrap();

        assert!(dir.serve(req).await.is_err());
    }
}
