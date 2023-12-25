//! type extractor for request uri query

use core::fmt;

use serde::de::Deserialize;

use crate::{
    body::BodyStream,
    context::WebContext,
    error::{BadRequest, Error},
    handler::FromRequest,
};

use super::lazy::Lazy;

pub struct Query<T>(pub T);

impl<T> fmt::Debug for Query<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Query").field("value", &self.0).finish()
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for Query<T>
where
    T: for<'de> Deserialize<'de>,
    B: BodyStream,
{
    type Type<'b> = Query<T>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        serde_urlencoded::from_str(ctx.req().uri().query().unwrap_or_default())
            .map(Query)
            .map_err(Error::from_service)
    }
}

impl<T> Lazy<'_, Query<T>> {
    pub fn deserialize<'de, C>(&'de self) -> Result<T, Error<C>>
    where
        T: Deserialize<'de>,
    {
        serde_urlencoded::from_bytes(self.as_slice()).map_err(Into::into)
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for Lazy<'a, Query<T>>
where
    B: BodyStream,
    T: Deserialize<'static>,
{
    type Type<'b> = Lazy<'b, Query<T>>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let query = ctx.req().uri().query().ok_or(BadRequest)?;
        Ok(Lazy::from(query.as_bytes()))
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{handler::handler_service, http::Uri, service::Service, test::collect_string_body};

    use super::*;

    #[derive(serde::Deserialize)]
    struct Id {
        id: String,
    }

    #[derive(serde::Deserialize)]
    struct Id2<'a> {
        id: &'a str,
    }

    #[test]
    fn query() {
        let mut req = WebContext::new_test(());
        let mut req = req.as_web_ctx();

        *req.req_mut().uri_mut() = Uri::from_static("/996/251/?id=dagongren");

        let Query(id) = Query::<Id>::from_request(&req).now_or_panic().unwrap();
        assert_eq!(id.id, "dagongren");
    }

    #[test]
    fn query_lazy() {
        let mut ctx = WebContext::new_test(());
        let mut ctx = ctx.as_web_ctx();

        *ctx.req_mut().uri_mut() = Uri::from_static("/996/251/?id=dagongren");

        async fn handler(lazy: Lazy<'_, Query<Id2<'_>>>) -> &'static str {
            let id = lazy.deserialize::<()>().unwrap();
            assert_eq!(id.id, "dagongren");
            "kubi"
        }

        let service = handler_service(handler).call(()).now_or_panic().unwrap();

        let body = service.call(ctx).now_or_panic().unwrap().into_body();
        let res = collect_string_body(body).now_or_panic().unwrap();

        assert_eq!(res, "kubi");
    }
}
