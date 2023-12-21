//! A Http server read multipart request and log it's field names.

use std::{io, pin::pin};

use tracing::info;
use xitca_web::{
    handler::{handler_service, multipart::Multipart},
    route::post,
    App,
};

fn main() -> io::Result<()> {
    tracing_subscriber::fmt().with_env_filter("[xitca-logger]=info").init();
    App::new()
        .at("/", post(handler_service(root)))
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

type Error = Box<dyn std::error::Error + Send + Sync>;

async fn root(multipart: Multipart) -> Result<&'static str, Error> {
    // pin multipart on stack for async stream handling.
    let mut multipart = pin!(multipart);

    // iterate multipart fields.
    while let Some(mut field) = multipart.try_next().await? {
        // try to log field name and file name
        if let Some(name) = field.name() {
            info!("field name: {name}");
        }
        if let Some(name) = field.file_name() {
            info!("field file name: {name}");
        }

        // read field content and drop it in place.
        while field.try_next().await?.is_some() {}
    }

    // return an empty string as response.
    Ok("")
}
