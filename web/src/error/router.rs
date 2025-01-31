pub use xitca_http::util::service::{
    route::MethodNotAllowed,
    router::{MatchError, RouterError},
};

use core::convert::Infallible;

use crate::{
    WebContext,
    body::ResponseBody,
    http::{StatusCode, WebResponse, header::ALLOW},
    service::Service,
};

use super::{Error, blank_error_service, error_from_service};

error_from_service!(MatchError);
blank_error_service!(MatchError, StatusCode::NOT_FOUND);

error_from_service!(MethodNotAllowed);

impl<'r, C, B> Service<WebContext<'r, C, B>> for MethodNotAllowed {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(ResponseBody::empty());

        let allowed = self.allowed_methods();

        let len = allowed.iter().fold(0, |a, m| a + m.as_str().len() + 1);

        let mut methods = String::with_capacity(len);

        for method in allowed {
            methods.push_str(method.as_str());
            methods.push(',');
        }
        methods.pop();

        res.headers_mut().insert(ALLOW, methods.parse().unwrap());
        *res.status_mut() = StatusCode::METHOD_NOT_ALLOWED;

        Ok(res)
    }
}

impl<E> From<RouterError<E>> for Error
where
    E: Into<Self>,
{
    fn from(e: RouterError<E>) -> Self {
        match e {
            RouterError::Match(e) => e.into(),
            RouterError::NotAllowed(e) => e.into(),
            RouterError::Service(e) => e.into(),
        }
    }
}
