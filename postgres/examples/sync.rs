//! example of running client in non async environment.

use std::future::{pending, IntoFuture};
use xitca_postgres::{types::Type, Error, Postgres};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // prepare a tokio runtime for client's Driver.
    // can be shared with existing runtime if any.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;

    // start client and spawn driver inside runtime.
    let cli = rt.block_on(async {
        let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
            .connect()
            .await?;
        tokio::spawn(drv.into_future());
        Ok::<_, Error>(cli)
    })?;

    // keep tokio running in a background thread
    std::thread::spawn(move || rt.block_on(pending::<()>()));

    // execute api needs a special suffix to be called inside sync context.
    cli.execute(
        "CREATE TEMPORARY TABLE foo (id SERIAL, name TEXT);
        INSERT INTO foo (name) VALUES ('alice'), ('bob'), ('charlie');",
    )
    .wait()?;

    // use blocking api to prepare statement
    let stmt = cli.prepare_blocking("INSERT INTO foo (name) VALUES ($1)", &[Type::TEXT])?;
    let bind = stmt.bind(["david"]);

    // execute statement
    let row_affected = cli.execute(bind).wait()?;
    assert_eq!(row_affected, 1);

    // retrieve the row just inserted
    let stmt = cli.prepare_blocking(
        "SELECT id, name FROM foo WHERE id = $1 AND name = $2",
        &[Type::INT4, Type::TEXT],
    )?;
    let bind = stmt.bind_dyn(&[&4i32, &"david"]);
    let stream = cli.query(bind)?;

    // convert async row stream to iterator and visit rows
    for item in stream.into_iter() {
        let row = item?;
        assert_eq!(row.get::<i32>("id"), 4);
        assert_eq!(row.get::<&str>("name"), "david");
    }

    Ok(())
}
