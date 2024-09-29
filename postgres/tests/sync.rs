use core::future::IntoFuture;

use xitca_postgres::{
    error::{DbError, SqlState},
    statement::Statement,
    types::Type,
    Client, Execute, Postgres,
};

fn connect() -> Client {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (cli, drv) = rt
        .block_on(Postgres::new("postgres://postgres:postgres@localhost:5432").connect())
        .unwrap();
    std::thread::spawn(move || rt.block_on(drv.into_future()));
    cli
}

#[test]
fn query_unnamed() {
    let cli = connect();
    "CREATE TEMPORARY TABLE foo (name TEXT, age INT);"
        .execute_blocking(&cli)
        .unwrap();

    let mut stream = Statement::unnamed(
        "INSERT INTO foo (name, age) VALUES ($1, $2), ($3, $4), ($5, $6) returning name, age",
        &[Type::TEXT, Type::INT4, Type::TEXT, Type::INT4, Type::TEXT, Type::INT4],
    )
    .bind_dyn(&[&"alice", &20i32, &"bob", &30i32, &"carol", &40i32])
    .query(&cli)
    .unwrap()
    .into_iter();

    let row = stream.next().unwrap().unwrap();
    assert_eq!(row.get::<&str>("name"), "alice");
    assert_eq!(row.get::<i32>(1), 20);

    let row = stream.next().unwrap().unwrap();
    assert_eq!(row.get::<&str>(0), "bob");
    assert_eq!(row.get::<i32>("age"), 30);

    let row = stream.next().unwrap().unwrap();
    assert_eq!(row.get::<&str>("name"), "carol");
    assert_eq!(row.get::<i32>("age"), 40);

    assert!(stream.next().is_none());
}

#[test]
fn cancel_query_blocking() {
    let cli = connect();

    let cancel_token = cli.cancel_token();

    let sleep = std::thread::spawn(move || "SELECT pg_sleep(10)".execute_blocking(&cli));

    std::thread::sleep(std::time::Duration::from_secs(3));

    cancel_token.query_cancel_blocking().unwrap();

    let e = sleep.join().unwrap().unwrap_err();

    let e = e.downcast_ref::<DbError>().unwrap();
    assert_eq!(e.code(), &SqlState::QUERY_CANCELED);
}
