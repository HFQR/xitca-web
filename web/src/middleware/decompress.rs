use core::{cell::RefCell, convert::Infallible, future::Future};

use http_encoding::{error::EncodingError, Coder};

use crate::{
    body::BodyStream,
    dev::service::{pipeline::PipelineE, ready::ReadyService, Service},
    handler::Responder,
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Request, StatusCode},
    request::WebRequest,
    response::WebResponse,
};

/// A decompress middleware look into [WebRequest]'s `Content-Encoding` header and
/// apply according decompression to it according to enabled compress feature.
/// `compress-x` feature must be enabled for this middleware to function correctly.
#[derive(Clone)]
pub struct Decompress;

impl<S> Service<S> for Decompress {
    type Response = DecompressService<S>;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where S: 'f;

    fn call<'s>(&self, service: S) -> Self::Future<'s>
    where
        S: 's,
    {
        async { Ok(DecompressService { service }) }
    }
}

pub struct DecompressService<S> {
    service: S,
}

pub type DecompressServiceError<E> = PipelineE<EncodingError, E>;

impl<'r, S, C, B, Res, Err> Service<WebRequest<'r, C, B>> for DecompressService<S>
where
    C: 'r,
    B: BodyStream + Default + 'r,
    S: for<'rs> Service<WebRequest<'rs, C, Coder<B>>, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = DecompressServiceError<Err>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, 'r: 'f;

    fn call<'s>(&'s self, mut req: WebRequest<'r, C, B>) -> Self::Future<'s>
    where
        'r: 's,
    {
        async move {
            let (parts, ext) = req.take_request().into_parts();
            let ctx = req.ctx;
            let (ext, body) = ext.replace_body(());
            let req = Request::from_parts(parts, ());

            let decoder = http_encoding::try_decoder(&req, body).map_err(DecompressServiceError::First)?;
            let mut body = RefCell::new(decoder);
            let mut req = req.map(|_| ext);

            let req = WebRequest::new(&mut req, &mut body, ctx);

            self.service.call(req).await.map_err(DecompressServiceError::Second)
        }
    }
}

impl<S> ReadyService for DecompressService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type Future<'f> = S::Future<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::Future<'_> {
        self.service.ready()
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for EncodingError {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
        let mut res = req.into_response(format!("{self}"));
        res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
        *res.status_mut() = StatusCode::UNSUPPORTED_MEDIA_TYPE;
        async { res }
    }
}

#[cfg(test)]
mod test {
    use http_encoding::{encoder, ContentEncoding};
    use xitca_http::body::{Once, RequestBody};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{dev::bytes::Bytes, http::header::CONTENT_ENCODING};

    use crate::{
        body::ResponseBody,
        handler::handler_service,
        http::{Request, RequestExt},
        response::WebResponse,
        test::collect_body,
        App,
    };

    use super::*;

    const Q: &[u8] = b"what is the goal of life";
    const A: &str = "go dock for chip";

    async fn handler(vec: Vec<u8>) -> &'static str {
        assert_eq!(Q, vec);
        A
    }

    #[test]
    fn build() {
        async fn noop() -> &'static str {
            "noop"
        }

        App::new()
            .at("/", handler_service(noop))
            .enclosed(Decompress)
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::new(RequestExt::<RequestBody>::default()))
            .now_or_panic()
            .ok()
            .unwrap();
    }

    #[test]
    fn plain() {
        let req = Request::new(RequestExt::<()>::default().map_body(|_| Once::new(Q)));
        App::new()
            .at("/", handler_service(handler))
            .enclosed(Decompress)
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req)
            .now_or_panic()
            .ok()
            .unwrap();
    }

    #[cfg(any(feature = "compress-br", feature = "compress-gz", feature = "compress-de"))]
    #[test]
    fn compressed() {
        // a hack to generate a compressed client request from server response.
        let res = WebResponse::<ResponseBody>::new(ResponseBody::bytes(Bytes::from_static(Q)));

        let encoding = {
            #[cfg(all(feature = "compress-br", not(any(feature = "compress-gz", feature = "compress-de"))))]
            {
                ContentEncoding::Br
            }
            #[cfg(all(feature = "compress-gz", not(any(feature = "compress-br", feature = "compress-de"))))]
            {
                ContentEncoding::Gzip
            }
            #[cfg(all(feature = "compress-de", not(any(feature = "compress-br", feature = "compress-gz"))))]
            {
                ContentEncoding::Deflate
            }
            #[cfg(all(feature = "compress-de", feature = "compress-br", feature = "compress-gz"))]
            {
                ContentEncoding::Br
            }
        };

        let (mut parts, body) = encoder(res, encoding).into_parts();

        let body = collect_body(body).now_or_panic().unwrap();

        let mut req =
            Request::new(Once::new(Bytes::from(body))).map(|body| RequestExt::<()>::default().map_body(|_| body));
        req.headers_mut()
            .insert(CONTENT_ENCODING, parts.headers.remove(CONTENT_ENCODING).unwrap());

        App::new()
            .at("/", handler_service(handler))
            .enclosed(Decompress)
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req)
            .now_or_panic()
            .ok()
            .unwrap();
    }
}
