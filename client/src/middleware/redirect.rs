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
        let ext = req.extensions().clone();
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

            let uri_location = location
                .to_str()
                .map_err(|_| InvalidUri::MissingPathQuery)?
                .parse::<Uri>()?;

            let mut uri_builder = Uri::builder();

            if let Some(a) = uri_location.authority() {
                uri_builder = uri_builder.authority(a.clone());
            } else if let Some(a) = uri.authority() {
                uri_builder = uri_builder.authority(a.clone());
            }

            if let Some(s) = uri_location.scheme() {
                uri_builder = uri_builder.scheme(s.clone());
            } else if let Some(s) = uri.scheme() {
                uri_builder = uri_builder.scheme(s.clone());
            }

            let path = uri_location.path_and_query().ok_or(InvalidUri::MissingPathQuery)?;
            uri = uri_builder.path_and_query(path.clone()).build().unwrap();

            *req.uri_mut() = uri.clone();
            *req.method_mut() = method.clone();
            *req.headers_mut() = headers.clone();
            *req.extensions_mut() = ext.clone();
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

        let redirect = FollowRedirect::new(service);

        let mut req = http::Request::builder()
            .uri("http://foo.bar/foo")
            .body(Default::default())
            .unwrap();

        let req = handle.mock(&mut req, |req| match req.uri().path() {
            "/foo" => Ok(http::Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header("location", "/bar")
                .body(ResponseBody::Eof)
                .unwrap()),
            "/bar" => Ok(http::Response::builder()
                .status(StatusCode::IM_A_TEAPOT)
                .body(ResponseBody::Eof)
                .unwrap()),
            p => panic!("unexpected uri path: {p}"),
        });

        let res = redirect.call(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::IM_A_TEAPOT);
    }
}
