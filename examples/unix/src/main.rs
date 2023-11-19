//! A UnixDomain server returns Hello World String as Response.

use xitca_web::{handler::handler_service, route::get, App};

fn main() -> std::io::Result<()> {
    let mut server = App::new().at("/", get(handler_service(handler))).serve();

    #[cfg(unix)]
    {
        server = server.bind_unix("/tmp/xitca-web.socket")?;
    }

    #[cfg(not(unix))]
    {
        server = server.bind("127.0.0.1:8080")?;
    }

    server.run().wait()
}

async fn handler() -> &'static str {
    "Hello,World!"
}
