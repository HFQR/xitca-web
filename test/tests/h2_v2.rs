use std::net::SocketAddr;

use xitca_client::Client;
use xitca_http::{h2, http::Version};
use xitca_io::net::TcpStream;
use xitca_service::fn_service;

#[tokio::test]
async fn h2_v2_get() {
    std::thread::spawn(server);

    let c = Client::new();

    let mut req = c.get("https://localhost:8080/").unwrap();

    req.headers_mut().insert("foo", "bar".parse().unwrap());

    let res = req.version(Version::HTTP_2).send().await.unwrap();

    assert!(res.status().is_success());
}

fn server() -> std::io::Result<()> {
    let factory = || fn_service(|(stream, _): (TcpStream, SocketAddr)| h2::run(stream));
    xitca_server::Builder::new()
        .bind("qa", "localhost:8080", factory)?
        .build()
        .wait()
}
