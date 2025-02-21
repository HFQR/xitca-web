//! decompression middleware

use crate::service::Service;

/// decompress middleware.
///
/// look into [WebContext]'s `Content-Encoding` header and apply according decompression to
/// it according to enabled compress feature.
/// `compress-x` feature must be enabled for this middleware to function correctly.
///
/// # Type mutation
/// `Decompress` would mutate request body type from `B` to `Coder<B>`. Service enclosed
/// by it must be able to handle it's mutation or utilize [TypeEraser] to erase the mutation.
/// For more explanation please reference [type mutation](crate::middleware#type-mutation).
///
/// [WebContext]: crate::WebContext
/// [TypeEraser]: crate::middleware::eraser::TypeEraser
#[derive(Clone)]
pub struct Decompress;

impl<S, E> Service<Result<S, E>> for Decompress {
    type Response = service::DecompressService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(service::DecompressService)
    }
}

mod service {
    use core::{cell::RefCell, convert::Infallible};

    use http_encoding::{Coder, error::EncodingError};

    use crate::{
        body::BodyStream,
        context::WebContext,
        error::Error,
        error::error_from_service,
        http::{Request, StatusCode, WebResponse, const_header_value::TEXT_UTF8, header::CONTENT_TYPE},
        service::ready::ReadyService,
    };

    use super::*;

    pub struct DecompressService<S>(pub(super) S);

    impl<'r, S, C, B, Res, Err> Service<WebContext<'r, C, B>> for DecompressService<S>
    where
        B: BodyStream + Default,
        S: for<'rs> Service<WebContext<'rs, C, Coder<B>>, Response = Res, Error = Err>,
        Err: Into<Error>,
    {
        type Response = Res;
        type Error = Error;

        async fn call(&self, mut ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
            let (parts, ext) = ctx.take_request().into_parts();
            let state = ctx.ctx;
            let (ext, body) = ext.replace_body(());
            let req = Request::from_parts(parts, ());

            let decoder = http_encoding::try_decoder(req.headers(), body)?;
            let mut body = RefCell::new(decoder);
            let mut req = req.map(|_| ext);

            self.0
                .call(WebContext::new(&mut req, &mut body, state))
                .await
                .map_err(|e| {
                    // restore original body as error path of other services may have use of it.
                    let body = body.into_inner().into_inner();
                    *ctx.body_borrow_mut() = body;
                    e.into()
                })
        }
    }

    impl<S> ReadyService for DecompressService<S>
    where
        S: ReadyService,
    {
        type Ready = S::Ready;

        #[inline]
        async fn ready(&self) -> Self::Ready {
            self.0.ready().await
        }
    }

    error_from_service!(EncodingError);

    impl<'r, C, B> Service<WebContext<'r, C, B>> for EncodingError {
        type Response = WebResponse;
        type Error = Infallible;

        async fn call(&self, req: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
            let mut res = req.into_response(format!("{self}"));
            res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
            *res.status_mut() = StatusCode::UNSUPPORTED_MEDIA_TYPE;
            Ok(res)
        }
    }
}

#[cfg(test)]
mod test {
    use http_encoding::{ContentEncoding, encoder};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        App,
        body::ResponseBody,
        handler::handler_service,
        http::header::CONTENT_ENCODING,
        http::{WebRequest, WebResponse},
        test::collect_body,
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
            .call(WebRequest::default())
            .now_or_panic()
            .ok()
            .unwrap();
    }

    #[test]
    fn plain() {
        let req = WebRequest::default().map(|ext| ext.map_body(|_: ()| Q.into()));
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
        let res = WebResponse::<ResponseBody>::new(Q.into());

        #[allow(unreachable_code)]
        let encoding = || {
            #[cfg(all(feature = "compress-br", not(any(feature = "compress-gz", feature = "compress-de"))))]
            {
                return ContentEncoding::Br;
            }

            #[cfg(all(feature = "compress-gz", not(any(feature = "compress-br", feature = "compress-de"))))]
            {
                return ContentEncoding::Gzip;
            }

            #[cfg(all(feature = "compress-de", not(any(feature = "compress-br", feature = "compress-gz"))))]
            {
                return ContentEncoding::Deflate;
            }

            ContentEncoding::Br
        };

        let encoding = encoding();

        let (mut parts, body) = encoder(res, encoding).into_parts();

        let body = collect_body(body).now_or_panic().unwrap();

        let mut req = WebRequest::default().map(|ext| ext.map_body(|_: ()| body.into()));

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
