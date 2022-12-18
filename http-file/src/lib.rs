//! local file serving with http.

#![feature(type_alias_impl_trait)]

pub mod runtime;

mod chunk;
mod date;
mod error;

pub use self::{chunk::ChunkReader, error::ServeError};

use std::path::{Component, Path, PathBuf};

use http::{
    header::{HeaderValue, CONTENT_LENGTH, CONTENT_TYPE, LAST_MODIFIED},
    Method, Request, Response,
};
use mime_guess::mime;
use percent_encoding::percent_decode;

use self::runtime::{AsyncFs, Meta};

#[cfg(feature = "tokio")]
#[derive(Clone)]
pub struct ServeDir<FS: AsyncFs = runtime::TokioFs> {
    chunk_size: usize,
    base_path: PathBuf,
    async_fs: FS,
}

#[cfg(not(feature = "tokio"))]
#[derive(Clone)]
pub struct ServeDir<FS: AsyncFs> {
    chunk_size: usize,
    base_path: PathBuf,
    async_fs: FS,
}

#[cfg(feature = "default")]
impl ServeDir<runtime::TokioFs> {
    /// Construct a new ServeDir with given path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::with_fs(path, runtime::TokioFs)
    }
}

#[cfg(feature = "tokio-uring")]
impl ServeDir<runtime::TokioUringFs> {
    /// Construct a new ServeDir with given path.
    pub fn new_tokio_uring(path: impl Into<PathBuf>) -> Self {
        Self::with_fs(path, runtime::TokioUringFs)
    }
}

impl<FS: AsyncFs> ServeDir<FS> {
    pub fn with_fs(path: impl Into<PathBuf>, async_fs: FS) -> Self {
        Self {
            chunk_size: 4096,
            base_path: path.into(),
            async_fs,
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
    pub async fn serve<Ext>(&self, req: &Request<Ext>) -> Result<Response<ChunkReader<FS::File>>, ServeError> {
        if !matches!(*req.method(), Method::HEAD | Method::GET) {
            return Err(ServeError::MethodNotAllowed);
        }

        let path = self.path_check(req.uri().path()).ok_or(ServeError::InvalidPath)?;

        if path.is_dir() {
            return Err(ServeError::InvalidPath);
        }

        let ct = mime_guess::from_path(&path)
            .first_raw()
            .unwrap_or_else(|| mime::APPLICATION_OCTET_STREAM.as_ref());

        let mut file = self.async_fs.open(path).await?;

        let modified = date::mod_date_check(req, &mut file)?;

        let size = file.len();

        let stream = if matches!(*req.method(), Method::HEAD) {
            ChunkReader::empty()
        } else {
            ChunkReader::reader(file, size, self.chunk_size)
        };
        let mut res = Response::new(stream);

        res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(ct));
        res.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from(size));

        if let Some(modified) = modified {
            let bytes = date::date_to_bytes(modified);
            if let Ok(val) = HeaderValue::from_maybe_shared(bytes) {
                res.headers_mut().insert(LAST_MODIFIED, val);
            }
        }

        Ok(res)
    }
}

impl<FS: AsyncFs> ServeDir<FS> {
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

    fn assert_send<F: Send>(_: &F) {}

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
    async fn tokio_fs_assert_send() {
        let dir = ServeDir::new("sample");
        let req = Request::builder().uri("/test.txt").body(()).unwrap();

        let fut = dir.serve(&req);

        assert_send(&fut);

        let res = fut.await.unwrap();

        assert_send(&res);
    }

    #[tokio::test]
    async fn method() {
        let dir = ServeDir::new("sample");
        let req = Request::builder()
            .method(Method::POST)
            .uri("/test.txt")
            .body(())
            .unwrap();

        let e = dir.serve(&req).await.err().unwrap();
        assert!(matches!(e, ServeError::MethodNotAllowed));
    }

    #[tokio::test]
    async fn head_method_body_check() {
        let dir = ServeDir::new("sample");
        let req = Request::builder()
            .method(Method::HEAD)
            .uri("/test.txt")
            .body(())
            .unwrap();

        let res = dir.serve(&req).await.unwrap();

        assert_eq!(
            res.headers().get(CONTENT_LENGTH).unwrap(),
            HeaderValue::from("hello, world!".len())
        );

        let mut stream = Box::pin(res.into_body());

        assert_eq!(stream.size_hint(), (usize::MAX, Some(0)));

        let body_chunk = poll_fn(|cx| stream.as_mut().poll_next(cx)).await;

        assert!(body_chunk.is_none())
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
        assert_eq!(
            res.headers().get(CONTENT_LENGTH).unwrap(),
            HeaderValue::from("hello, world!".len())
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

    #[cfg(feature = "tokio-uring")]
    #[test]
    fn basic_tokio_uring() {
        tokio_uring::start(async {
            let dir = ServeDir::new_tokio_uring("sample");
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
        })
    }

    #[cfg(feature = "tokio-uring")]
    #[test]
    fn tokio_uring_assert_send() {
        tokio_uring::start(async {
            let dir = ServeDir::new_tokio_uring("sample");
            let req = Request::builder().uri("/test.txt").body(()).unwrap();

            let fut = dir.serve(&req);

            assert_send(&fut);

            let _ = fut.await.unwrap();
        })
    }
}
