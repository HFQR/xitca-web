//! A Http server returns Hello World String as Response.

use std::{
    fs::File,
    io::{self, BufReader},
    sync::Arc,
};

use actix_http_alt::{
    http::{Request, Response},
    util::ErrorLoggerFactory,
    HttpServiceBuilder, RequestBody, ResponseBody,
};
use actix_service_alt::fn_service;
use bytes::Bytes;
use h3_quinn::quinn::generic::ServerConfig;
use h3_quinn::quinn::{crypto::rustls::TlsSession, CertificateChain, PrivateKey, ServerConfigBuilder};

use rustls::{
    self,
    internal::pemfile::{certs, pkcs8_private_keys},
    NoClientAuth,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt().with_env_filter("actix=trace").init();

    // set up rustls and alpn protocol.
    let acceptor = rustls_config()?;

    // construct http3 quic server config
    let config = h3_config()?;

    // construct server
    actix_server_alt::Builder::new()
        // bind to both tcp and udp addresses where a single service would handle http/1/2/3 traffic.
        .bind_all("hello-world", "127.0.0.1:8080", config, move || {
            let builder = HttpServiceBuilder::new(fn_service(handler)).rustls(acceptor.clone());

            ErrorLoggerFactory::new(builder)
        })?
        .build()
        .await
}

async fn handler(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>> {
    let res = Response::builder()
        .status(200)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Bytes::from_static(b"Hello World!").into())?;
    Ok(res)
}

fn h3_config() -> io::Result<ServerConfig<TlsSession>> {
    let mut config = ServerConfigBuilder::default();
    config.protocols(&[b"h3-29", b"h3-28", b"h3-27"]);

    let key = std::fs::read("./cert/key.pem")?;
    let key = PrivateKey::from_pem(&key).unwrap();

    let cert = std::fs::read("./cert/cert.pem")?;
    let cert = CertificateChain::from_pem(&cert).unwrap();

    config.certificate(cert, key).unwrap();

    Ok(config.build())
}

fn rustls_config() -> io::Result<Arc<rustls::ServerConfig>> {
    let mut acceptor = rustls::ServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(File::open("./cert/cert.pem")?);
    let key_file = &mut BufReader::new(File::open("./cert/key.pem")?);
    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();

    acceptor.set_single_cert(cert_chain, keys.remove(0)).unwrap();
    let protos = vec!["h2".into(), "http/1.1".into()];
    acceptor.set_protocols(&protos);

    Ok(Arc::new(acceptor))
}
