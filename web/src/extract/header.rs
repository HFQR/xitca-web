use std::{convert::Infallible, fmt, future::Future, ops::Deref, str::FromStr};

use xitca_http::util::service::FromRequest;

use crate::{http::header::HeaderValue, request::WebRequest};

// write all header names to given macro.
macro_rules! apply_header_names {
    ($header_name: ident; $macro_name: ident) => {
        $macro_name! {
            $header_name;
            ACCEPT,
            ACCEPT_ENCODING,
            HOST,
            CONTENT_TYPE,
            CONTENT_LENGTH
        }
    };
}

// hgenerate HeaderName enum. header_name ident is not used.
macro_rules! header_name_enum {
    ($header_name: ident; $($name: ident),*) => {
        #[allow(non_camel_case_types)]
        /// HeaderName enum for Extracting according HeaderValue.
        #[derive(Debug, Eq, PartialEq)]
        pub enum HeaderName {
            $(
                $name
            ),*
        }
    }
}

apply_header_names!(_nah; header_name_enum);

// map HeaderName variant to according http::header::<HeaderName>
macro_rules! match_header_name {
    ($name: expr; $($header_name: ident),*) => {
        match $name {
             $(
                HeaderName::$header_name => crate::http::header::$header_name
            ),*
        }
    }
}

pub struct HeaderRef<'a, const HEADER_NAME: HeaderName>(&'a HeaderValue);

impl<const HEADER_NAME: HeaderName> fmt::Debug for HeaderRef<'_, HEADER_NAME> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Header")
            .field("name", &HEADER_NAME)
            .field("value", &self.0)
            .finish()
    }
}

impl<const HEADER_NAME: HeaderName> Deref for HeaderRef<'_, HEADER_NAME> {
    type Target = HeaderValue;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, 's, S: 's, const HEADER_NAME: HeaderName> FromRequest<'a, &'r mut WebRequest<'s, S>>
    for HeaderRef<'a, HEADER_NAME>
{
    type Type<'b> = HeaderRef<'b, HEADER_NAME>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move {
            let header_name = apply_header_names!(HEADER_NAME; match_header_name);
            Ok(HeaderRef(req.req().headers().get(header_name).unwrap()))
        }
    }
}

impl<const HEADER_NAME: HeaderName> HeaderRef<'_, HEADER_NAME> {
    // TODO: handle error.
    pub fn try_parse<T>(&self) -> Result<T, Infallible>
    where
        T: FromStr,
        T::Err: fmt::Debug,
    {
        Ok(self.to_str().unwrap().parse().unwrap())
    }
}
