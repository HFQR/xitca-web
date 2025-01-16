use std::ops::{Deref, DerefMut};

use crate::{
    http::HeaderMap,
    service::{Service, ServiceRequest},
};

/// A default header map that can be used to append, replace or set headers if they are unset.
pub enum DefaultHeaderMap {
    Append(HeaderMap),
    Replace(HeaderMap),
    SetIfUnset(HeaderMap),
}

impl DefaultHeaderMap {
    pub fn new_append() -> Self {
        Self::Append(HeaderMap::new())
    }

    pub fn new_replace() -> Self {
        Self::Replace(HeaderMap::new())
    }

    pub fn new_set_if_unset() -> Self {
        Self::SetIfUnset(HeaderMap::new())
    }
}

impl Deref for DefaultHeaderMap {
    type Target = HeaderMap;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Append(headers) => headers,
            Self::Replace(headers) => headers,
            Self::SetIfUnset(headers) => headers,
        }
    }
}

impl DerefMut for DefaultHeaderMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Append(headers) => headers,
            Self::Replace(headers) => headers,
            Self::SetIfUnset(headers) => headers,
        }
    }
}

pub struct DefaultHeaders<S> {
    service: S,
    default_header_map: DefaultHeaderMap,
}

impl<S> DefaultHeaders<S> {
    pub fn new(service: S, default_header_map: DefaultHeaderMap) -> Self {
        Self {
            service,
            default_header_map,
        }
    }
}

impl<'r, 'c, S, Res, Err> Service<ServiceRequest<'r, 'c>> for DefaultHeaders<S>
where
    S: for<'r2, 'c2> Service<ServiceRequest<'r2, 'c2>, Response = Res, Error = Err> + Send + Sync,
{
    type Response = Res;
    type Error = Err;

    async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
        match &self.default_header_map {
            DefaultHeaderMap::Append(headers) => {
                for (key, value) in headers {
                    req.req.headers_mut().append(key, value.clone());
                }
            }
            DefaultHeaderMap::Replace(headers) => {
                for (key, value) in headers {
                    req.req.headers_mut().insert(key, value.clone());
                }
            }
            DefaultHeaderMap::SetIfUnset(headers) => {
                for (key, value) in headers {
                    req.req.headers_mut().entry(key).or_insert(value.clone());
                }
            }
        }

        self.service.call(req).await
    }
}

#[cfg(test)]
mod test {
    use crate::Client;
    use xitca_http::http::HeaderValue;

    use super::*;

    #[tokio::test]
    async fn build_default_headers_mw() {
        let mut default_headers = DefaultHeaderMap::new_append();
        default_headers.insert("content-type", HeaderValue::from_static("application/json"));

        let _ = Client::builder()
            .middleware(move |x| DefaultHeaders::new(x, default_headers))
            .finish();
    }
}
