//! type extractor and responder for cookies.

use core::{borrow::Borrow, marker::PhantomData, ops::Deref};

pub use cookie::{Cookie, Key, ParseError};

use cookie::CookieJar as _CookieJar;

use crate::{
    WebContext,
    body::ResponseBody,
    error::{Error, ErrorStatus, ExtensionNotFound, HeaderNotFound, error_from_service, forward_blank_bad_request},
    handler::{FromRequest, Responder},
    http::{
        WebResponse,
        header::ToStrError,
        header::{COOKIE, HeaderValue, SET_COOKIE},
    },
};

macro_rules! key_impl {
    ($key: tt) => {
        impl $key {
            /// generates signing/encryption keys from a secure, random source.
            /// see [Key] for further detail.
            #[inline]
            pub fn generate() -> Self {
                Self(Key::generate())
            }
        }

        impl From<$key> for Key {
            fn from(key: $key) -> Self {
                key.0
            }
        }

        impl From<Key> for $key {
            fn from(key: Key) -> Self {
                Self(key)
            }
        }
    };
}

/// an extractor type wrapping around [Key] hinting itself can be extracted from
/// application state. See [App::with_state] for compile time state management.
///
/// [App::with_state]: crate::App::with_state
#[derive(Clone, Debug)]
pub struct StateKey(Key);

key_impl!(StateKey);

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for StateKey
where
    C: Borrow<Self>,
{
    type Type<'b> = Self;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.state().borrow().clone())
    }
}

/// an extractor type wrapping around [Key] hinting itself can be extracted from
/// request extensions. See [WebRequest::extensions] for run time state management.
///
/// [WebRequest::extensions]: crate::http::WebRequest::extensions
#[derive(Clone, Debug)]
pub struct ExtensionKey(Key);

key_impl!(ExtensionKey);

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for ExtensionKey {
    type Type<'b> = Self;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        ctx.req()
            .extensions()
            .get::<Self>()
            .cloned()
            .ok_or_else(|| Error::from(ExtensionNotFound::from_type::<Self>()))
    }
}

/// container of cookies extracted from request.
pub struct CookieJar<K = Plain> {
    jar: _CookieJar,
    key: K,
}

impl CookieJar {
    /// construct a new cookie container with no encryption.
    pub fn plain() -> Self {
        Self {
            jar: _CookieJar::new(),
            key: Plain,
        }
    }

    /// get cookie with given key name
    #[inline]
    pub fn get(&self, name: &str) -> Option<&Cookie> {
        self.jar.get(name)
    }

    /// add cookie to container.
    #[inline]
    pub fn add<C>(&mut self, cookie: C)
    where
        C: Into<Cookie<'static>>,
    {
        self.jar.add(cookie)
    }

    /// remove cookie from container.
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

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for Plain {
    type Type<'b> = Self;
    type Error = Error;

    #[inline]
    async fn from_request(_: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(Self)
    }
}

macro_rules! cookie_variant {
    ($variant: tt, $method: tt, $method_mut: tt) => {
        /// encrypted cookie container type.
        /// must annotate the generic type param with types that can provide the key
        /// for encryption. See [StateKey] and [ExtensionKey] for detail.
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
            K: for<'a2, 'r2> FromRequest<'a2, WebContext<'r2, C, B>, Error = Error> + Into<Key>,
        {
            type Type<'b> = Self;
            type Error = Error;

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
    K: for<'a2, 'r2> FromRequest<'a2, WebContext<'r2, C, B>, Error = Error>,
{
    type Type<'b> = CookieJar<K>;
    type Error = Error;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let key = K::from_request(ctx).await?;

        let mut jar = _CookieJar::new();

        let headers = ctx.req().headers();

        if !headers.contains_key(COOKIE) {
            return Err(Error::from(HeaderNotFound(COOKIE)));
        }

        for val in headers.get_all(COOKIE) {
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
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = ctx.into_response(ResponseBody::empty());
        Responder::<WebContext<'r, C, B>>::map(self, res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        let headers = res.headers_mut();
        for cookie in self.jar.delta() {
            let value = HeaderValue::try_from(cookie.encoded().to_string()).map_err(|_| ErrorStatus::internal())?;
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
        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();

        let mut jar = CookieJar::plain();
        jar.add(("foo", "bar"));

        let cookie = jar
            .respond(ctx.reborrow())
            .now_or_panic()
            .unwrap()
            .headers_mut()
            .remove(SET_COOKIE)
            .unwrap();

        ctx.req_mut().headers_mut().insert(COOKIE, cookie);

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
        type Error = Error;

        async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
            Ok(ctx.req().extensions().get::<MyKey>().unwrap().clone())
        }
    }

    #[test]
    fn private_cookie() {
        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();

        let key = Key::generate();

        let mut jar = CookieJar::private(key.clone());
        jar.add(("foo", "bar"));

        let cookie = jar
            .respond(ctx.reborrow())
            .now_or_panic()
            .unwrap()
            .headers_mut()
            .remove(SET_COOKIE)
            .unwrap();

        ctx.req_mut().headers_mut().insert(COOKIE, cookie);
        ctx.req_mut().extensions_mut().insert(MyKey(key));

        let jar = CookieJar::<Private<MyKey>>::from_request(&ctx).now_or_panic().unwrap();

        let val = jar.get("foo").unwrap();
        assert_eq!(val.name(), "foo");
        assert_eq!(val.value(), "bar");
    }

    #[test]
    fn signed_cookie() {
        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();

        let key = Key::generate();

        let mut jar = CookieJar::signed(key.clone());
        jar.add(("foo", "bar"));

        let cookie = jar
            .respond(ctx.reborrow())
            .now_or_panic()
            .unwrap()
            .headers_mut()
            .remove(SET_COOKIE)
            .unwrap();

        ctx.req_mut().headers_mut().insert(COOKIE, cookie);
        ctx.req_mut().extensions_mut().insert(MyKey(key));

        let jar = CookieJar::<Signed<MyKey>>::from_request(&ctx).now_or_panic().unwrap();

        let val = jar.get("foo").unwrap();
        assert_eq!(val.name(), "foo");
        assert_eq!(val.value(), "bar");
    }
}
