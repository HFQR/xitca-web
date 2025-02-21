//! response generator for Html

use core::{convert::Infallible, fmt};

use xitca_http::util::service::router::{PathGen, RouteGen, RouterMapErr};

use crate::{
    body::ResponseBody,
    context::WebContext,
    error::Error,
    handler::Responder,
    http::{WebResponse, const_header_value::TEXT_HTML_UTF8, header::CONTENT_TYPE},
    service::Service,
};

#[derive(Clone)]
pub struct Html<T>(pub T);

impl<T> fmt::Debug for Html<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Html").field("value", &self.0).finish()
    }
}

impl<'r, C, B, T> Responder<WebContext<'r, C, B>> for Html<T>
where
    T: Into<ResponseBody>,
{
    type Response = WebResponse;
    type Error = Error;

    #[inline]
    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(self.0);
        res.headers_mut().insert(CONTENT_TYPE, TEXT_HTML_UTF8);
        Ok(res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        res.headers_mut().insert(CONTENT_TYPE, TEXT_HTML_UTF8);
        Ok(res.map(|_| self.0.into()))
    }
}

impl<T> PathGen for Html<T> {}

impl<T> RouteGen for Html<T> {
    type Route<R> = RouterMapErr<R>;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        RouterMapErr(route)
    }
}

impl<T> Service for Html<T>
where
    T: Clone,
{
    type Response = Self;
    type Error = Infallible;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        Ok(self.clone())
    }
}

impl<'r, C, B, T> Service<WebContext<'r, C, B>> for Html<T>
where
    T: Clone + Into<ResponseBody>,
{
    type Response = WebResponse;
    type Error = Error;

    #[inline]
    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.clone().respond(ctx).await
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{App, http::WebRequest};

    use super::*;

    #[test]
    fn service() {
        let res = App::new()
            .at("/", Html("hello,world!"))
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(WebRequest::default())
            .now_or_panic()
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }
}
