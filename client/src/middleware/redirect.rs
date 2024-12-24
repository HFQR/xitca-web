use crate::{
    body::BoxBody,
    error::Error,
    http::{
        header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, TRANSFER_ENCODING},
        Method, StatusCode,
    },
    response::Response,
    service::{Service, ServiceRequest},
};

/// middleware for following redirect response.
pub struct FollowRedirect<S> {
    service: S,
}

impl<S> FollowRedirect<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }
}

impl<'r, 'c, S> Service<ServiceRequest<'r, 'c>> for FollowRedirect<S>
where
    S: for<'r2, 'c2> Service<ServiceRequest<'r2, 'c2>, Response = Response, Error = Error> + Send + Sync,
{
    type Response = Response;
    type Error = Error;

    async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
        let ServiceRequest { req, client, timeout } = req;
        let mut headers = req.headers().clone();
        let mut method = req.method().clone();
        let mut uri = req.uri().clone();
        loop {
            let mut res = self.service.call(ServiceRequest { req, client, timeout }).await?;
            match res.status() {
                StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND | StatusCode::SEE_OTHER => {
                    if method != Method::HEAD {
                        method = Method::GET;
                    }

                    *req.body_mut() = BoxBody::default();

                    for header in &[TRANSFER_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, CONTENT_LENGTH] {
                        headers.remove(header);
                    }
                }
                StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT => {}
                _ => return Ok(res),
            };

            let Some(location) = res.headers_mut().remove(LOCATION) else {
                return Ok(res);
            };

            uri = format!("{}{}", uri, location.to_str().unwrap()).parse().unwrap();

            *req.uri_mut() = uri.clone();
            *req.method_mut() = method.clone();
            *req.headers_mut() = headers.clone();
        }
    }
}
