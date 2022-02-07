//! A Http server returns Hello World String as Response.

use std::{fs, io, sync::Arc};

use h3_quinn::quinn::ServerConfig;
use rustls::{Certificate, PrivateKey};
use xitca_http::{
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Response},
    HttpServiceBuilder, Request, RequestBody, ResponseBody,
};
use xitca_service::fn_service;

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("xitca=trace,[xitca-logger]=trace")
        .init();

    // setup h1 and h2 rustls config.
    let acceptor = h1_h2_config()?;

    // setup h3 quinn config on top of rustls config.
    let config = h3_config()?;

    // construct server
    xitca_server::Builder::new()
        // bind to both tcp and udp addresses where a single service would handle http/1/2/3 traffic.
        .bind_all("hello-world", "127.0.0.1:8080", config, move || {
            HttpServiceBuilder::new(fn_service(handler))
                .rustls(acceptor.clone())
                .with_logger()
        })?
        .build()
        .await
}

async fn handler(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>> {
    let res = Response::builder()
        .status(200)
        .header(CONTENT_TYPE, TEXT_UTF8)
        .body("Hello World!".into())?;
    Ok(res)
}

fn h1_h2_config() -> io::Result<Arc<rustls::ServerConfig>> {
    let config = rustls_config(vec!["h2".into(), "http/1.1".into()])?;
    Ok(config)
}

fn h3_config() -> io::Result<ServerConfig> {
    let config = rustls_config(vec![b"h3-29".to_vec(), b"h3-28".to_vec(), b"h3-27".to_vec()])?;
    Ok(ServerConfig::with_crypto(config))
}

fn rustls_config(alpn_protocols: Vec<Vec<u8>>) -> io::Result<Arc<rustls::ServerConfig>> {
    let cert = fs::read("./cert/cert.pem")?;
    let key = fs::read("./cert/key.pem")?;

    let key = rustls_pemfile::pkcs8_private_keys(&mut &*key).unwrap().remove(0);
    let key = PrivateKey(key);

    let cert = rustls_pemfile::certs(&mut &*cert)
        .unwrap()
        .into_iter()
        .map(Certificate)
        .collect();

    let mut acceptor = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert, key)
        .unwrap();

    acceptor.alpn_protocols = alpn_protocols;

    Ok(Arc::new(acceptor))
}
