//! A Http/1 tls server returns Hello World String as Response.
//!
//! *. io_uring is a linux OS feature.
//! *. random self signed cert is used for tls certification.

use std::{convert::Infallible, io, sync::Arc};

use rustls::{Certificate, PrivateKey, ServerConfig};
use xitca_http::{
    h1,
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Request, RequestExt, Response},
    HttpServiceBuilder, ResponseBody,
};
use xitca_service::fn_service;

fn main() -> io::Result<()> {
    let config = tls_config();
    xitca_server::Builder::new()
        .bind("http/1", "127.0.0.1:8080", move || {
            HttpServiceBuilder::h1(fn_service(handler))
                .io_uring() // specify io_uring flavor of http service.
                .rustls_uring(config.clone()) // specify io_uring flavor of tls.
        })?
        .build()
        .wait()
}

async fn handler(_: Request<RequestExt<h1::RequestBody>>) -> Result<Response<ResponseBody>, Infallible> {
    Ok(Response::builder()
        .header(CONTENT_TYPE, TEXT_UTF8)
        .body("Hello World from io_uring!".into())
        .unwrap())
}

// rustls configuration.
fn tls_config() -> Arc<ServerConfig> {
    let subject_alt_names = vec!["127.0.0.1".to_string(), "localhost".to_string()];

    let cert = rcgen::generate_simple_self_signed(subject_alt_names).unwrap();
    let key = PrivateKey(cert.serialize_private_key_der());
    let cert = vec![Certificate(cert.serialize_der().unwrap())];

    let mut config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert, key)
        .unwrap();

    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Arc::new(config)
}
