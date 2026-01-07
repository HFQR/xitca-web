use std::sync::Arc;
use xitca_postgres::{error::Error, pool::Pool};

/// Thread-safe PostgreSQL connection pool wrapper.
/// Uses Arc for safe sharing across async tasks.
#[derive(Clone)]
pub struct DbClient {
    pool: Arc<Pool>,
}

impl DbClient {
    /// Creates a new connection pool with the given database URL.
    /// Pool automatically manages connection lifecycle and reuse.
    pub async fn new(database_url: &str) -> Result<Self, Error> {
        let pool = Pool::builder(database_url).capacity(10).build()?;
        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// Returns a reference to the underlying connection pool.
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
}
