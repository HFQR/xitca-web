use std::{fmt, future::Future};

use futures_core::Stream;
use xitca_io::net::UdpStream;
use xitca_service::{ready::ReadyService, Service};

use crate::{bytes::Bytes, error::HttpServiceError, http::Response, request::Request};

use super::{body::RequestBody, proto::Dispatcher};

pub struct H3Service<S> {
    service: S,
}

impl<S> H3Service<S> {
    /// Construct new Http3Service.
    /// No upgrade/expect services allowed in Http/3.
    pub fn new(service: S) -> Self {
        Self { service }
    }
}

impl<S, ResB, BE> Service<UdpStream> for H3Service<S>
where
    S: Service<Request<RequestBody>, Response = Response<ResB>> + 'static,
    S::Error: fmt::Debug,

    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, stream: UdpStream) -> Self::Future<'_> {
        async move {
            let dispatcher = Dispatcher::new(stream, &self.service);

            dispatcher.run().await?;

            Ok(())
        }
    }
}

impl<S, ResB, BE> ReadyService<UdpStream> for H3Service<S>
where
    S: ReadyService<Request<RequestBody>, Response = Response<ResB>> + 'static,
    S::Error: fmt::Debug,

    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f>;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}
