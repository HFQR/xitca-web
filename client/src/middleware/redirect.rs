use crate::{
    body::BoxBody,
    error::{Error, InvalidUri},
    http::{
        header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, TRANSFER_ENCODING},
        Method, StatusCode, Uri,
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
        let version = req.version().clone();

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

            let location_uri = location.to_str().map_err(|e| Error::Std(Box::new(e)))?.parse::<Uri>()?;

            if location_uri.scheme().is_some() {
                // location is an absolute uri used as the new request uri.
                uri = location_uri;
            } else if location_uri.path().starts_with("/") {
                // location is an absolute path, merge it with current scheme and authority.
                uri = format!(
                    "{}://{}{}",
                    uri.scheme_str().ok_or(Error::InvalidUri(InvalidUri::MissingScheme))?,
                    uri.authority().ok_or(Error::InvalidUri(InvalidUri::MissingAuthority))?,
                    location_uri
                        .path_and_query()
                        .ok_or(Error::InvalidUri(InvalidUri::MissingPathQuery))?
                )
                .parse()?;
            } else {
                // location is a relative path, merge it with current scheme, authority and path.
                uri = format!(
                    "{}{}",
                    uri,
                    location_uri
                        .path_and_query()
                        .ok_or(Error::InvalidUri(InvalidUri::MissingPathQuery))?
                )
                .parse()?;
            }

            *req.version_mut() = version.clone();
            *req.uri_mut() = uri.clone();
            *req.method_mut() = method.clone();
            *req.headers_mut() = headers.clone();
        }
    }
}
