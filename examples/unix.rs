//! A UnixDomain server returns Hello World String as Response.

use xitca_http::{
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Request, Response},
    RequestBody, ResponseBody,
};
use xitca_web::{dev::fn_service, HttpServer};

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt().with_env_filter("xitca=trace").init();

    #[allow(unused_mut)]
    let mut server = HttpServer::new(|| fn_service(handler));

    #[cfg(unix)]
    {
        server = server.bind_unix("/tmp/xitca-web.socket")?;
    }

    server.bind("127.0.0.1:8080")?.run().await
}

async fn handler(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>> {
    let res = Response::builder()
        .status(200)
        .header(CONTENT_TYPE, TEXT_UTF8)
        .body("Hello World!".into())?;
    Ok(res)
}
