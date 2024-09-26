//! example of implementing proc macro to expand functionality of xitca-postgres

use std::future::IntoFuture;

use xitca_postgres::{iter::AsyncLendingIterator, Execute, Postgres};
// an example proc macro
use xitca_postgres_codegen::sql;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
        .connect()
        .await?;
    tokio::spawn(drv.into_future());

    "CREATE TEMPORARY TABLE foo (id SERIAL, name TEXT)"
        .execute(&cli)
        .await?;
    "INSERT INTO foo (name) VALUES ('alice'), ('bob'), ('charlie');"
        .execute(&cli)
        .await?;

    // this macro is expand into xitca_postgres::statement::Statement::unnamed
    // it's also possible to utilize xitca-postgres's Execute and Encode traits for more customizable macro usage
    let mut stream = sql!("SELECT * FROM foo WHERE id = $1 AND name = $2", &1i32, &"alice").query(&cli)?;

    let row = stream.try_next().await?.ok_or("row not found")?;

    assert_eq!(row.get::<&str>("name"), "alice");

    Ok(())
}
