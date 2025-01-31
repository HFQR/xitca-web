//! compression middleware

use crate::service::Service;

/// compress middleware.
///
/// look into [WebRequest]'s `Accept-Encoding` header and apply according compression to
/// [WebResponse]'s body according to enabled compress feature.
/// `compress-x` feature must be enabled for this middleware to function correctly.
///
/// # Type mutation
/// `Compress` would mutate response body type from `B` to `Coder<B>`. Service enclosed
/// by it must be able to handle it's mutation or utilize [TypeEraser] to erase the mutation.
/// For more explanation please reference [type mutation](crate::middleware#type-mutation).
///
/// [WebRequest]: crate::http::WebRequest
/// [WebResponse]: crate::http::WebResponse
/// [TypeEraser]: crate::middleware::eraser::TypeEraser
#[derive(Clone)]
pub struct Compress;

impl<S, E> Service<Result<S, E>> for Compress {
    type Response = service::CompressService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(service::CompressService)
    }
}
mod service {
    use http_encoding::{Coder, ContentEncoding, encoder};

    use crate::{
        body::{BodyStream, NONE_BODY_HINT},
        http::{BorrowReq, WebResponse, header::HeaderMap},
        service::{Service, ready::ReadyService},
    };

    pub struct CompressService<S>(pub(super) S);

    impl<S, Req, ResB> Service<Req> for CompressService<S>
    where
        Req: BorrowReq<HeaderMap>,
        S: Service<Req, Response = WebResponse<ResB>>,
        ResB: BodyStream,
    {
        type Response = WebResponse<Coder<ResB>>;
        type Error = S::Error;

        async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
            let mut encoding = ContentEncoding::from_headers(req.borrow());
            let res = self.0.call(req).await?;

            // TODO: expose encoding filter as public api.
            match res.body().size_hint() {
                (low, Some(up)) if low == up && low < 64 => encoding = ContentEncoding::NoOp,
                // this variant is a crate hack. see NONE_BODY_HINT for detail.
                NONE_BODY_HINT => encoding = ContentEncoding::NoOp,
                _ => {}
            }

            Ok(encoder(res, encoding))
        }
    }

    impl<S> ReadyService for CompressService<S>
    where
        S: ReadyService,
    {
        type Ready = S::Ready;

        #[inline]
        async fn ready(&self) -> Self::Ready {
            self.0.ready().await
        }
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{App, handler::handler_service, http::WebRequest};

    use super::*;

    #[test]
    fn build() {
        async fn noop() -> &'static str {
            "noop"
        }

        App::new()
            .at("/", handler_service(noop))
            .enclosed(Compress)
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(WebRequest::default())
            .now_or_panic()
            .ok()
            .unwrap();
    }
}
