//! A HttpServer returns Hello World String as HttpResponse and always close connection afterwards.

use std::io;

use actix_server_alt::net::TcpStream;
use actix_service::fn_service;
use tokio::io::AsyncWriteExt;

const BUF: &[u8] = b"HTTP/1.1 200 OK\r\n\
    content-length: 12\r\n\
    connection: close\r\n\
    date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\
    \r\n\
    Hello World!";

#[actix_web::main]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    let addr = "127.0.0.1:8080";

    actix_server_alt::Builder::new()
        .bind("test", addr, || fn_service(handle))?
        .build()
        .await
}

async fn handle(mut stream: TcpStream) -> io::Result<()> {
    stream.write(BUF).await?;
    stream.flush().await?;
    stream.shutdown().await
}
