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

use super::{Error, error_from_service};

error_from_service!(MatchError);

impl<'r, C, B> Service<WebContext<'r, C, B>> for MatchError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        #[cfg(feature = "grpc")]
        if is_grpc_request(&ctx) {
            return super::grpc::GrpcError::new(super::grpc::GrpcStatus::Unimplemented, "method not found")
                .call(ctx)
                .await;
        }

        let mut res = ctx.into_response(ResponseBody::empty());
        *res.status_mut() = StatusCode::NOT_FOUND;
        Ok(res)
    }
}

error_from_service!(MethodNotAllowed);

impl<'r, C, B> Service<WebContext<'r, C, B>> for MethodNotAllowed {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        #[cfg(feature = "grpc")]
        if is_grpc_request(&ctx) {
            return super::grpc::GrpcError::new(super::grpc::GrpcStatus::Unimplemented, "method not allowed")
                .call(ctx)
                .await;
        }

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

#[cfg(feature = "grpc")]
fn is_grpc_request<C, B>(ctx: &WebContext<'_, C, B>) -> bool {
    use crate::http::{const_header_value::GRPC, header::CONTENT_TYPE};
    ctx.req()
        .headers()
        .get(CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(GRPC.as_bytes()))
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
