use core::{marker::PhantomData, ops::Deref};

pub use cookie::{Cookie, Key, ParseError};

use cookie::CookieJar as _CookieJar;

use crate::{
    body::BodyStream,
    bytes::Bytes,
    error::{error_from_service, forward_blank_bad_request, Error},
    handler::{FromRequest, Responder},
    http::{
        header::ToStrError,
        header::{HeaderValue, COOKIE, SET_COOKIE},
        WebResponse,
    },
    WebContext,
};

pub struct CookieJar<K = Plain> {
    jar: _CookieJar,
    key: K,
}

impl CookieJar {
    pub fn plain() -> Self {
        Self {
            jar: _CookieJar::new(),
            key: Plain,
        }
    }

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

#[doc(hidden)]
pub struct Plain;

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for Plain
where
    B: BodyStream,
{
    type Type<'b> = Self;
    type Error = Error<C>;

    #[inline]
    async fn from_request(_: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(Self)
    }
}

macro_rules! cookie_variant {
    ($variant: tt, $method: tt, $method_mut: tt) => {
        pub struct $variant<K> {
            key: Key,
            _key: PhantomData<fn(K)>,
        }

        impl<K> Deref for $variant<K> {
            type Target = Key;

            fn deref(&self) -> &Self::Target {
                &self.key
            }
        }

        impl CookieJar {
            pub fn $method<K>(key: K) -> CookieJar<$variant<K>>
            where
                K: Into<Key>,
            {
                CookieJar {
                    jar: _CookieJar::new(),
                    key: $variant {
                        key: key.into(),
                        _key: PhantomData,
                    },
                }
            }
        }

        impl<K> CookieJar<$variant<K>> {
            #[inline]
            pub fn get(&self, name: &str) -> Option<Cookie> {
                self.jar.$method(&self.key).get(name)
            }

            #[inline]
            pub fn add<C>(&mut self, cookie: C)
            where
                C: Into<Cookie<'static>>,
            {
                self.jar.$method_mut(&self.key).add(cookie)
            }

            #[inline]
            pub fn remove<C>(&mut self, cookie: C)
            where
                C: Into<Cookie<'static>>,
            {
                self.jar.$method_mut(&self.key).remove(cookie)
            }
        }

        impl<'a, 'r, C, B, K> FromRequest<'a, WebContext<'r, C, B>> for $variant<K>
        where
            K: for<'a2, 'r2> FromRequest<'a2, WebContext<'r2, C, B>, Error = Error<C>> + Into<Key>,
            B: BodyStream,
        {
            type Type<'b> = Self;
            type Error = Error<C>;

            #[inline]
            async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
                K::from_request(ctx).await.map(|key| $variant {
                    key: key.into(),
                    _key: PhantomData,
                })
            }
        }
    };
}

cookie_variant!(Private, private, private_mut);
cookie_variant!(Signed, signed, signed_mut);

impl<'a, 'r, C, B, K> FromRequest<'a, WebContext<'r, C, B>> for CookieJar<K>
where
    K: for<'a2, 'r2> FromRequest<'a2, WebContext<'r2, C, B>, Error = Error<C>>,
    B: BodyStream,
{
    type Type<'b> = CookieJar<K>;
    type Error = Error<C>;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let key = K::from_request(ctx).await?;

        let mut jar = _CookieJar::new();

        for val in ctx.req().headers().get_all(COOKIE).into_iter() {
            for val in val.to_str()?.split(';') {
                let cookie = Cookie::parse_encoded(val.to_owned())?;
                jar.add_original(cookie);
            }
        }

        Ok(CookieJar { jar, key })
    }
}

error_from_service!(ToStrError);
forward_blank_bad_request!(ToStrError);

error_from_service!(ParseError);
forward_blank_bad_request!(ParseError);

impl<'r, C, B, K> Responder<WebContext<'r, C, B>> for CookieJar<K> {
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

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use super::*;

    #[test]
    fn cookie() {
        let _jar = CookieJar::plain();
        let _jar = CookieJar::private(Key::generate());
        let _jar = CookieJar::signed(Key::generate());

        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();
        ctx.req_mut().headers_mut().insert("cookie", "foo=bar".parse().unwrap());

        let mut jar: CookieJar = CookieJar::from_request(&ctx).now_or_panic().unwrap();

        let val = jar.get("foo").unwrap();
        assert_eq!(val.name(), "foo");
        assert_eq!(val.value(), "bar");

        jar.add(("996", "251"));

        let res = CookieJar::respond(jar, ctx).now_or_panic().unwrap();

        let header = res.headers().get(SET_COOKIE).unwrap();
        assert_eq!(header.to_str().unwrap(), "996=251");
    }

    #[derive(Clone)]
    struct MyKey(Key);

    impl From<MyKey> for Key {
        fn from(value: MyKey) -> Self {
            value.0
        }
    }

    impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for MyKey {
        type Type<'b> = MyKey;
        type Error = Error<C>;

        async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
            Ok(ctx.req().extensions().get::<MyKey>().unwrap().clone())
        }
    }

    #[test]
    fn private_cookie() {
        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();

        let key = Key::generate();

        let mut jar = _CookieJar::new();
        jar.private_mut(&key).add(Cookie::new("foo", "bar"));
        let cookie = jar.delta().next().unwrap().to_string().try_into().unwrap();

        ctx.req_mut().headers_mut().insert(COOKIE, cookie);
        ctx.req_mut().extensions_mut().insert(MyKey(key));

        let jar: CookieJar<Private<MyKey>> = CookieJar::<Private<MyKey>>::from_request(&ctx).now_or_panic().unwrap();

        let val = jar.get("foo").unwrap();
        assert_eq!(val.name(), "foo");
        assert_eq!(val.value(), "bar");
    }

    #[test]
    fn signed_cookie() {
        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();

        let key = Key::generate();

        let mut jar = _CookieJar::new();
        jar.signed_mut(&key).add(Cookie::new("foo", "bar"));

        let cookie = jar.delta().next().unwrap().to_string().try_into().unwrap();

        ctx.req_mut().headers_mut().insert(COOKIE, cookie);
        ctx.req_mut().extensions_mut().insert(MyKey(key));

        let jar: CookieJar<Signed<MyKey>> = CookieJar::<Signed<MyKey>>::from_request(&ctx).now_or_panic().unwrap();

        let val = jar.get("foo").unwrap();
        assert_eq!(val.name(), "foo");
        assert_eq!(val.value(), "bar");
    }
}
