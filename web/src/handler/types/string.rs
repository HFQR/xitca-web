use std::future::Future;

use crate::{
    handler::{
        error::{ExtractError, _ParseError},
        FromRequest,
    },
    request::WebRequest,
    stream::WebStream,
};

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for String
where
    B: WebStream + Default,
{
    type Type<'b> = String;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async move {
            let vec = Vec::from_request(req).await?;
            Ok(String::from_utf8(vec).map_err(|e| _ParseError::String(e.utf8_error()))?)
        }
    }
}
