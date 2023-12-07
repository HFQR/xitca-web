use core::convert::Infallible;

use http_encoding::{encoder, Coder, ContentEncoding};

use crate::{
    body::{BodyStream, NONE_BODY_HINT},
    dev::service::{ready::ReadyService, Service},
    http::{header::HeaderMap, BorrowReq, WebResponse},
};

/// A compress middleware look into [WebRequest]'s `Accept-Encoding` header and
/// apply according compression to [WebResponse]'s body according to enabled compress feature.
/// `compress-x` feature must be enabled for this middleware to function correctly.
#[derive(Clone)]
pub struct Compress;

impl<S> Service<S> for Compress {
    type Response = CompressService<S>;
    type Error = Infallible;

    async fn call(&self, service: S) -> Result<Self::Response, Self::Error> {
        Ok(CompressService { service })
    }
}

pub struct CompressService<S> {
    service: S,
}

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
        let res = self.service.call(req).await?;

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
        self.service.ready().await
    }
}
