use core::convert::Infallible;

use std::error;

use crate::{
    body::{BodyStream, RequestBody},
    context::WebContext,
    error::{BadRequest, Error},
    handler::FromRequest,
    http::WebResponse,
    service::Service,
};

pub type Multipart<B = RequestBody> = http_multipart::Multipart<B>;

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for Multipart<B>
where
    B: BodyStream + Default,
{
    type Type<'b> = Multipart<B>;
    type Error = Error<C>;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let body = ctx.take_body_ref();
        http_multipart::multipart(ctx.req(), body).map_err(Into::into)
    }
}

impl<E, C> From<http_multipart::MultipartError<E>> for Error<C>
where
    E: error::Error + Send + Sync + 'static,
{
    fn from(e: http_multipart::MultipartError<E>) -> Self {
        Error::from_service(e)
    }
}

impl<'r, C, B, E> Service<WebContext<'r, C, B>> for http_multipart::MultipartError<E> {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        BadRequest.call(ctx).await
    }
}

#[cfg(test)]
mod test {
    use core::pin::pin;

    use xitca_http::body::Once;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        bytes::Bytes,
        handler::handler_service,
        http::{
            header::{HeaderValue, CONTENT_TYPE, TRANSFER_ENCODING},
            Method, Request, RequestExt,
        },
        route::post,
        service::Service,
        test::collect_body,
        App,
    };

    use super::*;

    async fn handler(multipart: Multipart<Once<Bytes>>) -> Vec<u8> {
        let mut multipart = pin!(multipart);

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

        let mut req = Request::new(RequestExt::<()>::default().map_body(|_| Once::new(Bytes::from_static(body))));
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
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req)
            .now_or_panic()
            .unwrap();

        let body = collect_body(res.into_body()).now_or_panic().unwrap();

        assert_eq!(body, b"testtestdata");
    }
}
