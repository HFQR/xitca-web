// use std::{
//     io::{Read, Write},
//     net::TcpStream,
//     time::Duration,
// };

use futures_util::StreamExt;
use xitca_client::Client;
use xitca_http::{
    body::ResponseBody,
    bytes::{Bytes, BytesMut},
    h2,
    http::{header, Method, Request, Response, Version},
};
use xitca_service::fn_service;
use xitca_test::{test_h2_server, Error};

#[tokio::test]
async fn h2_get() -> Result<(), Error> {
    let mut handle = test_h2_server(|| fn_service(handle))?;

    let server_url = format!("https://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut res = c.get(&server_url)?.version(Version::HTTP_2).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(!res.is_close_connection());
        let body = res.string().await?;
        assert_eq!("GET Response", body);
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h2_post() -> Result<(), Error> {
    let mut handle = test_h2_server(|| fn_service(handle))?;

    let server_url = format!("https://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 * 1024 {
            body.extend_from_slice(b"Hello,World!");
        }
        let mut res = c.post(&server_url)?.version(Version::HTTP_2).text(body).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(!res.is_close_connection());
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

// #[tokio::test]
// async fn h2_keepalive() -> Result<(), Error> {
//     let mut handle = test_h2_server(|| fn_service(handle))?;

//     let mut stream = TcpStream::connect(handle.addr())?;

//     let mut buf = [0; 128];
//     for _ in 0..3 {
//         stream.write_all(SIMPLE_GET_REQ)?;

//         let n = stream.read(&mut buf)?;
//         tokio::time::sleep(Duration::from_millis(1)).await;
//         assert!(&buf[..n].ends_with(b"GET Response"));
//     }

//     // default h1 keep alive is 5 seconds.
//     // If test_server has a different setting this must be changed accordingly.
//     tokio::time::sleep(Duration::from_secs(6)).await;

//     // Due to platform difference this block may success or not.
//     // Either way it can not get a response from server as it has already close
//     // the connection.
//     {
//         let _ = stream.write_all(SIMPLE_GET_REQ);

//         if let Ok(n) = stream.read(&mut buf) {
//             assert_eq!(n, 0);
//         }
//     }

//     handle.try_handle()?.stop(true);

//     handle.await?;

//     Ok(())
// }

async fn handle(req: Request<h2::RequestBody>) -> Result<Response<ResponseBody>, Error> {
    // Some yield for testing h1 dispatcher's concurrent future handling.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => Ok(Response::new(Bytes::from("GET Response").into())),
        (&Method::POST, "/") => {
            let (parts, mut body) = req.into_parts();

            let length = parts
                .headers
                .get(header::CONTENT_LENGTH)
                .unwrap()
                .to_str()?
                .parse::<usize>()?;

            let mut buf = BytesMut::new();

            while let Some(bytes) = body.next().await {
                buf.extend_from_slice(&bytes?);
            }

            assert!(buf.len() == length);

            Ok(Response::new(Bytes::new().into()))
        }
        _ => todo!(),
    }
}

// const SIMPLE_GET_REQ: &[u8] = b"GET / HTTP/1.1\r\ncontent-length: 0\r\n\r\n";
