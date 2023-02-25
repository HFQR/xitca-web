use core::{convert::Infallible, future::Future};

use http_encoding::{encoder, Coder, ContentEncoding};

use crate::{
    body::BodyStream,
    dev::service::{ready::ReadyService, Service},
    request::WebRequest,
    response::WebResponse,
};

/// A compress middleware look into [WebRequest]'s `Accept-Encoding` header and
/// apply according compression to [WebResponse]'s body according to enabled compress feature.
/// `compress-x` feature must be enabled for this middleware to function correctly.
#[derive(Clone)]
pub struct Compress;

impl<S> Service<S> for Compress {
    type Response = CompressService<S>;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where S: 'f;

    fn call<'s>(&self, service: S) -> Self::Future<'s>
    where
        S: 's,
    {
        async { Ok(CompressService { service }) }
    }
}

pub struct CompressService<S> {
    service: S,
}

impl<'r, S, C, ReqB, ResB, Err> Service<WebRequest<'r, C, ReqB>> for CompressService<S>
where
    C: 'r,
    ReqB: 'r,
    S: for<'rs> Service<WebRequest<'rs, C, ReqB>, Response = WebResponse<ResB>, Error = Err>,
    ResB: BodyStream,
{
    type Response = WebResponse<Coder<ResB>>;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, 'r: 'f;

    fn call<'s>(&'s self, req: WebRequest<'r, C, ReqB>) -> Self::Future<'s>
    where
        'r: 's,
    {
        async {
            let mut encoding = ContentEncoding::from_headers(req.req().headers());
            let res = self.service.call(req).await?;

            // TODO: expose encoding filter as public api.
            match res.body().size_hint() {
                (low, Some(up)) if low == up && low < 64 => encoding = ContentEncoding::NoOp,
                // this variant is a crate hack. see xitca_http::body::none_body_hint for detail.
                (usize::MAX, Some(0)) => encoding = ContentEncoding::NoOp,
                _ => {}
            }

            Ok(encoder(res, encoding))
        }
    }
}

impl<S> ReadyService for CompressService<S>
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
