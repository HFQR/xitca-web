use std::{
    future::Future,
    task::{Context, Poll},
};

use actix_server_alt::net::TcpStream;
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::{ready, Stream};
use http::{Request, Response};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlow;
use crate::response::ResponseError;
use crate::util::date::DateTask;

use super::body::RequestBody;
use super::error::Error;
use super::proto::Dispatcher;

pub struct H1Service<S, X, U> {
    date: DateTask,
    flow: HttpFlow<S, X, U>,
}

impl<S, X, U> H1Service<S, X, U> {
    /// Construct new Http1Service.
    pub fn new(service: S, expect: X, upgrade: U) -> Self {
        Self {
            date: DateTask::new(),
            flow: HttpFlow::new(service, expect, upgrade),
        }
    }
}

impl<S, X, U, B, E> Service<TcpStream> for H1Service<S, X, U>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    X::Error: ResponseError<S::Response>,

    U: 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    type Response = ();
    type Error = HttpServiceError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // ready!(self
        //     .flow
        //     .upgrade
        //     .poll_ready(cx)
        //     .map_err(|_| HttpServiceError::ServiceReady))?;
        //
        ready!(self
            .flow
            .expect
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady))?;

        self.flow
            .service
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady)
    }

    fn call<'c>(&'c self, mut io: TcpStream) -> Self::Future<'c>
    where
        TcpStream: 'c,
    {
        async move {
            let mut dispatcher = Dispatcher::new(&mut io, &self.flow, &self.date);

            match dispatcher.run().await {
                Ok(_) | Err(Error::Closed) => Ok(()),
                Err(e) => Err(e.into()),
            }
        }
    }
}
