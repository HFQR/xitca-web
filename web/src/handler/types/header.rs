use std::{convert::Infallible, fmt, future::Future, ops::Deref, str::FromStr};

use crate::{
    handler::FromRequest,
    http::header::{self, HeaderValue},
    request::WebRequest,
};

macro_rules! const_header_name {
    ($n:expr ;) => {};
    ($n:expr ; $i: ident $(, $rest:ident)*) => {
        pub const $i: usize = $n;
        const_header_name!($n + 1; $($rest),*);
    };
    ($($i:ident), +) => { const_header_name!(0; $($i),*); };
}

macro_rules! map_to_header_name {
    ($($i:ident), +) => {
        const fn map_to_header_name<const HEADER_NAME: usize>() -> header::HeaderName {
            match HEADER_NAME  {
            $(
                $i => header::$i,
            )*
                _ => unreachable!()
            }
        }
    }
}

macro_rules! const_header_name_impl {
    ($($i:ident), +) => {
        const_header_name!($($i), +);
        map_to_header_name!($($i), +);
    }
}

const_header_name_impl!(ACCEPT, ACCEPT_ENCODING, HOST, CONTENT_TYPE, CONTENT_LENGTH);

pub struct HeaderRef<'a, const HEADER_NAME: usize>(&'a HeaderValue);

impl<const HEADER_NAME: usize> fmt::Debug for HeaderRef<'_, HEADER_NAME> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Header")
            .field("name", &map_to_header_name::<HEADER_NAME>())
            .field("value", &self.0)
            .finish()
    }
}

impl<const HEADER_NAME: usize> Deref for HeaderRef<'_, HEADER_NAME> {
    type Target = HeaderValue;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, 's, S, const HEADER_NAME: usize> FromRequest<'a, &'r mut WebRequest<'s, S>>
    for HeaderRef<'a, HEADER_NAME>
{
    type Type<'b> = HeaderRef<'b, HEADER_NAME>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move {
            Ok(HeaderRef(
                req.req().headers().get(&map_to_header_name::<HEADER_NAME>()).unwrap(),
            ))
        }
    }
}

impl<const HEADER_NAME: usize> HeaderRef<'_, HEADER_NAME> {
    // TODO: handle error.
    pub fn try_parse<T>(&self) -> Result<T, Infallible>
    where
        T: FromStr,
        T::Err: fmt::Debug,
    {
        Ok(self.to_str().unwrap().parse().unwrap())
    }
}
