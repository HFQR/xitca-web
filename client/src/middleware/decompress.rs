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

pub struct Decompress<S> {
    service: S,
}

impl<S> Decompress<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }
}

impl<'r, 'c, S> Service<ServiceRequest<'r, 'c>> for Decompress<S>
where
    S: for<'r2, 'c2> Service<ServiceRequest<'r2, 'c2>, Response = Response<'c2>, Error = Error> + Send + Sync,
{
    type Response = Response<'c>;
    type Error = Error;

    async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
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
