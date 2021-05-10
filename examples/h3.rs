//! A Http/3 Server runs on top of [h3](https://github.com/hyperium/h3) and [quinn](https://github.com/quinn-rs/quinn)

use std::io;

use actix_server_alt::{net::UdpStream, Builder};
use actix_service::fn_service;
use bytes::Bytes;
use h3::server::RequestStream;
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
    let (cert_chain, key) = build_certs();
    config.certificate(cert_chain, key).unwrap();

    let config = config.build();

    Builder::new()
        .bind_h3("test", addr, config, || fn_service(handle))?
        .build()
        .await
}

async fn handle(udp: UdpStream) -> io::Result<()> {
    let conn = udp.connecting().await.map_err(|err| {
        warn!("connecting client failed with error: {:?}", err);
        io::Error::new(io::ErrorKind::Other, err)
    })?;

    debug!("New connection now established");

    let conn = h3_quinn::Connection::new(conn);
    let mut conn = h3::server::Connection::new(conn).await.unwrap();

    while let Some(res) = conn.accept().await.transpose() {
        let (req, stream) = res.map_err(|err| {
            warn!("accepting request failed with error: {:?}", err);
            io::Error::new(io::ErrorKind::Other, err)
        })?;

        debug!("connection requested: {:#?}", req);

        tokio::spawn(async move {
            match send_response(stream).await {
                Ok(_) => {
                    debug!("Response to connection successful");
                }
                Err(err) => {
                    error!("Unable to send response to connection peer: {:?}", err);
                }
            }
        });
    }

    Ok(())
}

const BODY: &[u8] = b"
<!DOCTYPE html>\
<html>\
<body>\
\r\n\
<h1>Http3 Example</h1>\
\r\n\
<p>It's working.</p>\
\r\n\
</body>\
</html>";

async fn send_response<S>(mut stream: RequestStream<S>) -> Result<(), Box<dyn std::error::Error>>
where
    S: h3::quic::SendStream<Bytes>,
{
    let res = http::Response::builder()
        .status(http::StatusCode::OK)
        .header("Content-Type", "text/html")
        .header("content-length", BODY.len())
        .body(())?;

    stream.send_response(res).await?;
    stream.send_data(Bytes::from_static(BODY)).await?;
    stream.finish().await?;

    Ok(())
}

// random self sign cert generator.
fn build_certs() -> (CertificateChain, PrivateKey) {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let key = PrivateKey::from_der(&cert.serialize_private_key_der()).unwrap();
    let cert = Certificate::from_der(&cert.serialize_der().unwrap()).unwrap();

    (CertificateChain::from_certs(vec![cert.clone()]), key)
}
