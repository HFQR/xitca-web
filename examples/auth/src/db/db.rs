use std::{future::IntoFuture, sync::Arc};
use xitca_postgres::{Client, Config, Postgres, error::Error};

/// A thread-safe wrapper for the PostgreSQL database client.
/// Using Arc (Atomic Reference Counting) ensures the client can be
/// safely shared across multiple threads/tasks without re-connecting.
#[derive(Clone)]
pub struct DbClient {
    client: Arc<Client>,
}

impl DbClient {
    /// Creates a new database connection and starts the background driver.
    pub async fn new(database_url: &str) -> Result<Self, Error> {
        // Parse the connection string into a structured configuration
        let config = Config::try_from(database_url)?;

        // Establish connection. This returns both the 'Client' (to execute queries)
        // and the 'Driver' (which handles the actual network communication).
        let (client, driver) = Postgres::new(config).connect().await?;

        // CRITICAL: The driver must run in the background to maintain the connection.
        // We spawn it as a separate asynchronous task so it doesn't block the main flow.
        tokio::spawn(driver.into_future());

        Ok(Self {
            client: Arc::new(client),
        })
    }

    /// Provides a reference to the inner PostgreSQL client.
    /// Use this in your handlers to execute SQL queries.
    pub fn client(&self) -> &Client {
        &self.client
    }
}
