//! example for implementing pooling with xitca-postgres

use std::future::{Future, IntoFuture};

// use bb8 as connection pool
use bb8::{ManageConnection, Pool};
use xitca_postgres::{
    dev::{AsParams, ClientBorrowMut, Encode, Prepare, Query, Response},
    error::{DriverDown, Error},
    transaction::Transaction,
    types::{Oid, Type},
    AsyncLendingIterator, Client, Config, Postgres,
};

// type alias for reduce type naming complexity
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// a pool manager type containing necessary information for constructing a xitca-postgres connection
pub struct PoolManager {
    config: Config,
}

// implement manager trait for our pool manager so it can hook into bb8.
impl ManageConnection for PoolManager {
    type Connection = PoolConnection;
    type Error = Error;

    // logic where a new connection is created
    fn connect<'s, 'f>(&'s self) -> BoxFuture<'f, Result<Self::Connection, Self::Error>>
    where
        's: 'f,
    {
        Box::pin(async {
            let (conn, driver) = Postgres::new(self.config.clone()).connect().await?;
            tokio::spawn(driver.into_future());
            Ok(PoolConnection { conn })
        })
    }

    // logic where connection validation is checked. usually it's a sql query to database act like a ping.
    // but as this being an example we simply check if the connection is gone from local pov.
    fn is_valid<'s, 'c, 'f>(&'s self, conn: &'c mut Self::Connection) -> BoxFuture<'f, Result<(), Self::Error>>
    where
        's: 'f,
        'c: 'f,
    {
        Box::pin(async {
            if conn.conn.closed() {
                Err(DriverDown.into())
            } else {
                Ok(())
            }
        })
    }

    // like the is_valid method but at different lifetime cycle.
    // is_valid is after connection popping pool while has_broken is before connection pushed back to pool.
    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        conn.conn.closed()
    }
}

// a new type around xitca-postgres Client.
// addition state and logic can be attached to it but in this case it just forwards everything to
// the inner Client type.
pub struct PoolConnection {
    conn: Client,
}

// implementing following traits would enable the Client new type to be used as a generic client type
// with xitca_postgres::Transaction for transaction functionalities:

// trait for how a statement is prepared.
impl Prepare for PoolConnection {
    fn _get_type(&self, oid: Oid) -> BoxFuture<'_, Result<Type, Error>> {
        self.conn._get_type(oid)
    }
}

// trait for how a query is sent.
impl Query for PoolConnection {
    fn _send_encode_query<'a, S, I>(&self, stmt: S, params: I) -> Result<(S::Output<'a>, Response), Error>
    where
        S: Encode + 'a,
        I: AsParams,
    {
        self.conn._send_encode_query(stmt, params)
    }
}

// trait for confirming a mutable reference of Client can be obtained.
// transaction must have exclusive reference to Client through out it's lifecycle.
impl ClientBorrowMut for PoolConnection {
    fn _borrow_mut(&mut self) -> &mut Client {
        &mut self.conn
    }
}

impl PoolConnection {
    // with above traits implements you can begin a transaction with your client new type
    pub async fn transaction(&mut self) -> Result<Transaction<Self>, Error> {
        Transaction::<Self>::builder().begin(self).await
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // construct manager
    let config = Config::try_from("postgres://postgres:postgres@localhost:5432")?;
    let manager = PoolManager { config };

    // build bb8 connection pool with manager
    let pool = Pool::builder().build(manager).await?;

    // obtain connection from pool and query with it
    let mut conn = pool.get().await?;

    // you can forward query to xitca-postgres's client completely.
    let transaction = conn.conn.transaction().await?;
    let mut res = transaction.query("SELECT 1", &[])?;
    let row = res.try_next().await?.ok_or("row not found")?;
    assert_eq!(Some("1"), row.get(0));
    transaction.rollback().await?;

    // or use the new type definition of your pool connection for additional state and functionalities your
    // connection type could offer
    let transaction = conn.transaction().await?;
    let mut res = transaction.query("SELECT 1", &[])?;
    let row = res.try_next().await?.ok_or("row not found")?;
    assert_eq!(Some("1"), row.get(0));
    transaction.commit().await?;

    Ok(())
}
