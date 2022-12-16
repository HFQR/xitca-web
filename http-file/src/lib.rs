//! local file serving with http.

mod chunk;
mod error;

pub use self::{chunk::ChunkReadStream, error::ServeError};

use std::path::{Component, Path, PathBuf};

use http::{
    header::{HeaderValue, CONTENT_TYPE},
    Method, Request, Response,
};
use mime_guess::mime;
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

    pub async fn serve<Ext>(&self, req: &Request<Ext>) -> Result<Response<ChunkReadStream>, ServeError> {
        if !matches!(*req.method(), Method::HEAD | Method::GET) {
            return Err(ServeError::MethodNotAllowed);
        }

        let path = self.path_check(req.uri().path()).ok_or(ServeError::InvalidPath)?;

        assert!(!path.is_dir());

        let ct = mime_guess::from_path(&path)
            .first_raw()
            .map(HeaderValue::from_static)
            .unwrap_or_else(|| HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()));

        let (file, _) = tokio::task::spawn_blocking(move || {
            use std::{fs, io};

            use tokio::fs::File;

            let file = fs::File::open(path)?;
            let meta = file.metadata()?;
            Ok::<_, io::Error>((File::from_std(file), meta))
        })
        .await
        .unwrap()?;

        let stream = chunk::chunk_read_stream(file, self.chunk_size);
        let mut res = Response::new(stream);

        res.headers_mut().insert(CONTENT_TYPE, ct);

        Ok(res)
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

        let mut file = dir.serve(&req).await.unwrap().into_body();

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
        assert!(matches!(
            dir.serve(&req).await.err(),
            Some(ServeError::MethodNotAllowed)
        ));
    }

    #[tokio::test]
    async fn invalid_path() {
        let dir = ServeDir::new("sample");
        let req = Request::builder().uri("/../test.txt").body(()).unwrap();
        assert!(matches!(dir.serve(&req).await.err(), Some(ServeError::InvalidPath)));
    }

    #[tokio::test]
    async fn content_type() {
        let dir = ServeDir::new("sample");
        let req = Request::builder().uri("/test.txt").body(()).unwrap();
        let res = dir.serve(&req).await.unwrap();
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain")
        );
    }
}
