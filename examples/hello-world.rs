//! A Http/1 server returns Hello World String as Response.

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use actix_http_alt::{
    h1::RequestBody,
    http::{Request, Response},
    util::ErrorLoggerFactory,
    HttpServiceBuilder, ResponseBody,
};
use actix_service_alt::fn_service;
use actix_web_alt::HttpServer;
use bytes::Bytes;

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    // construct http server
    HttpServer::new(move || {
        let builder = HttpServiceBuilder::h1(fn_service(handler));
        ErrorLoggerFactory::new(builder)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

async fn handler(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>> {
    let res = Response::builder()
        .status(200)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Bytes::from_static(b"Hello World!").into())?;

    Ok(res)
}
