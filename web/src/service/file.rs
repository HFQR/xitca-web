//! static file serving.

use core::convert::Infallible;

use std::path::PathBuf;

use http_file::{ServeDir as _ServeDir, runtime::AsyncFs};
use xitca_http::util::service::router::{PathGen, RouteGen};

use crate::service::Service;

/// builder type for serve dir service.
pub struct ServeDir<F: AsyncFs = dumb::Dumb> {
    inner: _ServeDir<F>,
}

#[cfg(feature = "file")]
impl ServeDir {
    /// construct a new static file service that serve given relative file path's file based on file name.
    ///
    /// # Example
    /// ```rust
    /// # use xitca_web::{
    /// #     handler::{handler_service},
    /// #     service::file::ServeDir,
    /// #     App, WebContext
    /// # };
    /// App::new()
    ///     // http request would be matched against files inside ./static file path.
    ///     .at("/", ServeDir::new("static"))
    ///     // other named routes have higher priority than serve dir service.
    ///     .at("/foo", handler_service(|| async { "foo!" }))
    ///     # .at("/bar", handler_service(|_: &WebContext<'_>| async { "used for inferring types!" }));
    /// ```
    pub fn new(path: impl Into<PathBuf>) -> ServeDir<impl AsyncFs + Clone> {
        ServeDir {
            inner: _ServeDir::new(path),
        }
    }

    #[cfg(feature = "io-uring")]
    pub fn new_tokio_uring(path: impl Into<PathBuf>) -> ServeDir<impl AsyncFs + Clone> {
        ServeDir {
            inner: _ServeDir::new_tokio_uring(path),
        }
    }
}

impl<F> ServeDir<F>
where
    F: AsyncFs,
{
    /// construct a new static file service with given file system. file system must be a type impl [AsyncFs]
    /// trait to instruct how async read/write of disk(or in memory) file can be performed.
    pub fn with_fs(path: impl Into<PathBuf>, fs: F) -> Self {
        ServeDir {
            inner: _ServeDir::with_fs(path, fs),
        }
    }
}

impl<F> PathGen for ServeDir<F>
where
    F: AsyncFs,
{
    fn path_gen(&mut self, prefix: &str) -> String {
        let mut prefix = String::from(prefix);
        if prefix.ends_with('/') {
            prefix.pop();
        }

        prefix.push_str("/*p");

        prefix
    }
}

impl<F> RouteGen for ServeDir<F>
where
    F: AsyncFs,
{
    type Route<R> = R;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        route
    }
}

impl<F> Service for ServeDir<F>
where
    F: AsyncFs + Clone,
{
    type Response = service::ServeDirService<F>;
    type Error = Infallible;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        Ok(service::ServeDirService(self.inner.clone()))
    }
}

mod service {
    use http_file::{ServeDir, ServeError, runtime::AsyncFs};

    use crate::{
        body::ResponseBody,
        context::WebContext,
        error::{Error, ErrorStatus, MatchError, MethodNotAllowed, RouterError},
        http::{Method, StatusCode, WebResponse},
        service::Service,
    };

    pub struct ServeDirService<F: AsyncFs>(pub(super) ServeDir<F>);

    impl<'r, C, B, F> Service<WebContext<'r, C, B>> for ServeDirService<F>
    where
        F: AsyncFs,
        F::File: 'static,
    {
        type Response = WebResponse;
        type Error = RouterError<Error>;

        async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
            match self.0.serve(ctx.req()).await {
                Ok(res) => Ok(res.map(ResponseBody::box_stream)),
                Err(ServeError::NotModified) => {
                    let mut res = ctx.into_response(ResponseBody::none());
                    *res.status_mut() = StatusCode::NOT_MODIFIED;
                    Ok(res)
                }
                Err(e) => Err(match e {
                    ServeError::NotFound => RouterError::Match(MatchError),
                    ServeError::MethodNotAllowed => {
                        RouterError::NotAllowed(MethodNotAllowed(Box::new(vec![Method::GET, Method::HEAD])))
                    }
                    ServeError::Io(io) => RouterError::Service(Error::from(io)),
                    _ => RouterError::Service(Error::from(ErrorStatus::bad_request())),
                }),
            }
        }
    }
}

mod dumb {
    use core::future::Ready;

    use std::{io, path::PathBuf};

    use http_file::runtime::{AsyncFs, ChunkRead, Meta};

    use crate::bytes::BytesMut;

    // a dummy type to help make ServerDir::new work as is.
    #[derive(Clone, Copy)]
    pub struct Dumb;

    impl AsyncFs for Dumb {
        type File = DumbFile;

        type OpenFuture = Ready<io::Result<Self::File>>;

        fn open(&self, _: PathBuf) -> Self::OpenFuture {
            unimplemented!()
        }
    }

    // just like Dumb
    pub struct DumbFile;

    impl Meta for DumbFile {
        fn modified(&mut self) -> Option<std::time::SystemTime> {
            unimplemented!()
        }

        fn len(&self) -> u64 {
            unimplemented!()
        }
    }

    impl ChunkRead for DumbFile {
        type SeekFuture<'f>
            = Ready<io::Result<()>>
        where
            Self: 'f;
        type Future = Ready<io::Result<Option<(Self, BytesMut, usize)>>>;

        fn seek(&mut self, _: io::SeekFrom) -> Self::SeekFuture<'_> {
            unimplemented!()
        }

        fn next(self, _: xitca_http::bytes::BytesMut) -> Self::Future {
            unimplemented!()
        }
    }
}
