use core::convert::Infallible;

use std::{borrow::Cow, path::PathBuf};

use http_file::{ServeDir as _ServeDir, ServeError};
use xitca_http::{
    http::StatusCode,
    util::service::router::{RouterGen, RouterMapErr},
};

use crate::{
    body::ResponseBody,
    bytes::Bytes,
    context::WebContext,
    dev::service::Service,
    error::{BadRequest, Error, Internal, MatchError, MethodNotAllowed},
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
    type Route<R> = RouterMapErr<R>;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        let mut path = String::from(prefix);
        if path.ends_with('/') {
            path.pop();
        }

        path.push_str("/*p");

        Cow::Owned(path)
    }

    fn route_gen<R>(route: R) -> Self::Route<R> {
        RouterMapErr(route)
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
    type Error = Error<C>;

    async fn call(&self, req: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.0
            .serve(req.req())
            .await
            .map(|res| res.map(ResponseBody::box_stream))
            .map_err(Error::from_service)
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for ServeError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        match self {
            Self::NotFound => MatchError::NotFound.call(ctx).await,
            Self::MethodNotAllowed => MethodNotAllowed(vec![Method::GET, Method::HEAD]).call(ctx).await,
            Self::NotModified => {
                let mut res = ctx.into_response(Bytes::new());
                *res.status_mut() = StatusCode::NOT_MODIFIED;
                Ok(res)
            }
            // TODO: better error handling.
            Self::Io(_) => Internal.call(ctx).await,
            _ => BadRequest.call(ctx).await,
        }
    }
}
