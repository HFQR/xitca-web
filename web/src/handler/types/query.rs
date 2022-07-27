use std::{convert::Infallible, fmt, future::Future};

use crate::{handler::FromRequest, request::WebRequest};

pub struct Query<T>(pub T);

impl<T> fmt::Debug for Query<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Query").field("value", &self.0).finish()
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebRequest<'r, C, B>> for Query<T>
where
    T: serde::de::DeserializeOwned,
{
    type Type<'b> = Query<T>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async move {
            let value = serde_urlencoded::from_str(req.req().uri().query().unwrap_or_default()).unwrap();
            Ok(Query(value))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::http::Uri;

    #[derive(serde::Deserialize)]
    struct Id {
        id: String,
    }

    #[tokio::test]
    async fn query() {
        let mut req = WebRequest::new_test(());
        let mut req = req.as_web_req();

        *req.req_mut().uri_mut() = Uri::from_static("/996/251/?id=dagongren");

        let Query(id) = Query::<Id>::from_request(&req).await.unwrap();

        assert_eq!(id.id, "dagongren");
    }
}
