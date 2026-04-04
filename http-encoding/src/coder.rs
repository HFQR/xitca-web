use core::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use std::io;

use bytes::Bytes;
use http::HeaderMap;
use http_body_alt::{Body, Frame, SizeHint};
use pin_project_lite::pin_project;

use super::error::CoderError;

pin_project! {
    /// A coder type that can be used for either encode or decode which determined by De type.
    #[derive(Default)]
    pub struct Coder<S, C = FeaturedCode>{
        #[pin]
        body: S,
        coder: C,
        trailers: Option<HeaderMap>
    }
}

impl<S, C> Coder<S, C>
where
    S: Body,
    C: Code<S::Data>,
    S::Data: AsRef<[u8]>,
{
    /// Construct a new coder.
    #[inline]
    pub const fn new(body: S, coder: C) -> Self {
        Self {
            body,
            coder,
            trailers: None,
        }
    }

    #[inline]
    pub fn into_inner(self) -> S {
        self.body
    }
}

impl<S, C> Body for Coder<S, C>
where
    S: Body,
    CoderError: From<S::Error>,
    C: Code<S::Data>,
{
    type Data = C::Item;
    type Error = CoderError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let mut this = self.project();

        while let Some(res) = ready!(this.body.as_mut().poll_frame(cx)) {
            match res? {
                Frame::Data(item) => {
                    if let Some(item) = this.coder.code(item)? {
                        return Poll::Ready(Some(Ok(Frame::Data(item))));
                    }
                }
                Frame::Trailers(trailers) => {
                    this.trailers.replace(trailers);
                    break;
                }
            }
        }

        match this.coder.code_eof()? {
            Some(res) => Poll::Ready(Some(Ok(Frame::Data(res)))),
            None => Poll::Ready(this.trailers.take().map(|trailsers| Ok(Frame::Trailers(trailsers)))),
        }
    }

    fn is_end_stream(&self) -> bool {
        // forward is_end_stream to coder as it determines the data length after (de)compress.
        self.coder.is_end_stream(&self.body)
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        // forward size_hint to coder as it determines the data length after (de)compress.
        self.coder.size_hint(&self.body)
    }
}

pub trait Code<T>: Sized {
    type Item;

    fn code(&mut self, item: T) -> io::Result<Option<Self::Item>>;

    fn code_eof(&mut self) -> io::Result<Option<Self::Item>>;

    /// A helper method for overriding associated input body's end_stream state.
    /// by default it returns value the same as [`Body::is_end_stream`]'s default value.
    /// in other word the default prediction is (de)compress can not hint if body has ended.
    #[allow(unused_variables)]
    #[inline]
    fn is_end_stream(&self, body: &impl Body) -> bool {
        false
    }

    /// A helper method for overriding associated input body's size_hint.
    /// by default it returns value the same as [`SizeHint`]'s default value.
    /// in other word the default prediction is (de)compress can not hint an exact size.
    #[allow(unused_variables)]
    #[inline]
    fn size_hint(&self, body: &impl Body) -> SizeHint {
        SizeHint::default()
    }
}

/// coder serve as pass through that just forward items.
pub struct NoOpCode;

impl<T> Code<T> for NoOpCode
where
    T: AsRef<[u8]> + 'static,
{
    type Item = Bytes;

    fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
        Ok(Some(
            try_downcast_to_bytes(item).unwrap_or_else(|item| Bytes::copy_from_slice(item.as_ref())),
        ))
    }

    #[inline]
    fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
        Ok(None)
    }

    // similar reason to size_hint.
    #[inline]
    fn is_end_stream(&self, body: &impl Body) -> bool {
        body.is_end_stream()
    }

    // noop coder can take advantage of not doing any de/encoding work and hint the output body
    // size. this would help downstream to infer the size of body and avoid going through
    // transfer-encoding: chunked when possible.
    #[inline]
    fn size_hint(&self, body: &impl Body) -> SizeHint {
        body.size_hint()
    }
}

pub enum FeaturedCode {
    NoOp(NoOpCode),
    #[cfg(feature = "br")]
    DecodeBr(super::brotli::Decoder),
    #[cfg(feature = "br")]
    EncodeBr(super::brotli::Encoder),
    #[cfg(feature = "gz")]
    DecodeGz(super::gzip::Decoder),
    #[cfg(feature = "gz")]
    EncodeGz(super::gzip::Encoder),
    #[cfg(feature = "de")]
    DecodeDe(super::deflate::Decoder),
    #[cfg(feature = "de")]
    EncodeDe(super::deflate::Encoder),
}

impl Default for FeaturedCode {
    fn default() -> Self {
        Self::NoOp(NoOpCode)
    }
}

