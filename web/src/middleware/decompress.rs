use std::{cell::RefCell, convert::Infallible, fmt, future::Future};

use futures_core::stream::Stream;
use http_encoding::{Coder, FeatureError};

use crate::{
    dev::service::{pipeline::PipelineE, ready::ReadyService, BuildService, Service},
    request::WebRequest,
};

/// A decompress middleware look into [WebRequest]'s `Content-Encoding` header and
/// apply according decompression to it according to enabled compress feature.
/// `compress-x` feature must be enabled for this middleware to function correctly.
#[derive(Clone)]
pub struct Decompress;

impl<S> BuildService<S> for Decompress {
    type Service = DecompressService<S>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, service: S) -> Self::Future {
        async { Ok(DecompressService { service }) }
    }
}

pub struct DecompressService<S> {
    service: S,
}

pub type DecompressServiceError<E> = PipelineE<FeatureError, E>;

impl<'r, S, C, B, T, E, Res, Err> Service<WebRequest<'r, C, B>> for DecompressService<S>
where
    C: 'static,
    B: Stream<Item = Result<T, E>> + Default + 'static,
    T: AsRef<[u8]> + 'static,
    E: fmt::Debug,
    S: for<'rs> Service<WebRequest<'rs, C, Coder<B>>, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = DecompressServiceError<Err>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    fn call(&self, mut req: WebRequest<'r, C, B>) -> Self::Future<'_> {
        async move {
            let (mut http_req, body) = req.take_request().replace_body(());

            let decoder = http_encoding::try_decoder(&*http_req, body)
                // TODO: rework http-encoding error: seprate the error type to streaming error and construction error.
                .map_err(|_| DecompressServiceError::First(FeatureError::Br))?;

            let mut body = RefCell::new(decoder);

            let req = WebRequest::new(&mut http_req, &mut body, req.ctx);

            self.service.call(req).await.map_err(DecompressServiceError::Second)
        }
    }
}

impl<'r, S, C, B, T, E, Res, Err, Rdy> ReadyService<WebRequest<'r, C, B>> for DecompressService<S>
where
    C: 'static,
    B: Stream<Item = Result<T, E>> + Default + 'static,
    T: AsRef<[u8]> + 'static,
    E: fmt::Debug,
    S: for<'rs> ReadyService<WebRequest<'rs, C, Coder<B>>, Response = Res, Error = Err, Ready = Rdy>,
{
    type Ready = Rdy;
    type ReadyFuture<'f> = impl Future<Output = Self::Ready> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move { self.service.ready().await }
    }
}

#[cfg(test)]
mod test {
    use std::future::poll_fn;

    use http_encoding::{encoder, ContentEncoding};
    use xitca_http::{body::Once, Request};
    use xitca_unsafe_collection::{futures::NowOrPanic, pin};

    use crate::{dev::bytes::Bytes, http::header::CONTENT_ENCODING};

    use crate::{
        handler::{handler_service, vec::Vec},
        request::RequestBody,
        response::{ResponseBody, WebResponse},
        App,
    };

    use super::*;

    const Q: &[u8] = b"what is the goal of life";
    const A: &str = "go dock for chip";

    async fn handler(Vec(vec): Vec) -> &'static str {
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
            .build(())
            .now_or_panic()
            .unwrap()
            .call(Request::new(RequestBody::default()))
            .now_or_panic()
            .ok()
            .unwrap();
    }

    #[test]
    fn plain() {
        App::new()
            .at("/", handler_service(handler))
            .enclosed(Decompress)
            .finish()
            .build(())
            .now_or_panic()
            .unwrap()
            .call(Request::new(Once::new(Q)))
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

        pin!(body);

        let mut buf = std::vec::Vec::new();

        while let Some(Ok(bytes)) = poll_fn(|cx| body.as_mut().poll_next(cx)).now_or_panic() {
            buf.extend_from_slice(bytes.as_ref());
        }

        let mut req = Request::new(Once::new(Bytes::from(buf)));
        req.headers_mut()
            .insert(CONTENT_ENCODING, parts.headers.remove(CONTENT_ENCODING).unwrap());

        App::new()
            .at("/", handler_service(handler))
            .enclosed(Decompress)
            .finish()
            .build(())
            .now_or_panic()
            .unwrap()
            .call(req)
            .now_or_panic()
            .ok()
            .unwrap();
    }
}
