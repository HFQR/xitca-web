use crate::{
    error::Error,
    response::Response,
    service::{Service, ServiceRequest},
};

/// middleware for retrying closed connection
pub struct RetryClosedConnection<S, const MAX_COUNT: usize = 3> {
    service: S,
}

impl<S> RetryClosedConnection<S> {
    /// construct retry closed connection middleware for client.
    ///
    /// # Examples:
    /// ```rust
    /// # use xitca_client::{ClientBuilder, middleware::RetryClosedConnection};
    /// let builder = ClientBuilder::new()
    ///     .middleware(RetryClosedConnection::new);
    /// ```
    pub fn new(service: S) -> Self {
        Self { service }
    }
}

impl<S, const MAX: usize> RetryClosedConnection<S, MAX> {
    /// set max retry count for request. when max value is reached the request will return the most recent errror.
    pub fn max<const MAX2: usize>(self) -> RetryClosedConnection<S, MAX2> {
        RetryClosedConnection { service: self.service }
    }
}

impl<'r, 'c, S, const MAX: usize> Service<ServiceRequest<'r, 'c>> for RetryClosedConnection<S, MAX>
where
    S: for<'r2, 'c2> Service<ServiceRequest<'r2, 'c2>, Response = Response, Error = Error> + Send + Sync,
{
    type Response = Response;
    type Error = Error;

    async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
        let ServiceRequest { req, client, timeout } = req;
        let mut count = 0;

        loop {
            let res = self.service.call(ServiceRequest { req, client, timeout }).await;

            if count == MAX {
                return res;
            }

            match res {
                #[cfg(feature = "http1")]
                Err(Error::H1(crate::h1::Error::UnexpectedState(
                    crate::h1::UnexpectedStateError::ConnectionClosed,
                ))) => (),
                res => return res,
            }

            count += 1;
        }
    }
}
