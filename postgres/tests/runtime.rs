use core::future::IntoFuture;

use xitca_postgres::{
    Client, Execute, Postgres,
    error::{ClosedByDriver, DbError, SqlState},
    iter::AsyncLendingIterator,
    statement::Statement,
    transaction::{IsolationLevel, TransactionBuilder},
    types::Type,
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
async fn request_limit_one_preserves_protocol_progress() {
    let mut cfg = xitca_postgres::Config::try_from("postgres://postgres:postgres@localhost:5432").unwrap();
    cfg.max_in_flight_requests(1);
    let (mut client, driver) = Postgres::new(cfg).connect().await.unwrap();
    let handle = tokio::spawn(driver.into_future());

    tokio::time::timeout(core::time::Duration::from_secs(10), async {
        "CREATE TEMP TABLE lifecycle_items (value INT); CREATE TYPE pg_temp.lifecycle_enum AS ENUM ('x')"
            .execute(&client)
            .await
            .unwrap();
        // Type discovery submits additional requests while the prepare response still exists.
        let stmt = Statement::named("SELECT NULL::pg_temp.lifecycle_enum", &[])
            .execute(&client)
            .await
            .unwrap();
        stmt.query(&client).await.unwrap();
        drop(stmt);

        let (first, second) = tokio::join!("SELECT 1".execute(&client), "SELECT 2".execute(&client));
        assert_eq!(first.unwrap(), 1);
        assert_eq!(second.unwrap(), 1);
        assert!("SELECT 1 / 0".execute(&client).await.is_err());
        assert_eq!("SELECT 1".execute(&client).await.unwrap(), 1);

        let stmt = Statement::named("COPY lifecycle_items FROM STDIN", &[])
            .execute(&client)
            .await
            .unwrap()
            .leak();
        let mut copy = client.copy_in(&stmt).await.unwrap();
        copy.copy(&b"1\n2\n"[..]).unwrap();
        assert_eq!(copy.finish().await.unwrap(), 2);
        let mut copy = client.copy_in(&stmt).await.unwrap();
        copy.copy(&b"3\n"[..]).unwrap();
        drop(copy);
        assert_eq!("SELECT 1".execute(&client).await.unwrap(), 1);

        let transaction = client.transaction().await.unwrap();
        "SELECT 1".query(&transaction).await.unwrap();
        drop(transaction);
        assert_eq!("SELECT 1".execute(&client).await.unwrap(), 1);
    })
    .await
    .expect("request capacity blocked protocol progress");

    drop(client);
    tokio::time::timeout(core::time::Duration::from_secs(5), handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn pool_request_limit_one_supports_cached_and_concurrent_queries() {
    let mut cfg = xitca_postgres::Config::try_from("postgres://postgres:postgres@localhost:5432").unwrap();
    cfg.max_in_flight_requests(1);
    let pool = xitca_postgres::pool::Pool::builder(cfg.clone())
        .capacity(2)
        .build()
        .unwrap();
    let owned = xitca_postgres::pool::Pool::builder(cfg)
        .capacity(2)
        .build_owned()
        .unwrap();
    tokio::time::timeout(core::time::Duration::from_secs(10), async {
        for _ in 0..3 {
            let (a, b) = tokio::join!(
                Statement::named("SELECT 1", &[]).bind_none().execute(&pool),
                Statement::named("SELECT 2", &[]).bind_none().execute(&pool),
            );
            assert_eq!((a.unwrap(), b.unwrap()), (1, 1));
            let (a, b) = tokio::join!(
                Statement::named("SELECT 1", &[]).bind_none().execute(&owned),
                Statement::named("SELECT 2", &[]).bind_none().execute(&owned),
            );
            assert_eq!((a.unwrap(), b.unwrap()), (1, 1));
        }
    })
    .await
    .expect("pool capacity blocked statement preparation or execution");
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

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        cancel_token.query_cancel().await.unwrap();
    });

    let e = "SELECT pg_sleep(10)".execute(&client).await.unwrap_err();

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

    assert!(e.downcast_ref::<ClosedByDriver>().is_some());
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

    let portal = transaction.bind_dyn(&stmt, &[]).await.unwrap();
    // portal query is admitted in call order so the three execute messages reach the server in
    // the same order. polling them together keeps them in one write.
    let (stream1, stream2, stream3) =
        tokio::join!(portal.query_portal(2), portal.query_portal(2), portal.query_portal(2));
    let mut stream1 = stream1.unwrap();
    let mut stream2 = stream2.unwrap();
    let mut stream3 = stream3.unwrap();

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
async fn transaction_isolation() {
    let mut client = connect("postgres://postgres:postgres@localhost:5432").await;

    std::path::Path::new("samples/test.sql").execute(&client).await.unwrap();

    let transaction = TransactionBuilder::new()
        .isolation_level(IsolationLevel::Serializable)
        .read_only(true)
        .deferrable(true)
        .begin(&mut client)
        .await
        .unwrap();

    let stmt = Statement::named("SELECT id, name FROM foo ORDER BY id", &[])
        .execute(&transaction)
        .await
        .unwrap();

    let mut res = stmt.query(&transaction).await.unwrap();

    let row = res.try_next().await.unwrap().unwrap();
    assert_eq!(row.get::<i32>(0), 1);
    assert_eq!(row.get::<&str>(1), "alice");
}

#[tokio::test]
async fn query_unnamed_with_transaction() {
    let mut client = connect("postgres://postgres:postgres@localhost:5432").await;

    String::from("CREATE TEMPORARY TABLE foo (name TEXT, age INT);")
        .execute(&client)
        .await
        .unwrap();

    let transaction = client.transaction().await.unwrap();

    let mut stream = Statement::named(
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

    let mut stream = Statement::named(
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
    let mut stream = Statement::named("UPDATE foo set age = 33", &[])
        .bind_dyn(&[])
        .query(&transaction)
        .await
        .unwrap();
    assert!(stream.try_next().await.unwrap().is_none());
}

#[cfg(not(feature = "io-uring"))]
#[tokio::test]
async fn transaction_pool_connection() {
    let pool = xitca_postgres::pool::Pool::builder("postgres://postgres:postgres@localhost:5432")
        .build()
        .unwrap();

    let client = pool.get().await.unwrap();

    std::path::Path::new("samples/test.sql").execute(&client).await.unwrap();

    {
        let mut transaction = TransactionBuilder::new()
            .isolation_level(IsolationLevel::Serializable)
            .read_only(true)
            .deferrable(true)
            .begin_owned(client)
            .await
            .unwrap();

        let mut res = Statement::named("SELECT id, name FROM foo ORDER BY id", &[])
            .bind_none()
            .query(&mut transaction)
            .await
            .unwrap();

        let row = res.try_next().await.unwrap().unwrap();
        assert_eq!(row.get::<i32>(0), 1);
        assert_eq!(row.get::<&str>(1), "alice");
    }

    let mut client = pool.get().await.unwrap();

    {
        let mut transaction = TransactionBuilder::new().begin(&mut client).await.unwrap();

        let mut res = Statement::named("SELECT id, name FROM foo ORDER BY id", &[])
            .bind_none()
            .query(&mut transaction)
            .await
            .unwrap();

        let row = res.try_next().await.unwrap().unwrap();
        assert_eq!(row.get::<i32>(0), 1);
        assert_eq!(row.get::<&str>(1), "alice");
    }

    let mut transaction = TransactionBuilder::new().begin_owned(client).await.unwrap();

    let mut res = Statement::named("SELECT id, name FROM foo ORDER BY id", &[])
        .bind_none()
        .query(&mut transaction)
        .await
        .unwrap();

    let row = res.try_next().await.unwrap().unwrap();
    assert_eq!(row.get::<i32>(0), 1);
    assert_eq!(row.get::<&str>(1), "alice");
}
