//! type extractor for request uri query

use core::{fmt, marker::PhantomData};

use serde_core::de::Deserialize;

use crate::{
    context::WebContext,
    error::{Error, ErrorStatus},
    handler::FromRequest,
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

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for Query<T>
where
    T: for<'de> Deserialize<'de>,
{
    type Type<'b> = Query<T>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        serde_urlencoded::from_str(ctx.req().uri().query().unwrap_or_default())
            .map(Query)
            .map_err(Error::from_service)
    }
}

/// lazy deserialize type.
/// it lowers the deserialization to handler function where zero copy deserialize can happen.
pub struct LazyQuery<'a, T> {
    query: &'a [u8],
    _query: PhantomData<T>,
}

impl<T> LazyQuery<'_, T> {
    pub fn deserialize<'de>(&'de self) -> Result<T, Error>
    where
        T: Deserialize<'de>,
    {
        serde_urlencoded::from_bytes(self.query).map_err(Into::into)
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for LazyQuery<'a, T>
where
    T: Deserialize<'static>,
{
    type Type<'b> = LazyQuery<'b, T>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let query = ctx.req().uri().query().ok_or(ErrorStatus::bad_request())?;
        Ok(LazyQuery {
            query: query.as_bytes(),
            _query: PhantomData,
        })
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

        async fn handler(lazy: LazyQuery<'_, Id2<'_>>) -> &'static str {
            let id = lazy.deserialize().unwrap();
            assert_eq!(id.id, "dagongren");
            "kubi"
        }

        let service = handler_service(handler).call(()).now_or_panic().unwrap();

        let body = service.call(ctx).now_or_panic().unwrap().into_body();
        let res = collect_string_body(body).now_or_panic().unwrap();

        assert_eq!(res, "kubi");
    }
}