impl<T> Code<T> for FeaturedCode
where
    T: AsRef<[u8]> + 'static,
{
    type Item = Bytes;

    fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
        match self {
            Self::NoOp(coder) => coder.code(item),
            #[cfg(feature = "br")]
            Self::DecodeBr(coder) => coder.code(item),
            #[cfg(feature = "br")]
            Self::EncodeBr(coder) => coder.code(item),
            #[cfg(feature = "gz")]
            Self::DecodeGz(coder) => coder.code(item),
            #[cfg(feature = "gz")]
            Self::EncodeGz(coder) => coder.code(item),
            #[cfg(feature = "de")]
            Self::DecodeDe(coder) => coder.code(item),
            #[cfg(feature = "de")]
            Self::EncodeDe(coder) => coder.code(item),
        }
    }

    fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
        match self {
            Self::NoOp(coder) => <NoOpCode as Code<T>>::code_eof(coder),
            #[cfg(feature = "br")]
            Self::DecodeBr(coder) => <super::brotli::Decoder as Code<T>>::code_eof(coder),
            #[cfg(feature = "br")]
            Self::EncodeBr(coder) => <super::brotli::Encoder as Code<T>>::code_eof(coder),
            #[cfg(feature = "gz")]
            Self::DecodeGz(coder) => <super::gzip::Decoder as Code<T>>::code_eof(coder),
            #[cfg(feature = "gz")]
            Self::EncodeGz(coder) => <super::gzip::Encoder as Code<T>>::code_eof(coder),
            #[cfg(feature = "de")]
            Self::DecodeDe(coder) => <super::deflate::Decoder as Code<T>>::code_eof(coder),
            #[cfg(feature = "de")]
            Self::EncodeDe(coder) => <super::deflate::Encoder as Code<T>>::code_eof(coder),
        }
    }

    fn is_end_stream(&self, body: &impl Body) -> bool {
        match self {
            Self::NoOp(coder) => <NoOpCode as Code<T>>::is_end_stream(coder, body),
            #[cfg(feature = "br")]
            Self::DecodeBr(coder) => <super::brotli::Decoder as Code<T>>::is_end_stream(coder, body),
            #[cfg(feature = "br")]
            Self::EncodeBr(coder) => <super::brotli::Encoder as Code<T>>::is_end_stream(coder, body),
            #[cfg(feature = "gz")]
            Self::DecodeGz(coder) => <super::gzip::Decoder as Code<T>>::is_end_stream(coder, body),
            #[cfg(feature = "gz")]
            Self::EncodeGz(coder) => <super::gzip::Encoder as Code<T>>::is_end_stream(coder, body),
            #[cfg(feature = "de")]
            Self::DecodeDe(coder) => <super::deflate::Decoder as Code<T>>::is_end_stream(coder, body),
            #[cfg(feature = "de")]
            Self::EncodeDe(coder) => <super::deflate::Encoder as Code<T>>::is_end_stream(coder, body),
        }
    }

    fn size_hint(&self, body: &impl Body) -> SizeHint {
        match self {
            Self::NoOp(coder) => <NoOpCode as Code<T>>::size_hint(coder, body),
            #[cfg(feature = "br")]
            Self::DecodeBr(coder) => <super::brotli::Decoder as Code<T>>::size_hint(coder, body),
            #[cfg(feature = "br")]
            Self::EncodeBr(coder) => <super::brotli::Encoder as Code<T>>::size_hint(coder, body),
            #[cfg(feature = "gz")]
            Self::DecodeGz(coder) => <super::gzip::Decoder as Code<T>>::size_hint(coder, body),
            #[cfg(feature = "gz")]
            Self::EncodeGz(coder) => <super::gzip::Encoder as Code<T>>::size_hint(coder, body),
            #[cfg(feature = "de")]
            Self::DecodeDe(coder) => <super::deflate::Decoder as Code<T>>::size_hint(coder, body),
            #[cfg(feature = "de")]
            Self::EncodeDe(coder) => <super::deflate::Encoder as Code<T>>::size_hint(coder, body),
        }
    }
}

#[cfg(any(feature = "gz", feature = "de"))]
macro_rules! code_impl {
    ($coder: ident) => {
        impl<T> crate::Code<T> for $coder<crate::writer::BytesMutWriter>
        where
            T: AsRef<[u8]>,
        {
            type Item = ::bytes::Bytes;

            fn code(&mut self, item: T) -> ::std::io::Result<Option<Self::Item>> {
                use ::std::io::Write;

                self.write_all(item.as_ref())?;
                let b = self.get_mut().take();
                if !b.is_empty() { Ok(Some(b)) } else { Ok(None) }
            }

            fn code_eof(&mut self) -> ::std::io::Result<Option<Self::Item>> {
                self.try_finish()?;
                let b = self.get_mut().take();
                if !b.is_empty() { Ok(Some(b)) } else { Ok(None) }
            }
        }
    };
}

fn try_downcast_to_bytes<T: 'static>(item: T) -> Result<Bytes, T> {
    use core::any::Any;

    let item = &mut Some(item);
    match (item as &mut dyn Any).downcast_mut::<Option<Bytes>>() {
        Some(bytes) => Ok(bytes.take().unwrap()),
        None => Err(item.take().unwrap()),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn downcast_bytes() {
        let bytes = Bytes::new();
        assert!(try_downcast_to_bytes(bytes).is_ok());
        let bytes = Vec::<u8>::new();
        assert!(try_downcast_to_bytes(bytes).is_err());
    }
}
