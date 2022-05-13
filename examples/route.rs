//! A HTTP/1 and HTTP/2 plain text server with basic routing.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use xitca_http::{
    body::{Once, RequestBody},
    bytes::Bytes,
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, IntoResponse, Response},
    util::service::{route::get, Router},
    HttpServiceBuilder, Request,
};
use xitca_service::fn_service;

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("xitca=info,[xitca-logger]=trace")
        .init();

    // construct service factory.
    let factory = || {
        // construct router and insert routes.
        let route = Router::new()
            // route request to root path with GET method to index function.
            .insert("/", get(fn_service(index)))
            // route to foo with GET or POST method to foo function.
            .insert("/foo", get(fn_service(foo)).post(fn_service(foo)));

        // pass router to builder.
        HttpServiceBuilder::new(route)
    };

    // construct and start server
    xitca_server::Builder::new()
        .bind("http/1", "127.0.0.1:8080", factory)?
        .build()
        .await
}

async fn index(req: Request<RequestBody>) -> Result<Response<Once<Bytes>>, anyhow::Error> {
    let mut res = req.into_response(Bytes::from_static(b"Hello,World!"));
    res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
    Ok(res)
}

async fn foo(req: Request<RequestBody>) -> Result<Response<Once<Bytes>>, anyhow::Error> {
    let mut res = req.into_response(Bytes::from_static(b"Hello,World from foo!"));
    res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
    Ok(res)
}
