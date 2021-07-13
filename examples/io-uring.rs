//! A Http/1 server read a temporary file filled with dummy string
//! and return it's content as response.

use std::io::Write;
use std::rc::Rc;

use tempfile::NamedTempFile;
use tokio_uring::fs::File;
use xitca_http::http::HeaderValue;
use xitca_service::fn_service;
use xitca_web::{request::WebRequest, response::WebResponse, App, HttpServer};

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

    let mut buf = Vec::with_capacity(64 * HELLO.len());
    let mut current = 0;

    loop {
        let (res, b) = file.read_at(buf, current).await;
        let n = res?;
        buf = b;

        if n == 0 {
            break;
        } else {
            current += n as u64;
        }
    }

    let mut res = req.as_response(buf);
    res.headers_mut()
        .append("Content-Type", HeaderValue::from_static("text/plain; charset=utf-8"));

    Ok(res)
}
