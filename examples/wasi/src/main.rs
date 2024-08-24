use std::{io, os::wasi::io::FromRawFd};

use xitca_web::{handler::handler_service, middleware::Logger, route::get, App};

fn main() -> io::Result<()> {
    // get fd int from env.
    let fd = std::env::var("FD_COUNT")
        .ok()
        .and_then(|var| var.parse().ok())
        .expect("failed to parse FD_COUNT env");

    // convert to tcp listener.
    let listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };

    // run server.
    App::new()
        .at("/", get(handler_service(|| async { "hello,wasi" })))
        .enclosed(Logger::new())
        .serve()
        .listen(listener)?
        .run()
        .wait()
}
