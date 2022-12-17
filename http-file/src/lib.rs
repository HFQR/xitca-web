//! local file serving with http.

#![feature(type_alias_impl_trait)]

mod chunk;
mod date;
mod error;

pub use self::{chunk::ChunkReadStream, error::ServeError};

use std::path::{Component, Path, PathBuf};

use http::{
    header::{HeaderValue, CONTENT_TYPE, LAST_MODIFIED},
    Method, Request, Response,
};
use mime_guess::mime;
use percent_encoding::percent_decode;

#[derive(Clone)]
pub struct ServeDir {
    chunk_size: usize,
    base_path: PathBuf,
}

impl ServeDir {
    /// Construct a new ServeDir with given path.
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

    /// try to find a matching file from given input request and generate http response with stream
    /// reader of matched file.
    ///
    /// # Examples
    /// ```rust
    /// # use http_file::ServeDir;
    /// # use http::Request;
    /// async fn serve(req: &Request<()>) {
    ///     let dir = ServeDir::new("sample");
    ///     let res = dir.serve(&req).await;
    /// }
    /// ```
    ///
    /// # Note
    /// Due to different callers would potentially handling the response body in different way the
    /// output [Response] does not carry `content-length` header.
    /// If the file is supposed to be served as sized body rather than `transfer-encoding: chunked`
    /// one can get the body length with following method:
    /// ```rust
    /// # use http_file::ChunkReadStream;
    /// use futures::Stream;
    /// use http::{header::{CONTENT_LENGTH, HeaderValue}, Response};
    ///
    /// fn add_content_length(res: &mut Response<ChunkReadStream>) {
    ///     if let (lower, Some(upper)) = res.body().size_hint() {
    ///         // when lower and upper size hint is the same non zero value it means the size
    ///         // would be the exact length of file body stream.
    ///         if lower == upper && lower != 0 {
    ///             res.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from(lower));
    ///         }
    ///     }
    /// }
    /// ```
    pub async fn serve<Ext>(&self, req: &Request<Ext>) -> Result<Response<ChunkReadStream>, ServeError> {
        if !matches!(*req.method(), Method::HEAD | Method::GET) {
            return Err(ServeError::MethodNotAllowed);
        }

        let path = self.path_check(req.uri().path()).ok_or(ServeError::InvalidPath)?;

        assert!(!path.is_dir());

        let ct = mime_guess::from_path(&path)
            .first_raw()
            .unwrap_or_else(|| mime::APPLICATION_OCTET_STREAM.as_ref());

        let (file, md) = tokio::task::spawn_blocking(move || {
            use std::{fs::File, io};
            let file = File::open(path)?;
            let meta = file.metadata()?;
            Ok::<_, io::Error>((file.into(), meta))
        })
        .await
        .unwrap()?;

        let modified = date::mod_date_check(req, &md)?;

        let size = md.len();

        let stream = chunk::chunk_read_stream(file, size, self.chunk_size);
        let mut res = Response::new(stream);

        res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(ct));

        if let Some(modified) = modified {
            let bytes = date::date_to_bytes(modified);
            if let Ok(val) = HeaderValue::from_maybe_shared(bytes) {
                res.headers_mut().insert(LAST_MODIFIED, val);
            }
        }

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

    use futures_core::stream::Stream;

    use super::*;

    #[tokio::test]
    async fn basic() {
        let dir = ServeDir::new("sample");
        let req = Request::builder().uri("/test.txt").body(()).unwrap();

        let mut stream = Box::pin(dir.serve(&req).await.unwrap().into_body());

        let (low, high) = stream.size_hint();

        assert_eq!(low, high.unwrap());
        assert_eq!(low, "hello, world!".len());

        let mut res = String::new();

        while let Some(Ok(bytes)) = poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
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
    async fn response_headers() {
        let dir = ServeDir::new("sample");
        let req = Request::builder().uri("/test.txt").body(()).unwrap();
        let res = dir.serve(&req).await.unwrap();
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain")
        );
    }

    #[tokio::test]
    async fn body_size_hint() {
        let dir = ServeDir::new("sample");
        let req = Request::builder().uri("/test.txt").body(()).unwrap();
        let res = dir.serve(&req).await.unwrap();
        let (lower, Some(upper)) = res.body().size_hint() else { panic!("ChunkReadStream does not have a size") };
        assert_eq!(lower, upper);
        assert_eq!(lower, "hello, world!".len());
    }
}
