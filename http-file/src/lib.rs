//! local file serving with http.

pub mod runtime;

mod buf;
mod chunk;
mod date;
mod error;

pub use self::{chunk::ChunkReader, error::ServeError};

use std::{
    io::SeekFrom,
    path::{Component, Path, PathBuf},
};

use http::{
    header::{HeaderValue, ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, LAST_MODIFIED, RANGE},
    Method, Request, Response, StatusCode,
};
use mime_guess::mime;

use self::{
    buf::buf_write_header,
    runtime::{AsyncFs, ChunkRead, Meta},
};

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
    /// construct a new ServeDir with given path and async file system type. The type must impl
    /// [AsyncFs] trait for properly handling file streaming.
    pub fn with_fs(path: impl Into<PathBuf>, async_fs: FS) -> Self {
        Self {
            chunk_size: 4096,
            base_path: path.into(),
            async_fs,
        }
    }

    /// hint for chunk size of async file streaming.
    /// it's a best effort upper bound and should not be trusted to produce exact chunk size as
    /// under/over shoot can happen
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

        let path = self.path_check(req.uri().path())?;

        // TODO: enable nest dir serving?
        if path.is_dir() {
            return Err(ServeError::InvalidPath);
        }

        let ct = mime_guess::from_path(&path)
            .first_raw()
            .unwrap_or_else(|| mime::APPLICATION_OCTET_STREAM.as_ref());

        let mut file = self.async_fs.open(path).await?;

        let modified = date::mod_date_check(req, &mut file)?;

        let mut res = Response::new(());

        let mut size = file.len();

        if let Some(range) = req
            .headers()
            .get(RANGE)
            .and_then(|h| h.to_str().ok())
            .and_then(|range| http_range_header::parse_range_header(range).ok())
            .map(|range| range.validate(size))
        {
            let (start, end) = range
                .map_err(|_| ServeError::RangeNotSatisfied(size))?
                .pop()
                .expect("http_range_header produced empty range")
                .into_inner();

            file.seek(SeekFrom::Start(start)).await?;

            *res.status_mut() = StatusCode::PARTIAL_CONTENT;
            let val = buf_write_header!(0, "bytes {start}-{end}/{size}");
            res.headers_mut().insert(CONTENT_RANGE, val);

            size = end - start + 1;
        }

        res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(ct));
        res.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from(size));
        res.headers_mut()
            .insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));

        if let Some(modified) = modified {
            let val = date::date_to_header(modified);
            res.headers_mut().insert(LAST_MODIFIED, val);
        }

        let stream = if matches!(*req.method(), Method::HEAD) {
            ChunkReader::empty()
        } else {
            ChunkReader::reader(file, size, self.chunk_size)
        };

        Ok(res.map(|_| stream))
    }
}

impl<FS: AsyncFs> ServeDir<FS> {
    fn path_check(&self, path: &str) -> Result<PathBuf, ServeError> {
        let path = path.trim_start_matches('/').as_bytes();

        let path_decoded = percent_encoding::percent_decode(path)
            .decode_utf8()
            .map_err(|_| ServeError::InvalidPath)?;
        let path_decoded = Path::new(&*path_decoded);

        let mut path = self.base_path.clone();

        for component in path_decoded.components() {
            match component {
                Component::Normal(comp) => {
                    if Path::new(&comp)
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                    {
                        return Err(ServeError::InvalidPath);
                    }
                    path.push(comp)
                }
                Component::CurDir => {}
                Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                    return Err(ServeError::InvalidPath)
                }
            }
        }

        Ok(path)
    }
}

#[cfg(test)]
mod test {
    use core::future::poll_fn;

    use futures_core::stream::Stream;

    use super::*;

    fn assert_send<F: Send>(_: &F) {}

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
            res.headers().get(ACCEPT_RANGES).unwrap(),
            HeaderValue::from_static("bytes")
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
        let (lower, Some(upper)) = res.body().size_hint() else {
            panic!("ChunkReadStream does not have a size")
        };
        assert_eq!(lower, upper);
        assert_eq!(lower, "hello, world!".len());
    }

    async fn _basic<FS: AsyncFs>(dir: ServeDir<FS>) {
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
    async fn basic() {
        _basic(ServeDir::new("sample")).await;
    }

    #[cfg(all(target_os = "linux", feature = "tokio-uring"))]
    #[test]
    fn basic_tokio_uring() {
        tokio_uring::start(_basic(ServeDir::new_tokio_uring("sample")));
    }

    async fn test_range<FS: AsyncFs>(dir: ServeDir<FS>) {
        let req = Request::builder()
            .uri("/test.txt")
            .header("range", "bytes=2-12")
            .body(())
            .unwrap();
        let res = dir.serve(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain")
        );
        assert_eq!(
            res.headers().get(CONTENT_RANGE).unwrap(),
            HeaderValue::from_static("bytes 2-12/13")
        );
        assert_eq!(
            res.headers().get(CONTENT_LENGTH).unwrap(),
            HeaderValue::from("llo, world!".len())
        );

        let mut stream = Box::pin(res.into_body());

        let mut res = String::new();

        while let Some(Ok(bytes)) = poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
            res.push_str(std::str::from_utf8(bytes.as_ref()).unwrap());
        }

        assert_eq!("llo, world!", res);
    }

    #[tokio::test]
    async fn ranged() {
        test_range(ServeDir::new("sample")).await;
    }

    #[cfg(all(target_os = "linux", feature = "tokio-uring"))]
    #[test]
    fn ranged_tokio_uring() {
        tokio_uring::start(test_range(ServeDir::new_tokio_uring("sample")))
    }
}
