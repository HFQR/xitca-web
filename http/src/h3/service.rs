use core::{fmt, net::SocketAddr};

use futures_core::Stream;
use tokio_util::sync::CancellationToken;
use xitca_io::net::QuicStream;
use xitca_service::{Service, ready::ReadyService};

use crate::{
    bytes::Bytes,
    error::HttpServiceError,
    http::{Request, RequestExt, Response},
};

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

impl<S, ResB, BE> Service<((QuicStream, SocketAddr), CancellationToken)> for H3Service<S>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;
    async fn call(
        &self,
        ((stream, addr), cancellation_token): ((QuicStream, SocketAddr), CancellationToken),
    ) -> Result<Self::Response, Self::Error> {
        let dispatcher = Dispatcher::new(stream, addr, &self.service, cancellation_token);

        dispatcher.run().await?;

        Ok(())
    }
}

impl<S> ReadyService for H3Service<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.service.ready().await
    }
}
