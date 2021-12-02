use std::future::Future;

use xitca_http::{body::RequestBody, http::Request};
use xitca_service::{Service, ServiceFactory};

use crate::request::WebRequest;

/// Adaptor service for extracting [Request] from [WebRequest] and pass reference to inner service type.
/// Used to bridge xitca-web and xitca-http.
pub struct HttpServiceAdaptor<F>(F);

impl<F> HttpServiceAdaptor<F> {
    pub const fn new(factory: F) -> Self {
        Self(factory)
    }
}

impl<'r, D, F> ServiceFactory<&'r mut WebRequest<'_, D>> for HttpServiceAdaptor<F>
where
    F: ServiceFactory<&'r mut Request<RequestBody>>,
    F::Service: 'static,
{
    type Response = F::Response;
    type Error = F::Error;
    type Config = F::Config;
    type Service = HttpService<F::Service>;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let factory = self.0.new_service(cfg);
        async move {
            let service = factory.await?;
            Ok(HttpService(service))
        }
    }
}

pub struct HttpService<S>(S);

impl<'r, D, S> Service<&'r mut WebRequest<'_, D>> for HttpService<S>
where
    S: Service<&'r mut Request<RequestBody>> + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Ready<'f> = S::Ready<'f>;
    type Future<'f> = S::Future<'f>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        self.0.ready()
    }

    #[inline]
    fn call(&self, req: &'r mut WebRequest<'_, D>) -> Self::Future<'_> {
        self.0.call(req.request_mut())
    }
}
