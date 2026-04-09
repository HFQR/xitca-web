//! type extractor for request body stream.

use core::{cmp, convert::Infallible, pin::pin};

use crate::{
    body::{BodyExt, BodyStream, BoxBody, ResponseBody, SizeHint},
    bytes::{Bytes, BytesMut},
    context::WebContext,
    error::{BodyOverFlow, Error},
    handler::{FromRequest, Responder},
    http::{IntoResponse, WebResponse},
};

pub struct Body<B>(pub B);

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for Body<B>
where
    B: BodyStream + Default,
{
    type Type<'b> = Body<B>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(Body(ctx.take_body_ref()))
    }
}

/// helper type for limiting body size.
/// when LIMIT > 0 body size is limited to LIMIT in bytes.
/// when LIMIT == 0 body size is unlimited.
pub struct Limit<const LIMIT: usize>;

macro_rules! from_bytes_impl {
    ($type: ty, $original: tt) => {
        impl<'a, 'r, C, B, const LIMIT: usize> FromRequest<'a, WebContext<'r, C, B>> for ($type, Limit<LIMIT>)
        where
            B: BodyStream + Default,
        {
            type Type<'b> = ($type, Limit<LIMIT>);
            type Error = Error;

            async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
                let body = ctx.take_body_ref();

                let limit = match body.size_hint() {
                    SizeHint::Exact(size) if LIMIT > 0 => cmp::min(size as usize, LIMIT),
                    SizeHint::Exact(size) => size as usize,
                    _ => LIMIT,
                };

                let mut body = pin!(body);

                let mut buf = <$type>::with_capacity(limit);

                while let Some(data) = body.as_mut().data().await {
                    let data = data.map_err(Into::into)?;
                    buf.extend_from_slice(data.as_ref());
                    if limit > 0 && buf.len() > limit {
                        return Err(Error::from(BodyOverFlow { limit }));
                    }
                }

                Ok((buf, Limit))
            }
        }

        from_bytes_impl!($type);
    };
    ($type: ty) => {
        impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for $type
        where
            B: BodyStream + Default,
        {
            type Type<'b> = $type;
            type Error = Error;

            #[inline]
            async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
                <($type, Limit<0>)>::from_request(ctx)
                    .await
                    .map(|(bytes, _)| bytes)
            }
        }
    };
}

from_bytes_impl!(BytesMut, _);
from_bytes_impl!(Vec<u8>, _);

impl<'a, 'r, C, B, const LIMIT: usize> FromRequest<'a, WebContext<'r, C, B>> for (Bytes, Limit<LIMIT>)
where
    B: BodyStream + Default,
{
    type Type<'b> = (Bytes, Limit<LIMIT>);
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        <(BytesMut, Limit<LIMIT>)>::from_request(ctx)
            .await
            .map(|(bytes, limit)| (bytes.into(), limit))
    }
}

from_bytes_impl!(Bytes);

macro_rules! responder_impl {
    ($type: ty) => {
        impl<'r, C, B> Responder<WebContext<'r, C, B>> for $type {
            type Response = WebResponse;
            type Error = Infallible;

            #[inline]
            async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                Ok(ctx.into_response(self))
            }

            #[inline]
            fn map(self, res: Self::Response) -> Result<Self::Response, Self::Error> {
                Ok(res.map(|_| self.into()))
            }
        }
    };
}

responder_impl!(Bytes);
responder_impl!(BytesMut);
responder_impl!(Vec<u8>);

impl<'r, C, B, ResB> Responder<WebContext<'r, C, B>> for ResponseBody<ResB> {
    type Response = WebResponse<ResponseBody<ResB>>;
    type Error = Error;

    #[inline]
    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Ok(ctx.req.as_response(self))
    }

    #[inline]
    fn map(self, res: Self::Response) -> Result<Self::Response, Self::Error> {
        Ok(res.map(|_| self))
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for BoxBody {
    type Response = WebResponse;
    type Error = Error;

    #[inline]
    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        ResponseBody::body(self).respond(ctx).await
    }

    #[inline]
    fn map(self, res: Self::Response) -> Result<Self::Response, Self::Error> {
        Responder::<WebContext<'r, C, B>>::map(ResponseBody::body(self), res)
    }
}
