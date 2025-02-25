use http_encoding::try_decoder;

use crate::{
    body::ResponseBody,
    error::Error,
    http::{
        self,
        header::{HeaderValue, ACCEPT_ENCODING},
    },
    response::Response,
    service::{Service, ServiceRequest},
};

/// middleware handling compressed http response body and emit decompressed data.
pub struct Decompress<S> {
    service: S,
}

impl<S> Decompress<S> {
    /// construct a new decompress middleware with given http service type.
    pub const fn new(service: S) -> Self {
        Self { service }
    }
}

impl<'c, S> Service<ServiceRequest<'c>> for Decompress<S>
where
    S: for<'c2> Service<ServiceRequest<'c2>, Response = Response, Error = Error> + Send + Sync,
{
    type Response = Response;
    type Error = Error;

    async fn call(&self, mut req: ServiceRequest<'c>) -> Result<Self::Response, Self::Error> {
        req.req
            .headers_mut()
            .insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip, deflate, br"));

        let mut res = self.service.call(req).await?;

        let (parts, body) = res.res.into_parts();
        let body = try_decoder(&parts.headers, body).map_err(|e| Error::Std(Box::new(e)))?;
        res.res = http::Response::from_parts(parts, ResponseBody::Unknown(Box::pin(body)));
        Ok(res)
    }
}

#[cfg(test)]
mod test {
    use crate::Client;

    use super::*;

    #[tokio::test]
    async fn build_compress_mw() {
        let _ = Client::builder().middleware(Decompress::new).finish();
    }
}
