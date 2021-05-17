use std::{
    future::Future,
    io,
    task::{Context, Poll},
};

use actix_server_alt::net::TcpStream;
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::{ready, Stream};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlow;
use crate::request::HttpRequest;
use crate::response::{HttpResponse, ResponseError};

use super::body::RequestBody;
use super::proto::Dispatcher;

pub struct H1Service<S, X, U> {
    flow: HttpFlow<S, X, U>,
}

impl<S, X, U> H1Service<S, X, U> {
    /// Construct new Http1Service.
    pub fn new(service: S, expect: X, upgrade: U) -> Self {
        Self {
            flow: HttpFlow::new(service, expect, upgrade),
        }
    }
}

#[rustfmt::skip]
impl<S, X, U, B, E> Service for H1Service<S, X, U>
where
    S: for<'r> Service<
            Request<'r> = HttpRequest<RequestBody>,
            Response = HttpResponse<ResponseBody<B>>,
        > + 'static,
    S::Error: ResponseError<S::Response>,

    X: 'static,

    U: 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    type Request<'r> = TcpStream;
    type Response = ();
    type Error = HttpServiceError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // ready!(self.flow.upgrade.poll_ready(cx))

        self.flow
            .service
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady)
    }

    fn call<'s>(&'s self, io: Self::Request<'s>) -> Self::Future<'s> {
        async move {
            let mut dispatcher = Dispatcher::new(io, &self.flow);

            dispatcher.run().await?;

            Ok(())
        }
    }
}
