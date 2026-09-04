//! example of using Execute trait to expand functionality of xitca-postgres

use std::future::IntoFuture;

use xitca_postgres::{
    Client, Error, Execute, Postgres, RowStreamOwned, Statement, iter::AsyncLendingIteratorExt, types::Type,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // connect to database and spawn client driver.
    let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
        .connect()
        .await?;
    tokio::spawn(drv.into_future());

    // prepare a statement and then execute.
    let stmt = Statement::named("SELECT 1", &[]).execute(&cli).await?;
    let row_affected = stmt.execute(&cli).await?;
    assert_eq!(row_affected, 1);

    // it works but requires 2 loc and what if we want to capsulate it into a single function.
    // for this purpose we can do the following:

    // a new type containing necessary information for prepare statement and execute/query
    struct PrepareAndExecute<'a> {
        stmt: &'a str,
        types: &'a [Type],
    }

    // implement Execute trait
    impl<'p, 'c> Execute<&'c Client> for PrepareAndExecute<'p>
    where
        // both PrepareAndExecute<'p> and &'c Client are captured by the future the methods below
        // return. adding this means PrepareAndExecute<'p> lives as long as &'c Client and both of
        // them stay alive until that future resolves.
        'p: 'c,
    {
        // the associated types are the items produced. the future producing them is opaque so
        // there is no need to name it or to box it.
        type ExecuteOutput = u64;

        // like execute but produces an async stream iterator of database rows.
        type QueryOutput = RowStreamOwned;

        async fn execute(self, cli: &'c Client) -> Result<Self::ExecuteOutput, Error> {
            // prepare a statement and execute it with the client.
            Statement::named(self.stmt, self.types)
                .execute(cli)
                .await?
                .execute(cli)
                .await
        }

        async fn query(self, cli: &'c Client) -> Result<Self::QueryOutput, Error> {
            // prepare statement and query for async iterator of rows
            let stmt = Statement::named(self.stmt, self.types).execute(cli).await?;
            let stream = stmt.query(cli).await?;
            // convert borrowed stream to owned stream. as borrowed stream reference the statement
            // this function just produced.
            Ok(RowStreamOwned::from(stream))
        }
    }

    // use the new type to prepare and execute a statement.
    let row_affected = PrepareAndExecute {
        stmt: "SELECT 1",
        types: &[],
    }
    .execute(&cli)
    .await?;
    assert_eq!(row_affected, 1);

    // use the new type to prepare and query a statement
    let rows = PrepareAndExecute {
        stmt: "SELECT 1",
        types: &[],
    }
    .query(&cli)
    .await?
    // use async iterator methods to visit rows and collect column index 0 to integers.
    .map_ok(|row| row.get::<i32>(0))
    .try_collect::<Vec<_>>()
    .await?;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], 1);

    Ok(())
}
