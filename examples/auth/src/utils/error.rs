use std::fmt;
use xitca_postgres::error::Error as PgError;
use xitca_web::{
    WebContext,
    error::Error as WebError,
    http::{Response, StatusCode},
    service::Service,
};

/// Custom error wrapper for PostgreSQL errors.
/// Allows automatic conversion to xitca-web's Error type.
#[derive(Debug)]
pub struct DbError(pub PgError);

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Database error: {}", self.0)
    }
}

impl std::error::Error for DbError {}

/// Convert DbError to xitca-web Error for middleware compatibility.
impl From<DbError> for WebError {
    fn from(e: DbError) -> Self {
        WebError::from(std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

/// Auto-convert PostgreSQL errors to DbError using ? operator.
impl From<PgError> for DbError {
    fn from(e: PgError) -> Self {
        DbError(e)
    }
}

/// Implement Service trait to allow DbError to be converted to HTTP response.
impl<C> Service<WebContext<'_, C>> for DbError {
    type Response = Response<String>;
    type Error = WebError;

    async fn call(&self, _ctx: WebContext<'_, C>) -> Result<Self::Response, Self::Error> {
        Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body("Database error occurred".to_string())
            .unwrap())
    }
}
