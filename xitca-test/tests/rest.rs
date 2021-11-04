use xitca_http::{
    body::ResponseBody,
    h1,
    http::{Method, Request, Response},
};
use xitca_io::bytes::Bytes;
use xitca_service::fn_service;
use xitca_test::{test_h1_server, Error};

#[tokio::test]
async fn h1_get() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let c = xitca_client::Client::new();

    let res = c.get(&format!("http://{}/", handle.ip_port_string()))?.send().await?;

    assert_eq!(res.head().status().as_u16(), 200);

    let body = res.string().await?;
    assert_eq!("GET Response", body);

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

async fn handle(req: Request<h1::RequestBody>) -> Result<Response<ResponseBody>, Error> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => Ok(Response::new(Bytes::from("GET Response").into())),
        _ => todo!(),
    }
}
