use core::future::IntoFuture;

use xitca_postgres::{
    error::{DbError, SqlState},
    AsyncLendingIterator, Client, Postgres,
};

async fn connect(s: &str) -> Client {
    let (client, driver) = Postgres::new(s).connect().await.unwrap();
    tokio::spawn(driver.into_future());
    client
}

#[tokio::test]
async fn cancel_statement() {
    let cli = connect("postgres://postgres:postgres@localhost:5432").await;

    cli.execute_simple(
        "CREATE TEMPORARY TABLE foo (
            id SERIAL,
            name TEXT
        );

        INSERT INTO foo (name) VALUES ('alice'), ('bob'), ('charlie');",
    )
    .await
    .unwrap();

    let stmt = cli.prepare("SELECT id, name FROM foo ORDER BY id", &[]).await.unwrap();

    let stmt_raw = stmt.clone();

    drop(stmt);

    let mut stream = cli.query(&stmt_raw, &[]).unwrap();

    let e = stream.try_next().await.err().unwrap();

    let e = e.downcast_ref::<DbError>().unwrap();

    assert_eq!(e.code(), &SqlState::INVALID_SQL_STATEMENT_NAME);
}
