use std::{convert::Infallible, future::Future};

use futures_core::stream::Stream;

use crate::{handler::FromRequest, request::WebRequest};

use super::vec::Vec;

pub struct String(pub std::string::String);

impl<'a, 'r, C, B, T, E> FromRequest<'a, WebRequest<'r, C, B>> for String
where
    B: Stream<Item = Result<T, E>> + Default,
    T: AsRef<[u8]>,
{
    type Type<'b> = String;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async move {
            let Vec(vec) = Vec::from_request(req).await?;
            Ok(String(std::string::String::from_utf8(vec).unwrap()))
        }
    }
}
