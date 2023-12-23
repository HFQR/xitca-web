use core::{convert::Infallible, marker::PhantomData};

pub use cookie::{Cookie, ParseError};

use cookie::CookieJar as _CookieJar;

use crate::{
    body::BodyStream,
    bytes::Bytes,
    error::{error_from_service, forward_blank_bad_request, Error},
    handler::{FromRequest, Responder},
    http::{
        header::{HeaderValue, COOKIE, SET_COOKIE},
        header::{InvalidHeaderValue, ToStrError},
        WebResponse,
    },
    WebContext,
};

pub struct CookieJar<M = marker::Plain> {
    jar: _CookieJar,
    _marker: PhantomData<M>,
}

impl<M> CookieJar<M> {
    #[inline]
    pub fn get(&self, name: &str) -> Option<&Cookie> {
        self.jar.get(name)
    }

    #[inline]
    pub fn add<C>(&mut self, cookie: C)
    where
        C: Into<Cookie<'static>>,
    {
        self.jar.add(cookie)
    }

    #[inline]
    pub fn remove<C>(&mut self, cookie: C)
    where
        C: Into<Cookie<'static>>,
    {
        self.jar.remove(cookie)
    }
}

mod marker {
    pub struct Plain;
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for CookieJar<marker::Plain>
where
    B: BodyStream,
{
    type Type<'b> = CookieJar<marker::Plain>;
    type Error = Error<C>;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let mut jar = _CookieJar::new();

        for val in ctx.req().headers().get_all(COOKIE).into_iter() {
            for val in val.to_str()?.split(';') {
                let cookie = Cookie::parse_encoded(val.to_owned())?;
                jar.add_original(cookie);
            }
        }

        Ok(CookieJar {
            jar,
            _marker: PhantomData,
        })
    }
}

error_from_service!(ToStrError);
forward_blank_bad_request!(ToStrError);

error_from_service!(ParseError);
forward_blank_bad_request!(ParseError);

impl<'r, C, B> Responder<WebContext<'r, C, B>> for CookieJar<marker::Plain> {
    type Response = WebResponse;
    type Error = Error<C>;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = ctx.into_response(Bytes::new());
        <Self as Responder<WebContext<'r, C, B>>>::map(self, res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        let headers = res.headers_mut();
        for cookie in self.jar.delta() {
            let value = HeaderValue::try_from(cookie.encoded().to_string())?;
            headers.append(SET_COOKIE, value);
        }
        Ok(res)
    }
}

error_from_service!(InvalidHeaderValue);
forward_blank_bad_request!(InvalidHeaderValue);

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use super::*;

    #[test]
    fn cookie() {
        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();
        ctx.req_mut().headers_mut().insert("cookie", "foo=bar".parse().unwrap());

        let mut jar = CookieJar::from_request(&ctx).now_or_panic().unwrap();

        let val = jar.get("foo").unwrap();
        assert_eq!(val.name(), "foo");
        assert_eq!(val.value(), "bar");

        jar.add(("996", "251"));

        let res = CookieJar::respond(jar, ctx).now_or_panic().unwrap();

        let header = res.headers().get(SET_COOKIE).unwrap();
        assert_eq!(header.to_str().unwrap(), "996=251");
    }
}
