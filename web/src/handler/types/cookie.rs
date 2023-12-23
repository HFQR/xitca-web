use core::{convert::Infallible, marker::PhantomData};

pub use cookie::{Cookie, ParseError};

use cookie::CookieJar as _CookieJar;

use crate::{
    body::BodyStream,
    error::{error_from_service, forward_blank_bad_request, Error},
    handler::FromRequest,
    http::{header::ToStrError, header::COOKIE, WebResponse},
    WebContext,
};

pub struct CookieJar<M = marker::Plain> {
    jar: _CookieJar,
    _marker: PhantomData<M>,
}

impl<M> CookieJar<M> {
    #[inline]
    pub fn get(&self, name: &str) -> Option<&Cookie<'_>> {
        self.jar.get(name)
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
