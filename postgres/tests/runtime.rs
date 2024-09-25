use core::future::IntoFuture;

use xitca_postgres::{
    error::{DbError, SqlState},
    iter::AsyncLendingIterator,
    types::Type,
    Client, Postgres,
};

async fn connect(s: &str) -> Client {
    let (client, driver) = Postgres::new(s).connect().await.unwrap();
    tokio::spawn(driver.into_future());
    client
}

async fn smoke_test(s: &str) {
    let client = connect(s).await;

    let stmt = client.prepare("SELECT $1::INT", &[]).await.unwrap();
    let mut stream = client.query(stmt.bind([1i32])).unwrap();
    let row = stream.try_next().await.unwrap().unwrap();
    assert_eq!(row.get::<i32>(0), 1i32);
}

// #[tokio::test]
// #[ignore] // FIXME doesn't work with our docker-based tests :(
// async fn unix_socket() {
//     smoke_test("host=/var/run/postgresql port=5432 user=postgres").await;
// }

#[tokio::test]
async fn tcp() {
    smoke_test("host=localhost port=5432 user=postgres password=postgres").await;
}

#[tokio::test]
async fn multiple_hosts_one_port() {
    smoke_test("host=foobar.invalid,localhost port=5432 user=postgres password=postgres").await;
}

#[tokio::test]
async fn multiple_hosts_multiple_ports() {
    smoke_test("host=foobar.invalid,localhost port=5432,5432 user=postgres password=postgres").await;
}

// #[tokio::test]
// async fn wrong_port_count() {
//     Postgres::new("host=localhost port=5432,5432 user=postgres")
//         .connect()
//         .await
//         .err()
//         .unwrap();
// }

#[tokio::test]
async fn target_session_attrs_ok() {
    smoke_test("host=localhost port=5432 user=postgres password=postgres target_session_attrs=read-write").await;
}

#[tokio::test]
async fn target_session_attrs_err() {
    Postgres::new(
        "host=localhost port=5432 user=postgres target_session_attrs=read-write
         options='-c default_transaction_read_only=on'",
    )
    .connect()
    .await
    .err()
    .unwrap();
}

#[tokio::test]
async fn host_only_ok() {
    let _ = Postgres::new("host=localhost port=5432 user=postgres dbname=postgres password=postgres")
        .connect()
        .await
        .unwrap();
}

// #[tokio::test]
// async fn hostaddr_only_ok() {
//     let _ = Postgres::new(
//         "hostaddr=127.0.0.1 port=5432 user=postgres dbname=postgres password=postgres"
//     )
//     .connect()
//     .await
//     .unwrap();
// }

// #[tokio::test]
// async fn hostaddr_and_host_ok() {
//     let _ = Postgres::new(
//         "hostaddr=127.0.0.1 host=localhost port=5432 user=postgres dbname=postgres password=postgres"
//     )
//     .connect()
//     .await
//     .unwrap();
// }

#[tokio::test]
async fn hostaddr_host_mismatch() {
    let _ = Postgres::new(
        "hostaddr=127.0.0.1,127.0.0.2 host=localhost port=5432 user=postgres dbname=postgres password=postgres",
    )
    .connect()
    .await
    .err()
    .unwrap();
}

#[tokio::test]
async fn hostaddr_host_both_missing() {
    let _ = Postgres::new("port=5432 user=postgres dbname=postgres password=postgres")
        .connect()
        .await
        .err()
        .unwrap();
}

#[tokio::test]
async fn cancel_query() {
    let client = connect("host=localhost port=5432 user=postgres password=postgres").await;

    let cancel_token = client.cancel_token();

    let sleep = client.execute("SELECT pg_sleep(10)");

    tokio::task::yield_now().await;

    cancel_token.query_cancel().await.unwrap();

    let e = sleep.await.unwrap_err();

    let e = e.downcast_ref::<DbError>().unwrap();
    assert_eq!(e.code(), &SqlState::QUERY_CANCELED);
}

