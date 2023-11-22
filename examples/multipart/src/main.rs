//! A Http server read multipart request and log it's field names.

use std::{io, pin::pin};

use tracing::info;
use xitca_web::{
    dev::service::Service,
    handler::{handler_service, multipart::Multipart, Responder},
    request::WebRequest,
    route::post,
    App,
};

fn main() -> io::Result<()> {
    tracing_subscriber::fmt().with_env_filter("[xitca-logger]=info").init();
    App::new()
        .at("/", post(handler_service(root)))
        .enclosed_fn(error_handler)
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

async fn root(multipart: Multipart<'_>) -> Result<&'static str, Box<dyn std::error::Error>> {
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

// an error handler that would catch root function's result type and transform it to response.
async fn error_handler<S, C, B, Res, SErr, Err>(service: &S, mut req: WebRequest<'_, C, B>) -> Result<Res, Err>
where
    S: for<'r> Service<WebRequest<'r, C, B>, Response = Result<Res, SErr>, Error = Err>,
    SErr: for<'r> Responder<WebRequest<'r, C, B>, Output = Res>,
{
    match service.call(req.reborrow()).await? {
        Ok(res) => Ok(res),
        Err(err) => Ok(err.respond_to(req).await),
    }
}
