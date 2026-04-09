use core::convert::Infallible;

pub use http_grpc::{error::GrpcError, status::GrpcStatus};

use crate::{
    body::ResponseBody,
    context::WebContext,
    http::{WebResponse, const_header_value::GRPC, header::CONTENT_TYPE},
    service::Service,
};

use super::Error;

impl<'r, C, B> Service<WebContext<'r, C, B>> for GrpcError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        // gRPC "Trailers-Only" response: status goes in the response headers
        // (the single HEADERS frame with END_STREAM set).
        let mut res = ctx.into_response(ResponseBody::empty());
        res.headers_mut().insert(CONTENT_TYPE, GRPC);
        res.headers_mut().extend(self.trailers());
        Ok(res)
    }
}

impl From<GrpcError> for Error {
    fn from(e: GrpcError) -> Self {
        Self::from_service(e)
    }
}
