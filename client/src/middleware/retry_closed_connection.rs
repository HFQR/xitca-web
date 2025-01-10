use std::io;

use crate::{
    error::Error,
    response::Response,
    service::{Service, ServiceRequest},
};

/// middleware for retrying closed connection
pub struct RetryClosedConnection<S> {
    service: S,
}

impl<S> RetryClosedConnection<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }
}

impl<'r, 'c, S> Service<ServiceRequest<'r, 'c>> for RetryClosedConnection<S>
where
    S: for<'r2, 'c2> Service<ServiceRequest<'r2, 'c2>, Response = Response, Error = Error> + Send + Sync,
{
    type Response = Response;
    type Error = Error;

    async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
        let ServiceRequest { req, client, timeout } = req;
        let headers = req.headers().clone();
        let method = req.method().clone();
        let uri = req.uri().clone();

        loop {
            let res = self.service.call(ServiceRequest { req, client, timeout }).await;

            match res {
                Err(Error::Io(err)) => {
                    if err.kind() != io::ErrorKind::UnexpectedEof {
                        return Err(Error::Io(err));
                    }
                }
                #[cfg(feature = "http1")]
                Err(Error::H1(crate::h1::Error::Io(err))) => {
                    if err.kind() != io::ErrorKind::UnexpectedEof {
                        return Err(Error::H1(crate::h1::Error::Io(err)));
                    }
                }
                #[cfg(feature = "http2")]
                Err(Error::H2(crate::h2::Error::H2(err))) => {
                    if !err.is_go_away() {
                        return Err(Error::H2(crate::h2::Error::H2(err)));
                    }

                    let reason = err.reason().unwrap();

                    if reason != h2::Reason::NO_ERROR {
                        return Err(Error::H2(crate::h2::Error::H2(err)));
                    }
                }
                #[cfg(feature = "http2")]
                Err(Error::H2(crate::h2::Error::Io(err))) => {
                    if err.kind() != io::ErrorKind::UnexpectedEof {
                        return Err(Error::H2(crate::h2::Error::Io(err)));
                    }
                }
                #[cfg(feature = "http3")]
                Err(Error::H3(crate::h3::Error::Io(err))) => {
                    if err.kind() != io::ErrorKind::UnexpectedEof {
                        return Err(Error::H3(crate::h3::Error::Io(err)));
                    }
                }
                res => return res,
            }

            *req.uri_mut() = uri.clone();
            *req.method_mut() = method.clone();
            *req.headers_mut() = headers.clone();
        }
    }
}
