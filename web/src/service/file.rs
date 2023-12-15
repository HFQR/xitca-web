use core::{convert::Infallible, fmt};

use std::{borrow::Cow, error, path::PathBuf};

use http_file::{ServeDir as _ServeDir, ServeError};
use xitca_http::{http::StatusCode, util::service::router::RouterGen};

use crate::{
    body::ResponseBody,
    bytes::Bytes,
    context::WebContext,
    dev::service::Service,
    error::{BadRequest, Error, Internal, MatchError, MethodNotAllowed, RouterError},
    http::{Method, WebResponse},
};

pub struct ServeDir {
    inner: _ServeDir,
}

impl ServeDir {
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

impl<Arg> Service<Arg> for ServeDir {
    type Response = ServeDirService;
    type Error = Infallible;

    async fn call(&self, _: Arg) -> Result<Self::Response, Self::Error> {
        Ok(ServeDirService(self.inner.clone()))
    }
}

pub struct ServeDirService(_ServeDir);

impl<'r, C, B> Service<WebContext<'r, C, B>> for ServeDirService {
    type Response = WebResponse;
    type Error = RouterError<Error<C>>;

    async fn call(&self, req: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.0
            .serve(req.req())
            .await
            .map(|res| res.map(ResponseBody::box_stream))
            .map_err(|e| match e {
                ServeError::NotFound => RouterError::Match(MatchError::NotFound),
                ServeError::MethodNotAllowed => {
                    RouterError::NotAllowed(MethodNotAllowed(vec![Method::GET, Method::HEAD]))
                }
                ServeError::NotModified => RouterError::Service(Error::from_service(NotModified)),
                ServeError::Io(_) => RouterError::Service(Error::from_service(Internal)),
                _ => RouterError::Service(Error::from_service(BadRequest)),
            })
    }
}

#[derive(Debug)]
pub struct NotModified;

impl fmt::Display for NotModified {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("File not Modified")
    }
}

impl error::Error for NotModified {}

impl<'r, C, B> Service<WebContext<'r, C, B>> for NotModified {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(Bytes::new());
        *res.status_mut() = StatusCode::NOT_MODIFIED;
        Ok(res)
    }
}
