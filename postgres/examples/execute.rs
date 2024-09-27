//! example of using Execute trait to expand functionality of xitca-postgres

use std::{
    future::{Future, IntoFuture},
    pin::Pin,
};

use xitca_postgres::{types::Type, Client, Error, Execute, Postgres, RowStream, Statement};

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

    // a new type containing necessary information for prepare statement and query
    struct PrepareAndExecute<'a> {
        stmt: &'a str,
        types: &'a [Type],
    }

    // implement Execute trait
    impl<'p, 'c> Execute<'c, Client> for PrepareAndExecute<'p>
    where
        // in execute method both PrepareAndExecute<'p> and &'c Client are moved into async block
        // and we use c as the output lifetime in Execute::ExecuteFuture's boxed async block.
        // adding this means PrepareAndExecute<'p> can live as long as &'c Client and both of them
        // will live until Execute::ExecuteFuture is resolved.
        'p: 'c,
    {
        // the new execute future we are returning is a boxed async code bock. the 'c lifetime annotation is
        // to tell the compiler the async block referencing &'c Client can live as long as 'c is alive
        type ExecuteFuture = Pin<Box<dyn Future<Output = Result<u64, Error>> + Send + 'c>>;

        // not related to this example.
        type RowStream = RowStream<'p>;

        fn execute(self, cli: &'c Client) -> Self::ExecuteFuture {
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

        fn query(self, _: &'c Client) -> Result<Self::RowStream, Error> {
            todo!("not related to this example")
        }

        // blocking version execute method. it's much simpler to implement than it's async variant.
        fn execute_blocking(self, cli: &Client) -> Result<u64, Error> {
            Statement::named(self.stmt, self.types)
                .execute_blocking(cli)?
                .execute_blocking(cli)
        }
    }

    // use the new type to execute a query.
    let row_affected = PrepareAndExecute {
        stmt: "SELECT 1",
        types: &[],
    }
    .execute(&cli)
    .await?;
    assert_eq!(row_affected, 1);

    Ok(())
}
