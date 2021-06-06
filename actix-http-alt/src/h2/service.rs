use std::{
    future::Future,
    task::{Context, Poll},
};

use ::h2::server::handshake;
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::{ready, Stream};
use http::{Request, Response};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlow;
use crate::response::ResponseError;

use super::body::RequestBody;
use super::proto::Dispatcher;

pub struct H2Service<S, A> {
    flow: HttpFlow<S, (), ()>,
    tls_acceptor: A,
}

impl<S, A> H2Service<S, A> {
    /// Construct new Http2Service.
    /// No upgrade/expect services allowed in Http/2.
    pub fn new(service: S, tls_acceptor: A) -> Self {
        Self {
            flow: HttpFlow::new(service, (), ()),
            tls_acceptor,
        }
    }
}

impl<St, S, B, E, A, TlsSt> Service<St> for H2Service<S, A>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    A: Service<St, Response = TlsSt> + 'static,

    S::Error: ResponseError<S::Response>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncRead + AsyncWrite + Unpin,
    TlsSt: AsyncRead + AsyncWrite + Unpin,

    HttpServiceError: From<A::Error>,
{
    type Response = ();
    type Error = HttpServiceError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self
            .tls_acceptor
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady))?;

        self.flow
            .service
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady)
    }

    fn call<'c>(&'c self, req: St) -> Self::Future<'c>
    where
        St: 'c,
    {
        async move {
            let tls_stream = self.tls_acceptor.call(req).await?;

            let mut conn = handshake(tls_stream).await?;

            let dispatcher = Dispatcher::new(&mut conn, &self.flow);

            dispatcher.run().await
        }
    }
}
