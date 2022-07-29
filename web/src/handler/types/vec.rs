use std::{
    convert::Infallible,
    future::{poll_fn, Future},
};

use futures_core::stream::Stream;
use xitca_unsafe_collection::pin;

use crate::{handler::FromRequest, request::WebRequest};

pub struct Vec(pub std::vec::Vec<u8>);

impl<'a, 'r, C, B, T, E> FromRequest<'a, WebRequest<'r, C, B>> for Vec
where
    B: Stream<Item = Result<T, E>> + Default,
    T: AsRef<[u8]>,
{
    type Type<'b> = Vec;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        let body = req.take_body_ref();

        async {
            pin!(body);

            let mut vec = std::vec::Vec::new();

            while let Some(Ok(chunk)) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                vec.extend_from_slice(chunk.as_ref());
            }

            Ok(Vec(vec))
        }
    }
}
