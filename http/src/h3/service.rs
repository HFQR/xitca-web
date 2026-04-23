use core::{fmt, net::SocketAddr};

use std::sync::Arc;

use crate::body::Body;
use xitca_io::net::QuicStream;
use xitca_service::{Service, ready::ReadyService, shutdown::ShutdownToken};

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

impl<S, ResB, BE> Service<((QuicStream, SocketAddr), Arc<ShutdownToken>)> for H3Service<S>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ResB: Body<Data = Bytes, Error = BE>,
    BE: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;
    async fn call(
        &self,
        ((stream, addr), st): ((QuicStream, SocketAddr), Arc<ShutdownToken>),
    ) -> Result<Self::Response, Self::Error> {
        let dispatcher = Dispatcher::new(stream, addr, &self.service);

        dispatcher.run(st.as_ref()).await?;

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
