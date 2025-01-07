use redirectionio_xitca_proxy::{HttpPeer, Proxy};
use std::net::ToSocketAddrs;
use xitca_web::App;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();

    let address = "echo.websocket.org:443"
        .to_socket_addrs()
        .expect("error getting addresses")
        .next()
        .expect("cannot get address");

    App::new()
        .at(
            "",
            Proxy::new(HttpPeer::new(address, "echo.websocket.org:443").tls(true)),
        )
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .await?;

    Ok(())
}
