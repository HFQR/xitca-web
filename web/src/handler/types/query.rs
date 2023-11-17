//! type extractor for request uri query

use core::fmt;

use serde::de::DeserializeOwned;

use crate::{
    body::BodyStream,
    handler::{
        error::{ExtractError, _ParseError},
        FromRequest,
    },
    request::WebRequest,
};

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
    T: DeserializeOwned,
    B: BodyStream,
{
    type Type<'b> = Query<T>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebRequest<'r, C, B>) -> Result<Self, Self::Error> {
        let value =
            serde_urlencoded::from_str(req.req().uri().query().unwrap_or_default()).map_err(_ParseError::UrlEncoded)?;
        Ok(Query(value))
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::http::Uri;

    use super::*;

    #[derive(serde::Deserialize)]
    struct Id {
        id: String,
    }

    #[test]
    fn query() {
        let mut req = WebRequest::new_test(());
        let mut req = req.as_web_req();

        *req.req_mut().uri_mut() = Uri::from_static("/996/251/?id=dagongren");

        let Query(id) = Query::<Id>::from_request(&req).now_or_panic().unwrap();

        assert_eq!(id.id, "dagongren");
    }
}
