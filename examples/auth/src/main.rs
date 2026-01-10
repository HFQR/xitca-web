mod db;
mod routes;
mod utils;

use crate::{
    db::DbClient,
    routes::{login::login, register::register},
    utils::structs::ApiResponse,
};
use dotenvy::dotenv;
use xitca_web::{
    App, WebContext,
    error::Error,
    handler::{Responder, handler_service, json::Json},
    http::WebResponse,
    middleware::{compress::Compress, rate_limit::RateLimit},
    route::post,
    service::Service,
};

/// Global application state shared across all handlers.
#[derive(Clone)]
pub struct AppState {
    db_client: DbClient,
}

/// Global error handler middleware.
async fn error_handler<S, C>(service: &S, mut ctx: WebContext<'_, C>) -> Result<WebResponse, Error>
where
    C: 'static,
    S: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Error>,
{
    match service.call(ctx.reborrow()).await {
        Ok(res) => Ok(res),
        Err(e) => {
            // print and convert error to http response
            eprintln!("{e}");

            let res = e.call(ctx.reborrow()).await?;

            // override response body to json object.
            (
                res,
                Json(ApiResponse::<()> {
                    success: false,
                    message: e.to_string(),
                    data: None,
                }),
            )
                .respond(ctx)
                .await
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
    let db_client = DbClient::new(&database_url).await.map_err(std::io::Error::other)?;

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
        .serve()
        .bind("localhost:8080")?
        .run()
        .await
}
