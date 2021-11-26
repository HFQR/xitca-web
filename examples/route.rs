//! A HTTP/1 and HTTP/2 plain text server with basic routing.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use xitca_http::{
    body::{RequestBody, ResponseBody},
    http::{
        header::{HeaderValue, CONTENT_TYPE},
        IntoResponse, Request, Response,
    },
    util::service::{get, Router},
    HttpServiceBuilder,
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

async fn index(req: Request<RequestBody>) -> Result<Response<ResponseBody>, anyhow::Error> {
    let mut res = req.into_response("Hello,World!");
    res.headers_mut().insert(CONTENT_TYPE, TEXT);
    Ok(res)
}

async fn foo(req: Request<RequestBody>) -> Result<Response<ResponseBody>, anyhow::Error> {
    let mut res = req.into_response("Hello,World from foo!");
    res.headers_mut().insert(CONTENT_TYPE, TEXT);
    Ok(res)
}

const TEXT: HeaderValue = HeaderValue::from_static("text/plain; charset=utf-8");
