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

async fn smoke_test(s: &str) {
    let client = connect(s).await;

    let stmt = client.prepare("SELECT $1::INT", &[]).await.unwrap();
    let mut stream = client.query(&stmt, &[&1i32]).unwrap();
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

    let sleep = client.execute_simple("SELECT pg_sleep(100)");

    tokio::task::yield_now().await;

    cancel_token.query_cancel().await.unwrap();

    let e = sleep.await.unwrap_err();

    let e = e.downcast_ref::<DbError>().unwrap();
    assert_eq!(e.code(), &SqlState::QUERY_CANCELED);
}
