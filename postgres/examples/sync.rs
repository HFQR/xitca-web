//! example of running client in non async environment.

use std::future::IntoFuture;
use xitca_postgres::{ExecuteBlocking, Postgres, Statement, types::Type};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // prepare a tokio runtime for client's Driver.
    // can be shared with existing runtime if any.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;

    // start client and driver with help of tokio runtime.
    let (cli, drv) = rt.block_on(Postgres::new("postgres://postgres:postgres@localhost:5432").connect())?;

    // keep tokio running in a background thread by spawn driver inside
    let handle = std::thread::spawn(move || rt.block_on(drv.into_future()));

    {
        // execute sql with blocking api
        "CREATE TEMPORARY TABLE foo (id SERIAL, name TEXT)".execute_blocking(&cli)?;
        "INSERT INTO foo (name) VALUES ('alice'), ('bob'), ('charlie');".execute_blocking(&cli)?;

        // use blocking api to prepare statement
        let stmt = Statement::named("INSERT INTO foo (name) VALUES ($1)", &[Type::TEXT]).execute_blocking(&cli)?;
        // execute statement and obtain rows affected by insert statement.
        let row_affected = stmt.bind(["david"]).execute_blocking(&cli)?;
        assert_eq!(row_affected, 1);

        // retrieve the row just inserted
        let stmt = Statement::named(
            "SELECT id, name FROM foo WHERE id = $1 AND name = $2",
            &[Type::INT4, Type::TEXT],
        )
        .execute_blocking(&cli)?;
        // query api shares the same convention no matter the context.
        let stream = stmt.bind_dyn(&[&4i32, &"david"]).query_blocking(&cli)?;

        // async row stream implement IntoIterator trait to convert stream into a sync iterator.
        for item in stream {
            let row = item?;
            assert_eq!(row.get::<i32>("id"), 4);
            assert_eq!(row.get::<&str>("name"), "david");
        }
    }

    // drop client after usage
    drop(cli);

    // client drop signal Driver to shutdown. which would result in the shutdown of tokio runtime
    // and the thread it runs on will be able to be joined.
    handle.join().unwrap()?;

    Ok(())
}
