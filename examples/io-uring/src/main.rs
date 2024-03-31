//! A Http/1 tls server returns Hello World String as Response.
//!
//! *. io_uring is a linux OS feature.
//! *. random self signed cert is used for tls certification.

use std::{convert::Infallible, io, sync::Arc};

use rustls::ServerConfig;
use xitca_http::{
    h1,
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Request, RequestExt, Response},
    HttpServiceBuilder, ResponseBody,
};
use xitca_service::{fn_service, ServiceExt};

fn main() -> io::Result<()> {
    xitca_server::Builder::new()
        .bind(
            "http/1",
            "127.0.0.1:8080",
            fn_service(handler).enclosed(
                HttpServiceBuilder::h1()
                    .io_uring() // specify io_uring flavor of http service.
                    .rustls_uring(tls_config()), // specify io_uring flavor of tls.
            ),
        )?
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

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.into()],
            cert.key_pair.serialize_der().try_into().unwrap(),
        )
        .unwrap();

    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Arc::new(config)
}
