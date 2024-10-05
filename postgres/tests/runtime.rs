use core::future::IntoFuture;

use xitca_postgres::{
    error::{Completed, DbError, SqlState},
    iter::AsyncLendingIterator,
    pipeline::Pipeline,
    statement::Statement,
    types::Type,
    Client, Execute, Postgres,
};

async fn connect(s: &str) -> Client {
    let (client, driver) = Postgres::new(s).connect().await.unwrap();
    tokio::spawn(driver.into_future());
    client
}

async fn smoke_test(s: &str) {
    let client = connect(s).await;
    let stmt = Statement::named("SELECT $1::INT", &[]).execute(&client).await.unwrap();
    let mut stream = stmt.bind([1i32]).query(&client).await.unwrap();
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

    let sleep = "SELECT pg_sleep(10)".execute(&client);

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

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

    "SELECT 1".execute(&cli).await.unwrap();

    // yield to execute the abort of driver task. this depends on single thread
    // tokio runtime's behavior specifically.
    handle.abort();
    tokio::task::yield_now().await;

    let e = "SELECT 1".execute(&cli).await.err().unwrap();
    assert!(e.is_driver_down());
}

#[tokio::test]
async fn poll_after_response_finish() {
    let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
        .connect()
        .await
        .unwrap();

    tokio::spawn(drv.into_future());

    let mut stream = "SELECT 1".query(&cli).await.unwrap();

    stream.try_next().await.unwrap().unwrap();

    assert!(stream.try_next().await.unwrap().is_none());

    let e = stream.try_next().await.unwrap_err();

    assert!(e.downcast_ref::<Completed>().is_some());
}

#[tokio::test]
async fn query_portal() {
    let mut client = connect("postgres://postgres:postgres@localhost:5432").await;

    std::path::Path::new("samples/test.sql").execute(&client).await.unwrap();

    let transaction = client.transaction().await.unwrap();

    let stmt = Statement::named("SELECT id, name FROM foo ORDER BY id", &[])
        .execute(&transaction)
        .await
        .unwrap();

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

    String::from("CREATE TEMPORARY TABLE foo (name TEXT, age INT);")
        .execute(&client)
        .await
        .unwrap();

    let transaction = client.transaction().await.unwrap();

    let mut stream = Statement::unnamed(
        "INSERT INTO foo (name, age) VALUES ($1, $2), ($3, $4), ($5, $6) returning name, age",
        &[Type::TEXT, Type::INT4, Type::TEXT, Type::INT4, Type::TEXT, Type::INT4],
    )
    .bind_dyn(&[&"alice", &20i32, &"bob", &30i32, &"carol", &40i32])
    .query(&transaction)
    .await
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

    let mut stream = Statement::unnamed(
        "SELECT name, age, 'literal', 5 FROM foo WHERE name <> $1 AND age < $2 ORDER BY age",
        &[Type::TEXT, Type::INT4],
    )
    .bind_dyn(&[&"alice", &50i32])
    .query(&transaction)
    .await
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
    let mut stream = Statement::unnamed("UPDATE foo set age = 33", &[])
        .bind_dyn(&[])
        .query(&transaction)
        .await
        .unwrap();
    assert!(stream.try_next().await.unwrap().is_none());
}

#[tokio::test]
async fn pipeline() {
    let cli = connect("postgres://postgres:postgres@localhost:5432").await;

    "CREATE TEMPORARY TABLE foo (name TEXT, age INT);"
        .execute(&cli)
        .await
        .unwrap();

    Statement::unnamed(
        "INSERT INTO foo (name, age) VALUES ($1, $2), ($3, $4), ($5, $6);",
        &[Type::TEXT, Type::INT4, Type::TEXT, Type::INT4, Type::TEXT, Type::INT4],
    )
    .bind_dyn(&[&"alice", &20i32, &"bob", &30i32, &"charlie", &40i32])
    .execute(&cli)
    .await
    .unwrap();

    let stmt = Statement::named("SELECT * FROM foo", &[]).execute(&cli).await.unwrap();

    let mut pipe = Pipeline::new();

    stmt.query(&mut pipe).unwrap();
    stmt.query(&mut pipe).unwrap();

    let mut res = pipe.query(&cli).unwrap();

    {
        let mut item = res.try_next().await.unwrap().unwrap();
        let row = item.try_next().await.unwrap().unwrap();
        assert_eq!(row.get::<&str>(0), "alice");
        assert_eq!(row.get::<i32>(1), 20);

        let row = item.try_next().await.unwrap().unwrap();
        assert_eq!(row.get::<&str>(0), "bob");
        assert_eq!(row.get::<i32>(1), 30);
    }

    {
        let mut item = res.try_next().await.unwrap().unwrap();
        item.try_next().await.unwrap().unwrap();
        item.try_next().await.unwrap().unwrap();
        let row = item.try_next().await.unwrap().unwrap();
        assert_eq!(row.get::<&str>(0), "charlie");
        assert_eq!(row.get::<i32>(1), 40);
    }

    assert!(res.try_next().await.unwrap().is_none());

    let e = res.try_next().await.err().unwrap();

    assert!(e.downcast_ref::<Completed>().is_some());
}
