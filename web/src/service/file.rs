use core::convert::Infallible;

use std::{borrow::Cow, path::PathBuf};

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

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        match self.0.serve(ctx.req()).await {
            Ok(res) => Ok(res.map(ResponseBody::box_stream)),
            Err(ServeError::NotModified) => {
                let mut res = ctx.into_response(Bytes::new());
                *res.status_mut() = StatusCode::NOT_MODIFIED;
                Ok(res)
            }
            Err(e) => Err(match e {
                ServeError::NotFound => RouterError::Match(MatchError::NotFound),
                ServeError::MethodNotAllowed => {
                    RouterError::NotAllowed(MethodNotAllowed(vec![Method::GET, Method::HEAD]))
                }
                ServeError::Io(_) => RouterError::Service(Error::from_service(Internal)),
                _ => RouterError::Service(Error::from_service(BadRequest)),
            }),
        }
    }
}
