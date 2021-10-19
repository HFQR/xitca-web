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
use xitca_web::{dev::fn_service, http::HeaderValue, request::WebRequest, response::WebResponse, App, HttpServer};

const HELLO: &[u8] = b"hello world!";

fn main() -> std::io::Result<()> {
    tokio_uring::start(async {
        HttpServer::new(move || {
            // a temporary file with 64 hello world string.
            let mut file = NamedTempFile::new().unwrap();
            for _ in 0..64 {
                file.write_all(HELLO).unwrap();
            }
            App::with_current_thread_state(Rc::new(file)).service(fn_service(handler))
        })
        .bind("127.0.0.1:8080")?
        .run()
        .await
    })
}

async fn handler(req: &mut WebRequest<'_, Rc<NamedTempFile>>) -> Result<WebResponse, Box<dyn std::error::Error>> {
    let file = File::open(req.state().path()).await?;
    let res = read(&file).await;
    file.close().await?;
    let buf = res?;

    let mut res = req.as_response(buf);
    res.headers_mut()
        .append("Content-Type", HeaderValue::from_static("text/plain; charset=utf-8"));

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
