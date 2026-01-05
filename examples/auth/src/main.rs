mod db;
mod routes;
mod utils;

use dotenvy::dotenv;
use xitca_web::{
    App,
    handler::handler_service,
    middleware::{compress::Compress, decompress::Decompress, rate_limit::RateLimit},
    route::post,
};

use crate::{
    db::db::DbClient,
    routes::{login::login, register::register},
};

/// Global application state shared across all request handlers.
/// Wrapping the DbClient here allows handlers to access the database pool.
#[derive(Clone)]
pub struct AppState {
    db_client: DbClient,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Load environment variables from .env file (e.g., DATABASE_URL)
    dotenv().ok();

    // Retrieve database connection string from environment
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    // Initialize the database connection pool
    let db_client = DbClient::new(&database_url)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    println!("Database client initialized successfully");

    // Create the shared state instance
    let state = AppState { db_client };

    App::new()
        .with_state(state) // Inject the shared state into the app context
        .at("/login", post(handler_service(login)))
        .at("/register", post(handler_service(register)))
        .enclosed(RateLimit::per_minute(60))
        .enclosed(Compress)
        .enclosed(Decompress)
        .serve()
        .bind("localhost:8080")?
        .run()
        .wait()
}
