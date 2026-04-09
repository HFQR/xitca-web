//! http body types and traits.

pub use xitca_http::body::{
    Body, BodyExt, BoxBody, Empty, Frame, Full, RequestBody, ResponseBody, SizeHint, StreamDataBody, Trailers,
};

pub(crate) use xitca_http::body::Either;

use core::any::Any;

use crate::{bytes::Bytes, error::BodyError};

/// an extended trait for [Stream] that specify additional type info of the [Stream::Item] type.
pub trait BodyStream: Body<Data: Into<Bytes> + AsRef<[u8]> + 'static, Error: Into<BodyError>> {}

impl<B> BodyStream for B
where
    B: Body,
    B::Data: Into<Bytes> + AsRef<[u8]> + 'static,
    B::Error: Into<BodyError>,
{
}

// conditional boxing body to convert generic body type to conrete
pub(crate) fn downcast_body<B: BodyStream + 'static>(body: B) -> RequestBody {
    let body = &mut Some(body);
    match (body as &mut dyn Any).downcast_mut::<Option<RequestBody>>() {
        Some(body) => body.take().unwrap(),
        None => RequestBody::Boxed(BoxBody::new(body.take().unwrap())),
    }
}

#[cfg(feature = "nightly")]
pub use nightly::AsyncBody;

#[cfg(feature = "nightly")]
mod nightly {
    use core::{
        async_iter::AsyncIterator,
        pin::Pin,
        task::{Context, Poll},
    };

    use pin_project_lite::pin_project;

    use crate::{body::Frame, bytes::Bytes};

    use super::*;

    pin_project! {
        pub struct AsyncBody<B> {
            #[pin]
            inner: B
        }

    }

    impl<B, T, E> From<B> for AsyncBody<B>
    where
        B: AsyncIterator<Item = Result<T, E>> + 'static,
        E: Into<BodyError>,
        Bytes: From<T>,
    {
        fn from(inner: B) -> Self {
            Self { inner }
        }
    }

    impl<B, T, E> Body for AsyncBody<B>
    where
        B: AsyncIterator<Item = Result<T, E>>,
        Bytes: From<T>,
    {
        type Data = Bytes;
        type Error = E;

        #[inline]
        fn poll_frame(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            AsyncIterator::poll_next(self.project().inner, cx).map_ok(|data| Frame::Data(data.into()))
        }
    }
}
