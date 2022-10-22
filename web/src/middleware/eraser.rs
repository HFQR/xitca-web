use std::{convert::Infallible, error, future::Future, marker::PhantomData};

use xitca_http::ResponseBody;

use crate::{
    dev::{
        bytes::Bytes,
        service::{ready::ReadyService, Service},
    },
    request::WebRequest,
    response::{StreamBody, WebResponse},
    stream::WebStream,
};

#[doc(hidden)]
mod marker {
    // pub struct EraseReqBody;
    pub struct EraseResBody;

    pub struct EraseErr;
}

use marker::*;

pub struct TypeEraser<M>(PhantomData<M>);

impl<M> Clone for TypeEraser<M> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<M> TypeEraser<M> {
    fn new() -> Self {
        TypeEraser(PhantomData)
    }
}

// impl TypeEraser<EraseReqBody> {
//     // Erase generic B type param from WebRequest<'_, C, B>. making downstream middlewares observe WebRequest<'_, C> type.
//     pub fn request_body() -> Self {
//         TypeEraser::new()
//     }
// }

impl TypeEraser<EraseResBody> {
    // Erase generic B type param from WebResponse<B>. making downstream middlewares observe WebResponse type.
    pub fn response_body() -> Self {
        TypeEraser::new()
    }
}

impl TypeEraser<EraseErr> {
    // Erase generic E type from Service::Error = E. making downstream middlewares observe Box<dyn std::error::Error + Send + Sync>
    // as Service::Error type.
    pub fn error() -> Self {
        TypeEraser::new()
    }
}

impl<M, S> Service<S> for TypeEraser<M> {
    type Response = EraserService<M, S>;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, S: 'f;

    fn call<'s, 'f>(&'s self, service: S) -> Self::Future<'f>
    where
        's: 'f,
        S: 'f,
    {
        async {
            Ok(EraserService {
                service,
                _erase: PhantomData,
            })
        }
    }
}

pub struct EraserService<M, S> {
    service: S,
    _erase: PhantomData<M>,
}

impl<'r, S, C, B, ResB, Err> Service<WebRequest<'r, C, B>> for EraserService<EraseResBody, S>
where
    S: for<'rs> Service<WebRequest<'rs, C, B>, Response = WebResponse<ResB>, Error = Err>,
    C: 'r,
    B: 'r,
    ResB: WebStream<Chunk = Bytes> + 'static,
    <ResB as WebStream>::Error: Send + Sync,
{
    type Response = WebResponse;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f
    where
        Self: 'f, 'r: 'f;

    #[inline]
    fn call<'s, 'f>(&'s self, req: WebRequest<'r, C, B>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
        async {
            let res = self.service.call(req).await?;
            Ok(res.map(|b| ResponseBody::stream(StreamBody::new(b))))
        }
    }
}

impl<S, Req> Service<Req> for EraserService<EraseErr, S>
where
    S: Service<Req>,
    S::Error: error::Error + Send + Sync + 'static,
{
    type Response = S::Response;
    type Error = Box<dyn error::Error + Send + Sync>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f
    where
        Self: 'f, Req: 'f;

    #[inline]
    fn call<'s, 'f>(&'s self, req: Req) -> Self::Future<'f>
    where
        's: 'f,
        Req: 'f,
    {
        async { self.service.call(req).await.map_err(|e| Box::new(e) as _) }
    }
}

impl<M, S> ReadyService for EraserService<M, S>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}

#[cfg(test)]
mod test {
    use xitca_http::{body::Once, request::Request};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{dev::service::ServiceExt, handler::handler_service, App};

    use super::*;

    async fn handler(_: &WebRequest<'_>) -> &'static str {
        "996"
    }

    async fn map_body<S, C, B, Err>(_: &S, _: WebRequest<'_, C, B>) -> Result<WebResponse<Once<Bytes>>, Err>
    where
        S: for<'r> Service<WebRequest<'r, C, B>, Response = WebResponse, Error = Err>,
    {
        Ok(WebResponse::new(Once::new(Bytes::new())))
    }

    async fn middleware_fn<S, C, B, Err>(s: &S, req: WebRequest<'_, C, B>) -> Result<WebResponse, Err>
    where
        S: for<'r> Service<WebRequest<'r, C, B>, Response = WebResponse, Error = Err>,
    {
        s.call(req).await
    }

    #[test]
    fn erase_body() {
        let _ = App::new()
            // map WebResponse to WebResponse<Once<Bytes>> type.
            .at("/", handler_service(handler).enclosed_fn(map_body))
            // earse the body type to make it WebResponse type again.
            .enclosed(TypeEraser::response_body())
            // observe erased body type.
            .enclosed_fn(middleware_fn)
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();
    }
}
