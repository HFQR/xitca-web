//! A Http/3 Server runs on top of [h3](https://github.com/hyperium/h3) and [quinn](https://github.com/quinn-rs/quinn)

use std::io;

use actix_server_alt::net::UdpStream;
use actix_service::fn_service;
use h3_quinn::quinn::{Certificate, CertificateChain, PrivateKey, ServerConfigBuilder};
use log::{debug, error, warn};

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, debug");
    env_logger::init();

    let addr = "localhost:8080";

    // construct server config
    let mut config = ServerConfigBuilder::default();
    config.protocols(&[b"h3-29"]);
    let (cert_chain, _cert, key) = build_certs();
    config.certificate(cert_chain, key).unwrap();

    let config = config.build();

    actix_server_alt::Builder::new()
        .workers(1)
        .bind_h3("test", addr, config, || fn_service(handle))?
        .build()
        .await
}

async fn handle(mut udp: UdpStream) -> io::Result<()> {
    match (&mut *udp).await {
        Ok(conn) => {
            debug!("New connection now established");

            let mut h3_conn = h3::server::Connection::new(h3_quinn::Connection::new(conn))
                .await
                .unwrap();

            while let Some((req, mut stream)) = h3_conn.accept().await.unwrap() {
                debug!("connection requested: {:#?}", req);

                tokio::spawn(async move {
                    let resp = http::Response::builder()
                        .status(http::StatusCode::NOT_FOUND)
                        .body(())
                        .unwrap();

                    match stream.send_response(resp).await {
                        Ok(_) => {
                            debug!("Response to connection successful");
                        }
                        Err(err) => {
                            error!("Unable to send response to connection peer: {:?}", err);
                        }
                    }

                    stream.finish().await.unwrap();
                });
            }

            Ok(())
        }
        Err(err) => {
            warn!("connecting client failed with error: {:?}", err);
            panic!("error not handled");
        }
    }
}

// random self sign cert generator.
fn build_certs() -> (CertificateChain, Certificate, PrivateKey) {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let key = PrivateKey::from_der(&cert.serialize_private_key_der()).unwrap();
    let cert = Certificate::from_der(&cert.serialize_der().unwrap()).unwrap();

    (CertificateChain::from_certs(vec![cert.clone()]), cert, key)
}
