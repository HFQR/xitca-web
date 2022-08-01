use std::{fmt, future::Future};

use futures_core::stream::Stream;

use crate::{
    handler::{
        error::{ExtractError, _ParseError},
        FromRequest,
    },
    request::WebRequest,
};

impl<'a, 'r, C, B, T, E> FromRequest<'a, WebRequest<'r, C, B>> for String
where
    B: Stream<Item = Result<T, E>> + Default,
    T: AsRef<[u8]>,
    E: fmt::Debug,
{
    type Type<'b> = String;
    type Error = ExtractError;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async move {
            let vec = Vec::from_request(req).await?;
            Ok(String::from_utf8(vec).map_err(_ParseError::String)?)
        }
    }
}