#[tokio::test]
async fn client_shutdown() {
    let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
        .connect()
        .await
        .unwrap();

    let handle = tokio::spawn(drv.into_future());

    drop(cli);

    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn driver_shutdown() {
    let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
        .connect()
        .await
        .unwrap();

    let handle = tokio::spawn(drv.into_future());

    cli.execute("SELECT 1").await.unwrap();

    // yield to execute the abort of driver task. this depends on single thread
    // tokio runtime's behavior specifically.
    handle.abort();
    tokio::task::yield_now().await;

    let e = cli.execute("SELECT 1").await.err().unwrap();
    assert!(e.is_driver_down());
}

#[tokio::test]
async fn query_portal() {
    let mut client = connect("postgres://postgres:postgres@localhost:5432").await;

    client
        .execute(
            "CREATE TEMPORARY TABLE foo (
                id SERIAL,
                name TEXT
            );

            INSERT INTO foo (name) VALUES ('alice'), ('bob'), ('charlie');",
        )
        .await
        .unwrap();

    let transaction = client.transaction().await.unwrap();

    let stmt = transaction
        .prepare("SELECT id, name FROM foo ORDER BY id", &[])
        .await
        .unwrap()
        .leak();

    let portal = transaction.bind(&stmt, &[]).await.unwrap();
    let mut stream1 = portal.query_portal(2).unwrap();
    let mut stream2 = portal.query_portal(2).unwrap();
    let mut stream3 = portal.query_portal(2).unwrap();

    let row = stream1.try_next().await.unwrap().unwrap();
    assert_eq!(row.get::<i32>(0), 1);
    assert_eq!(row.get::<&str>(1), "alice");
    let row = stream1.try_next().await.unwrap().unwrap();
    assert_eq!(row.get::<i32>(0), 2);
    assert_eq!(row.get::<&str>(1), "bob");
    assert!(stream1.try_next().await.unwrap().is_none());

    let row = stream2.try_next().await.unwrap().unwrap();
    assert_eq!(row.get::<i32>(0), 3);
    assert_eq!(row.get::<&str>(1), "charlie");
    assert!(stream2.try_next().await.unwrap().is_none());

    assert!(stream3.try_next().await.unwrap().is_none());
}

#[tokio::test]
async fn query_unnamed_with_transaction() {
    let mut client = connect("postgres://postgres:postgres@localhost:5432").await;

    client
        .execute("CREATE TEMPORARY TABLE foo (name TEXT, age INT);")
        .await
        .unwrap();

    let transaction = client.transaction().await.unwrap();

    let mut stream = transaction
        .query_unnamed(
            "INSERT INTO foo (name, age) VALUES ($1, $2), ($3, $4), ($5, $6) returning name, age",
            &[Type::TEXT, Type::INT4, Type::TEXT, Type::INT4, Type::TEXT, Type::INT4],
            &[&"alice", &20i32, &"bob", &30i32, &"carol", &40i32],
        )
        .unwrap();

    let mut inserted_values = Vec::new();

    while let Some(row) = stream.try_next().await.unwrap() {
        inserted_values.push((row.get::<String>(0), row.get::<i32>(1)));
    }

    assert_eq!(
        inserted_values,
        [
            ("alice".to_string(), 20),
            ("bob".to_string(), 30),
            ("carol".to_string(), 40)
        ]
    );

    let mut stream = transaction
        .query_unnamed(
            "SELECT name, age, 'literal', 5 FROM foo WHERE name <> $1 AND age < $2 ORDER BY age",
            &[Type::TEXT, Type::INT4],
            &[&"alice", &50i32],
        )
        .unwrap();

    let row = stream.try_next().await.unwrap().unwrap();
    assert_eq!(row.get::<&str>(0), "bob");
    assert_eq!(row.get::<i32>(1), 30);
    assert_eq!(row.get::<&str>(2), "literal");
    assert_eq!(row.get::<i32>(3), 5);

    let row = stream.try_next().await.unwrap().unwrap();
    assert_eq!(row.get::<&str>(0), "carol");
    assert_eq!(row.get::<i32>(1), 40);
    assert_eq!(row.get::<&str>(2), "literal");
    assert_eq!(row.get::<i32>(3), 5);

    assert!(stream.try_next().await.unwrap().is_none());

    // Test for UPDATE that returns no data
    let mut stream = transaction.query_unnamed("UPDATE foo set age = 33", &[], &[]).unwrap();
    assert!(stream.try_next().await.unwrap().is_none());
}
