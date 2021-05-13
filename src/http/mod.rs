mod h2;
mod service;

use std::{fmt::Debug, future::Future};

use ::h2::RecvStream;
use bytes::Bytes;
use http::request::Parts;
use http::Response;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::service::ServiceFactory;

use self::h2::Http2Service;

pub struct HttpServiceBuilder<F> {
    factory: F,
}

impl<F> HttpServiceBuilder<F>
where
    F: ServiceFactory<(Parts, RecvStream), Response = Response<Bytes>, Config = ()>,
    F::Service: 'static,
    F::Error: Debug,
{
    pub fn new(factory: F) -> Self {
        Self { factory }
    }

    pub fn finish(self) -> impl ServiceFactory<TcpStream, Config = ()> {
        self
    }
}

impl<F, St> ServiceFactory<St> for HttpServiceBuilder<F>
where
    F: ServiceFactory<(Parts, RecvStream), Response = Response<Bytes>, Config = ()>,
    F::Service: 'static,
    F::Error: Debug,
    St: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = ();
    type Error = ();
    type Config = ();
    type Service = Http2Service<F::Service, St>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let service = self.factory.new_service(());
        async {
            let service = match service.await {
                Ok(service) => service,
                Err(_) => panic!("TODO"),
            };

            Ok(Http2Service::new(service))
        }
    }
}
