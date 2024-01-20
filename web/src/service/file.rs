//! static file serving.

use core::convert::Infallible;

use std::{borrow::Cow, path::PathBuf};

use http_file::ServeDir as _ServeDir;
use xitca_http::util::service::router::RouterGen;

use crate::service::Service;

/// builder type for serve dir service.
pub struct ServeDir {
    inner: _ServeDir,
}

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
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            inner: _ServeDir::new(path),
        }
    }
}

impl RouterGen for ServeDir {
    type Route<R> = R;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        let mut path = String::from(prefix);
        if path.ends_with('/') {
            path.pop();
        }

        path.push_str("/*p");

        Cow::Owned(path)
    }

    fn route_gen<R>(route: R) -> Self::Route<R> {
        route
    }
}

impl Service for ServeDir {
    type Response = service::ServeDirService;
    type Error = Infallible;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        Ok(service::ServeDirService(self.inner.clone()))
    }
}

mod service {
    use http_file::{ServeDir, ServeError};

    use crate::{
        body::ResponseBody,
        context::WebContext,
        error::{BadRequest, Error, Internal, MatchError, MethodNotAllowed, RouterError},
        http::{Method, StatusCode, WebResponse},
        service::Service,
    };

    pub struct ServeDirService(pub(super) ServeDir);

    impl<'r, C, B> Service<WebContext<'r, C, B>> for ServeDirService {
        type Response = WebResponse;
        type Error = RouterError<Error<C>>;

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
                        RouterError::NotAllowed(MethodNotAllowed(vec![Method::GET, Method::HEAD]))
                    }
                    ServeError::Io(_) => RouterError::Service(Error::from_service(Internal)),
                    _ => RouterError::Service(Error::from_service(BadRequest)),
                }),
            }
        }
    }
}
