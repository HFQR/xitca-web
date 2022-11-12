use std::{fmt, future::Future, net::SocketAddr};

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

impl<S, ResB, BE> Service<(UdpStream, SocketAddr)> for H3Service<S>
where
    S: Service<Request<RequestBody>, Response = Response<ResB>>,
    S::Error: fmt::Debug,

    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f;

    fn call<'s>(&'s self, (stream, addr): (UdpStream, SocketAddr)) -> Self::Future<'s>
    where
        UdpStream: 's,
    {
        async move {
            let dispatcher = Dispatcher::new(stream, addr.into(), &self.service);

            dispatcher.run().await?;

            Ok(())
        }
    }
}

impl<S> ReadyService for H3Service<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}
