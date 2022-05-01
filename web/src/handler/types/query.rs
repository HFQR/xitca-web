use std::{convert::Infallible, future::Future};

use serde::Deserialize;

use crate::{handler::FromRequest, request::WebRequest};

pub struct Query<T>(pub T);

impl<'a, 'r, S, T> FromRequest<'a, WebRequest<'r, S>> for Query<T>
where
    S: 'r,
    T: for<'de> Deserialize<'de>,
{
    type Type<'b> = Query<T>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    fn from_request(req: &'a WebRequest<'r, S>) -> Self::Future {
        async move {
            let value = serde_urlencoded::from_str::<T>(req.req().uri().query().unwrap_or_default()).unwrap();
            Ok(Query(value))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::http::Uri;

    #[derive(Deserialize)]
    struct Id<'a> {
        id: &'a str,
    }

    #[tokio::test]
    async fn query() {
        let mut req = WebRequest::new_test(());
        let mut req = req.as_web_req();

        *req.req_mut().uri_mut() = Uri::from_static("/996/251/?id=dagongren");

        let Query(id) = Query::<Id<'_>>::from_request(&req).await.unwrap();

        assert_eq!(id.id, "dagongren");
    }
}
