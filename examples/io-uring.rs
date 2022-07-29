//! A Http/1 server read a temporary file filled with dummy string
//! and return it's content as response.

/*
   Need io-uring feature flag to start. e.g:
   cargo run --example io-uring --features io-uring
*/

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::{io::Write, rc::Rc};

use tempfile::NamedTempFile;
use tokio_uring::fs::File;
use xitca_web::{
    dev::service::fn_service,
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE},
    request::WebRequest,
    response::WebResponse,
    App, HttpServer,
};

const HELLO: &[u8] = b"hello world!";

fn main() -> std::io::Result<()> {
    tokio_uring::start(async {
        HttpServer::new(move || {
            // a temporary file with 64 hello world string.
            let mut file = NamedTempFile::new().unwrap();
            for _ in 0..64 {
                file.write_all(HELLO).unwrap();
            }
            App::with_current_thread_state(Rc::new(file))
                .at("/", fn_service(handler))
                .finish()
        })
        .bind("127.0.0.1:8080")?
        .run()
        .await
    })
}

async fn handler(req: WebRequest<'_, Rc<NamedTempFile>>) -> Result<WebResponse, Box<dyn std::error::Error>> {
    let file = File::open(req.state().path()).await?;
    let res = read(&file).await;
    file.close().await?;
    let buf = res?;

    let mut res = req.into_response(buf);
    res.headers_mut().append(CONTENT_TYPE, TEXT_UTF8);

    Ok(res)
}

async fn read(file: &File) -> std::io::Result<Vec<u8>> {
    let cap = 64 * HELLO.len();
    let mut buf = Vec::with_capacity(cap);
    let mut current = 0;

    while current < cap {
        let (res, b) = file.read_at(buf, current as u64).await;
        let n = res?;
        buf = b;

        current += n;
    }

    Ok(buf)
}
