pub(crate) mod async_fn;
pub(crate) mod http;

use core::{future::Future, net::SocketAddr, pin::Pin, time::Duration};

use crate::{body::BoxBody, client::Client, http::Request};
pub use http::HttpService;

type BoxFuture<'f, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'f>>;

/// trait for composable http services. Used for middleware, resolver and tls connector.
pub trait Service<Req> {
    type Response;
    type Error;

    fn call(&self, req: Req) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;
}

mod _seal {
    #[doc(hidden)]
    /// dynamic compatible counterpart for [`Service`](crate::service::Service) trait.
    pub trait ServiceDyn<Req> {
        type Response;
        type Error;

        fn call<'s>(&'s self, req: Req) -> super::BoxFuture<'s, Self::Response, Self::Error>
        where
            Req: 's;
    }
}

pub(crate) use _seal::ServiceDyn;

impl<S, Req> ServiceDyn<Req> for S
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> BoxFuture<'s, Self::Response, Self::Error>
    where
        Req: 's,
    {
        Box::pin(Service::call(self, req))
    }
}

impl<I, Req> Service<Req> for Box<I>
where
    Req: Send,
    I: ServiceDyn<Req> + ?Sized + Send + Sync,
{
    type Response = I::Response;
    type Error = I::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        ServiceDyn::call(&**self, req).await
    }
}

/// request type for middlewares.
/// It's similar to [RequestBuilder] type but with additional side effect enabled.
///
/// [RequestBuilder]: crate::request::RequestBuilder
pub struct ServiceRequest<'r, 'c> {
    pub req: &'r mut Request<BoxBody>,
    pub client: &'c Client,
    pub address: Option<SocketAddr>,
    pub timeout: Duration,
}

#[cfg(test)]
pub(crate) use test::mock_service;

#[cfg(test)]
mod test {
    use core::time::Duration;

    use std::sync::Arc;

    use crate::{
        body::{BoxBody, ResponseBody},
        client::Client,
        error::Error,
        http::{self, Request},
        response::Response,
        service::{Service, ServiceRequest},
    };

    // http service and it's handle to make http service where a request and it's server side handler logic
    // is mocked on client side.
    pub(crate) fn mock_service() -> (HttpServiceMockHandle, HttpServiceMock) {
        (HttpServiceMockHandle(Client::new()), HttpServiceMock { _p: () })
    }

    pub(crate) struct HttpServiceMock {
        _p: (),
    }

    pub(crate) struct HttpServiceMockHandle(Client);

    type HandlerFn = Arc<dyn Fn(Request<BoxBody>) -> Result<http::Response<ResponseBody>, Error> + Send + Sync>;

    impl HttpServiceMockHandle {
        /// compose a service request with given http request and it's mocked server side handler function
        pub(crate) fn mock<'r, 'c>(
            &'c self,
            req: &'r mut Request<BoxBody>,
            handler: impl Fn(Request<BoxBody>) -> Result<http::Response<ResponseBody>, Error> + Send + Sync + 'static,
        ) -> ServiceRequest<'r, 'c> {
            req.extensions_mut().insert(Arc::new(handler) as HandlerFn);
            ServiceRequest {
                req,
                address: None,
                client: &self.0,
                timeout: self.0.timeout_config.request_timeout,
            }
        }
    }

    impl<'r, 'c> Service<ServiceRequest<'r, 'c>> for HttpServiceMock {
        type Response = Response;
        type Error = Error;

        async fn call(
            &self,
            ServiceRequest { req, timeout, .. }: ServiceRequest<'r, 'c>,
        ) -> Result<Self::Response, Self::Error> {
            let handler = req.extensions().get::<HandlerFn>().unwrap().clone();

            let res = handler(core::mem::take(req))?;

            Ok(Response::new(
                res,
                Box::pin(tokio::time::sleep(Duration::from_secs(0))),
                timeout,
            ))
        }
    }
}
