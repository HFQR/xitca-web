//! example of using Execute trait to expand functionality of xitca-postgres

use std::{
    future::{Future, IntoFuture},
    pin::Pin,
};

use futures::stream::{Stream, TryStreamExt};
use xitca_postgres::{row::RowOwned, types::Type, Client, Error, Execute, Postgres, RowStreamOwned, Statement};

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
    impl<'p, 'c> Execute<'c, Client> for PrepareAndExecute<'p>
    where
        // in execute methods both PrepareAndExecute<'p> and &'c Client are moved into async block
        // and we use c as the output lifetime in Execute::ExecuteOutput's boxed async block.
        // adding this means PrepareAndExecute<'p> can live as long as &'c Client and both of them
        // will live until Execute::ExecuteOutput is resolved.
        // the same reason goes for Execute::QueryOutput.
        'p: 'c,
    {
        // new execute future we are returning is a boxed async code bock. the 'c lifetime annotation is
        // to tell the compiler the async block referencing &'c Client can live as long as 'c is alive
        type ExecuteOutput = Pin<Box<dyn Future<Output = Result<u64, Error>> + Send + 'c>>;

        // like the execute but output an async stream iterator that produces database rows.
        type QueryOutput = Pin<Box<dyn Stream<Item = Result<RowOwned, Error>> + Send + 'c>>;

        fn execute(self, cli: &'c Client) -> Self::ExecuteOutput {
            // move PrepareAndExecute<'p> and &'c Client into an async block.
            // inside it we use the state and client to prepare a statement and execute it.
            Box::pin(async move {
                Statement::named(self.stmt, self.types)
                    .execute(cli)
                    .await?
                    .execute(cli)
                    .await
            })
        }

        fn query(self, cli: &'c Client) -> Self::QueryOutput {
            // async stream macro is used to move prepare statement and query into a single
            // streaming type.
            Box::pin(async_stream::try_stream! {
                // prepare statement and query for async iterator of rows
                let stmt = Statement::named(self.stmt, self.types)
                    .execute(cli)
                    .await?;
                let stream = stmt.query(cli)?;

                // async stream macro does not support lending iterator types and we convert
                // row stream to an owned version where it does not contain references.
                let mut stream = RowStreamOwned::from(stream);

                // futures::stream::TryStreamExt trait is utilized here to produce database rows.
                while let Some(item) = stream.try_next().await? {
                    yield item;
                }
            })
        }

        // blocking version execute method. it's much simpler to implement than it's async variant.
        fn execute_blocking(self, cli: &Client) -> Result<u64, Error> {
            Statement::named(self.stmt, self.types)
                .execute_blocking(cli)?
                .execute_blocking(cli)
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
    // use TryStreamExt trait methods to visit rows and collect column index 0 to integers.
    .map_ok(|row| row.get::<i32>(0))
    .try_collect::<Vec<_>>()
    .await?;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], 1);

    Ok(())
}
