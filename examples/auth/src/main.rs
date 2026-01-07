mod db;
mod routes;
mod utils;

use crate::{
    db::db::DbClient,
    routes::{login::login, register::register},
    utils::structs::ApiResponse,
};
use dotenvy::dotenv;
use xitca_web::{
    App, WebContext,
    error::Error,
    handler::handler_service,
    http::{Response, StatusCode, WebResponse},
    middleware::{compress::Compress, decompress::Decompress, rate_limit::RateLimit},
    route::post,
    service::Service,
};

/// Global application state shared across all handlers.
#[derive(Clone)]
pub struct AppState {
    db_client: DbClient,
}

/// Global error handler middleware.
/// Converts all errors to appropriate HTTP responses with correct status codes.
async fn error_handler<S, C, B>(
    service: &S,
    mut ctx: WebContext<'_, C, B>,
) -> Result<WebResponse, Error>
where
    C: 'static,
    B: 'static,
    S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Error>,
{
    match service.call(ctx.reborrow()).await {
        Ok(res) => Ok(res),
        Err(e) => {
            let error_msg = e.to_string();

            // Map error types to appropriate HTTP status codes.
            let status = if error_msg.contains("InvalidInput") || error_msg.contains("Validation") {
                StatusCode::BAD_REQUEST
            } else if error_msg.contains("NotFound") {
                StatusCode::NOT_FOUND
            } else if error_msg.contains("PermissionDenied")
                || error_msg.contains("Invalid email or password")
            {
                StatusCode::UNAUTHORIZED
            } else if error_msg.contains("AlreadyExists")
                || error_msg.contains("already registered")
            {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };

            // If error message is already JSON, use it directly.
            // Otherwise, wrap in ApiResponse structure.
            let json_body = if error_msg.starts_with('{') && error_msg.ends_with('}') {
                error_msg
            } else {
                let error_response = ApiResponse::<()> {
                    success: false,
                    message: error_msg,
                    data: None,
                };
                serde_json::to_string(&error_response).unwrap_or_else(|_| {
                    r#"{"success":false,"message":"Internal error"}"#.to_string()
                })
            };

            Ok(Response::builder()
                .status(status)
                .header("content-type", "application/json")
                .body(json_body.into())
                .unwrap()
                .into())
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Load environment variables from .env file.
    dotenv().ok();

    // Get database URL from environment.
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    // Initialize connection pool.
    let db_client = DbClient::new(&database_url)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    println!("✓ Server running on http://localhost:8080");

    let state = AppState { db_client };

    // Build application with middleware stack.
    App::new()
        .with_state(state)
        .at("/register", post(handler_service(register)))
        .at("/login", post(handler_service(login)))
        .enclosed_fn(error_handler) // Global error handling
        .enclosed(RateLimit::per_minute(60)) // Rate limiting: 60 requests/minute
        .enclosed(Compress) // Response compression
        .enclosed(Decompress) // Request decompression
        .serve()
        .bind("localhost:8080")?
        .run()
        .await
}
