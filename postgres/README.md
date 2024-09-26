# async postgresql client integrated with [xitca-web](https://github.com/HFQR/xitca-web). Inspired and depend on [rust-postgres](https://github.com/sfackler/rust-postgres)

## Compare to tokio-postgres
- Pros
    - async/await native
    - less heap allocation on query
    - zero copy row data parsing
    - quic transport layer for lossy database connection
- Cons
    - no built in back pressure mechanism. possible to cause excessive memory usage if database requests are unbounded or not rate limited
    - expose lifetime in public type params.(hard to return from function or contained in new types)

## Features
- Pipelining:
    - offer both "implicit" and explicit API. 
    - support for more relaxed pipeline.

- SSL/TLS support:

    - powered by `rustls`
    - QUIC transport layer: offer transparent QUIC transport layer and proxy for lossy remote database connection

- Connection Pool:
    - built in connection pool with pipelining support enabled

## Quick Start
```rust
use std::future::IntoFuture;

use xitca_postgres::{iter::AsyncLendingIterator, types::Type, Execute, Postgres};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // connect to database and spawn driver as tokio task.
    let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
        .connect()
        .await?;

    tokio::spawn(drv.into_future());

    // execute raw sql queries with client type. multiple sql queries are separated by ;
    "CREATE TEMPORARY TABLE foo (id SERIAL, name TEXT);
    INSERT INTO foo (name) VALUES ('alice'), ('bob'), ('charlie');"
        .execute(&cli)
        .await?;

    // prepare statement with type parameters. multiple params can be annotate as $1, $2 .. $n inside sql string as
    // it's value identifier.
    //
    // following the sql query is a slice of potential postgres type for each param in the same order. the types are
    // optional and if not provided types will be inferred from database.
    //
    // in this case we declare for $1 param's value has to be TEXT type. it's according Rust type can be String/&str
    // or other types that can represent a text string
    let stmt = cli.prepare("INSERT INTO foo (name) VALUES ($1)", &[Type::TEXT]).await?;

    // bind the prepared statement to parameter values. the value's Rust type representation must match the postgres 
    // Type we declared.
    // execute the bind and return number of rows affected by the sql query on success.
    let rows_affected = stmt.bind(["david"]).execute(&cli).await?;
    assert_eq!(rows_affected, 1);

    // prepare another statement with type parameters.
    //
    // in this case we declare for $1 param's value has to be INT4 type. it's according Rust type representation is i32 
    // and $2 is TEXT type mentioned before.
    let stmt = cli
        .prepare(
            "SELECT id, name FROM foo WHERE id = $1 AND name = $2",
            &[Type::INT4, Type::TEXT],
        )
        .await?;

    // bind the prepared statement to parameter values it declared.
    // when parameters are different Rust types it's suggested to use dynamic binding as following
    // query with the bind and get an async streaming for database rows on success
    let mut stream = stmt.bind_dyn(&[&1i32, &"alice"]).query(&cli)?;

    // use async iterator to visit rows
    let row = stream.try_next().await?.ok_or("no row found")?;

    // parse column value from row to rust types
    let id = row.get::<i32>(0); // column's numeric index can be used for slicing the row and parse column.
    assert_eq!(id, 1);
    let name = row.get::<&str>("name"); // column's string name index can be used for parsing too.
    assert_eq!(name, "alice");
    // when all rows are visited the stream would yield Ok(None) to indicate it has ended.
    assert!(stream.try_next().await?.is_none());

    // like execute method. query can be used with raw sql string.
    let mut stream = "SELECT id, name FROM foo WHERE name = 'david'".query(&cli)?;
    let row = stream.try_next().await?.ok_or("no row found")?;

    // unlike query with prepared statement. raw sql query would return rows that can only be parsed to Rust string types.
    let id = row.get(0).ok_or("no id found")?;
    assert_eq!(id, "4");
    let name = row.get("name").ok_or("no name found")?;
    assert_eq!(name, "david");

    Ok(())
}
```

## Synchronous API
`xitca_postgres::Client` can run outside of tokio async runtime and using blocking API to interact with database 
```rust
use xitca_postgres::{Client, Error, Execute};

fn query(client: &Client) -> Result<(), Error> {
    // execute sql query with blocking api
    "SELECT 1".execute_blocking(client)?;

    let stream = "SELECT 1".query(client)?;

    // use sync iterator to visit streaming rows
    for item in stream {
        let row = item?;
        let one = row.get(0).expect("database must return 1");
        assert_eq!(one, "1")
    }

    Ok(())
}
```