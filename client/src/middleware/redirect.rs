use crate::{
    body::BoxBody,
    error::{Error, InvalidUri},
    http::{
        header::{
            AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, COOKIE, LOCATION, PROXY_AUTHORIZATION,
            TRANSFER_ENCODING,
        },
        Method, StatusCode, Uri,
    },
    response::Response,
    service::{Service, ServiceRequest},
};

/// middleware for following redirect response.
pub struct FollowRedirect<S, const MAX_COUNT: usize = 10> {
    service: S,
}

impl<S> FollowRedirect<S> {
    /// construct redirect following middleware for client.
    ///
    /// # Examples:
    /// ```rust
    /// # use xitca_client::{ClientBuilder, middleware::FollowRedirect};
    /// let builder = ClientBuilder::new()
    ///     .middleware(FollowRedirect::new);
    /// ```
    pub const fn new(service: S) -> Self {
        Self { service }
    }
}

impl<S, const MAX: usize> FollowRedirect<S, MAX> {
    /// set max depth of redirect following for request. when max value is reached the redirect following
    /// would stop and the most recent response will be returned as output.
    ///
    /// Default to 10 times.
    pub fn max<const MAX2: usize>(self) -> FollowRedirect<S, MAX2> {
        FollowRedirect { service: self.service }
    }
}

impl<'r, 'c, S, const MAX: usize> Service<ServiceRequest<'r, 'c>> for FollowRedirect<S, MAX>
where
    S: for<'r2, 'c2> Service<ServiceRequest<'r2, 'c2>, Response = Response, Error = Error> + Send + Sync,
{
    type Response = Response;
    type Error = Error;

    async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
        let ServiceRequest {
            req,
            client,
            request_timeout,
            response_timeout,
        } = req;
        let mut headers = req.headers().clone();
        let mut method = req.method().clone();
        let mut uri = req.uri().clone();
        let ext = req.extensions().clone();
        let mut count = 0;

        loop {
            let mut res = self
                .service
                .call(ServiceRequest {
                    req,
                    client,
                    request_timeout,
                    response_timeout,
                })
                .await?;

            if count == MAX {
                return Ok(res);
            }

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

            let parts = uri.into_parts();

            let parts_location = location
                .to_str()
                .map_err(|_| InvalidUri::MissingPathQuery)?
                .parse::<Uri>()?
                .into_parts();

            // remove authenticated headers when redirected to different scheme/authority
            if parts_location.scheme != parts.scheme || parts_location.authority != parts.authority {
                headers.remove(AUTHORIZATION);
                headers.remove(PROXY_AUTHORIZATION);
                headers.remove(COOKIE);
            }

            let mut uri_builder = Uri::builder();

            if let Some(a) = parts_location.authority.or(parts.authority) {
                uri_builder = uri_builder.authority(a);
            }

            if let Some(s) = parts_location.scheme.or(parts.scheme) {
                uri_builder = uri_builder.scheme(s);
            }

            let path = parts_location.path_and_query.ok_or(InvalidUri::MissingPathQuery)?;
            uri = uri_builder.path_and_query(path).build().unwrap();

            *req.uri_mut() = uri.clone();
            *req.method_mut() = method.clone();
            *req.headers_mut() = headers.clone();
            *req.extensions_mut() = ext.clone();

            count += 1;
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{
        body::ResponseBody,
        http,
        service::{mock_service, Service},
    };

    use super::*;

    #[tokio::test]
    async fn redirect() {
        let (handle, service) = mock_service();

        let redirect = FollowRedirect::new(service).max::<1>();

        let handler = |req: http::Request<BoxBody>| match req.uri().path() {
            "/foo" => Ok(http::Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header("location", "/bar")
                .body(ResponseBody::Eof)
                .unwrap()),
            "/bar" => Ok(http::Response::builder()
                .status(StatusCode::IM_A_TEAPOT)
                .body(ResponseBody::Eof)
                .unwrap()),
            "/fur" => Ok(http::Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header("location", "/foo")
                .body(ResponseBody::Eof)
                .unwrap()),
            p => panic!("unexpected uri path: {p}"),
        };

        let mut req = http::Request::builder()
            .uri("http://foo.bar/foo")
            .body(Default::default())
            .unwrap();

        let req = handle.mock(&mut req, handler);
        let res = redirect.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::IM_A_TEAPOT);

        let mut req = http::Request::builder()
            .uri("http://foo.bar/fur")
            .body(Default::default())
            .unwrap();

        let req = handle.mock(&mut req, handler);
        let res = redirect.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get(LOCATION).unwrap().to_str().unwrap(), "/bar");
    }
}
