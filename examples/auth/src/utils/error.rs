use thiserror::Error;
use xitca_web::{
    WebContext,
    error::Error as WebError,
    handler::Responder,
    http::{StatusCode, WebResponse},
    service::Service,
};

/// New type for postgres error so we can implement customized http response generating trait
#[derive(Debug, Error)]
#[error(transparent)]
pub struct DbError(#[from] pub xitca_postgres::error::Error);

/// Conversion helper between db error and xitca-web error object
impl From<DbError> for WebError {
    fn from(e: DbError) -> Self {
        WebError::from_service(e)
    }
}

/// Implement Service trait to allow DbError to be converted to HTTP response.
impl<C> Service<WebContext<'_, C>> for DbError {
    type Response = WebResponse;
    type Error = std::convert::Infallible;

    async fn call(&self, ctx: WebContext<'_, C>) -> Result<Self::Response, Self::Error> {
        let res = StatusCode::INTERNAL_SERVER_ERROR
            .respond(ctx)
            .await
            .expect("predetermined response generator pattern can not fail");
        Ok(res)
    }
}

/// Collection of error happens during authentication
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("user not found")]
    NotFound,
    #[error("operation not permitted")]
    PermissionDenied,
}

impl From<AuthError> for WebError {
    fn from(e: AuthError) -> Self {
        WebError::from_service(e)
    }
}

impl<C> Service<WebContext<'_, C>> for AuthError {
    type Response = WebResponse;
    type Error = std::convert::Infallible;

    async fn call(&self, ctx: WebContext<'_, C>) -> Result<Self::Response, Self::Error> {
        StatusCode::BAD_REQUEST.call(ctx).await
    }
}

/// New type for validation error so we can implement customized http response generating trait
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ValidationError(#[from] pub validator::ValidationErrors);

impl From<ValidationError> for WebError {
    fn from(e: ValidationError) -> Self {
        WebError::from_service(e)
    }
}

impl<C> Service<WebContext<'_, C>> for ValidationError {
    type Response = WebResponse;
    type Error = std::convert::Infallible;

    async fn call(&self, ctx: WebContext<'_, C>) -> Result<Self::Response, Self::Error> {
        StatusCode::BAD_REQUEST.call(ctx).await
    }
}
