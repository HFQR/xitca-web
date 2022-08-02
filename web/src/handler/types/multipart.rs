use std::future::Future;

use crate::{
    handler::{ExtractError, FromRequest},
    request::{RequestBody, WebRequest},
    stream::WebStream,
};

pub type Multipart<'a, B = RequestBody> = http_multipart::Multipart<'a, B>;

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for Multipart<'a, B>
where
    B: WebStream + Default,
{
    type Type<'b> = Multipart<'b, B>;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async move {
            let body = req.take_body_ref();
            let multipart = http_multipart::multipart(req.req(), body).unwrap();
            Ok(multipart)
        }
    }
}

#[cfg(test)]
mod test {
    use xitca_http::{body::Once, request::Request};
    use xitca_unsafe_collection::futures::NowOrPanic;
    use xitca_unsafe_collection::pin;

    use crate::test::collect_body;
    use crate::{
        dev::{
            bytes::Bytes,
            service::{BuildService, Service},
        },
        handler::handler_service,
        http::{
            header::{HeaderValue, CONTENT_TYPE, TRANSFER_ENCODING},
            Method,
        },
        route::post,
        App,
    };

    use super::*;

    async fn handler(multipart: Multipart<'_, Once<Bytes>>) -> Vec<u8> {
        pin!(multipart);

        let mut res = Vec::new();

        {
            let mut field = multipart.try_next().await.ok().unwrap().unwrap();

            assert_eq!(field.name().unwrap(), "file");
            assert_eq!(field.file_name().unwrap(), "foo.txt");

            while let Some(bytes) = field.try_next().await.ok().unwrap() {
                res.extend_from_slice(bytes.as_ref());
            }
        }

        {
            let mut field = multipart.try_next().await.ok().unwrap().unwrap();

            assert_eq!(field.name().unwrap(), "file");
            assert_eq!(field.file_name().unwrap(), "bar.txt");

            while let Some(bytes) = field.try_next().await.ok().unwrap() {
                res.extend_from_slice(bytes.as_ref());
            }
        }

        res
    }

    #[test]
    fn simple() {
        let body = b"\
            --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
            Content-Disposition: form-data; name=\"file\"; filename=\"foo.txt\"\r\n\
            Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
            test\r\n\
            --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
            Content-Disposition: form-data; name=\"file\"; filename=\"bar.txt\"\r\n\
            Content-Type: text/plain\r\nContent-Length: 8\r\n\r\n\
            testdata\r\n\
            --abbc761f78ff4d7cb7573b5a23f96ef0--\r\n";

        let mut req = Request::new(Once::new(Bytes::from_static(body)));
        *req.method_mut() = Method::POST;
        req.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("multipart/mixed; boundary=abbc761f78ff4d7cb7573b5a23f96ef0"),
        );
        req.headers_mut()
            .insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));

        let res = App::new()
            .at("/", post(handler_service(handler)))
            .finish()
            .build(())
            .now_or_panic()
            .unwrap()
            .call(req)
            .now_or_panic()
            .unwrap();

        let body = collect_body(res.into_body()).now_or_panic().unwrap();

        assert_eq!(body, b"testtestdata");
    }
}
