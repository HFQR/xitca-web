use std::{
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// A coder type that can be used for either encode or decode which determined by De type.
    pub struct Coder<S, C>{
        #[pin]
        body: S,
        coder: C,
    }
}

impl<S, C, T, E> Coder<S, C>
where
    S: Stream<Item = Result<T, E>>,
    C: Code<T>,
    T: AsRef<[u8]> + Send + 'static,
    Bytes: From<T>,
{
    /// Construct a new coder.
    pub fn new(body: S, coder: C) -> Self {
        Self { body, coder }
    }
}

/// Coder Error collection. Error can either from coding process as std::io::Error
/// or input Stream's error type.
pub enum CoderError<E> {
    Io(io::Error),
    Feature(Feature),
    Stream(E),
}

#[doc(hidden)]
pub enum Feature {
    Br,
    Gzip,
    Deflate,
}

impl<E> fmt::Debug for CoderError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{:?}", e),
            Self::Feature(Feature::Br) => write!(f, "br feature is disabled."),
            Self::Feature(Feature::Gzip) => write!(f, "gz feature is disabled."),
            Self::Feature(Feature::Deflate) => write!(f, "de feature is disabled."),
            Self::Stream(..) => write!(f, "Input Stream body error."),
        }
    }
}

impl<E> fmt::Display for CoderError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{}", e),
            Self::Feature(Feature::Br) => write!(f, "br feature is disabled."),
            Self::Feature(Feature::Gzip) => write!(f, "gz feature is disabled."),
            Self::Feature(Feature::Deflate) => write!(f, "de feature is disabled."),
            Self::Stream(..) => write!(f, "Input Stream body error."),
        }
    }
}

impl<E> std::error::Error for CoderError<E> {}

impl<E> From<io::Error> for CoderError<E> {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl<S, C, T, E> Stream for Coder<S, C>
where
    S: Stream<Item = Result<T, E>>,
    C: Code<T>,
    C::Item: From<T>,
{
    type Item = Result<C::Item, CoderError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        while let Some(res) = ready!(this.body.as_mut().poll_next(cx)) {
            match res {
                Ok(item) => {
                    if let Some(item) = this.coder.code(item)? {
                        return Poll::Ready(Some(Ok(item)));
                    }
                }
                Err(e) => return Poll::Ready(Some(Err(CoderError::Stream(e)))),
            }
        }

        match this.coder.code_eof()? {
            Some(res) => Poll::Ready(Some(Ok(res))),
            None => Poll::Ready(None),
        }
    }
}

pub trait Code<Item>: Sized {
    type Item;

    fn code(&mut self, item: Item) -> io::Result<Option<Self::Item>>;

    fn code_eof(&mut self) -> io::Result<Option<Self::Item>>;
}

/// Identity coder serve as a pass through coder that just forward items.
pub struct IdentityCoder;

impl<Item> Code<Item> for IdentityCoder
where
    Bytes: From<Item>,
{
    type Item = Bytes;

    fn code(&mut self, item: Item) -> io::Result<Option<Self::Item>> {
        Ok(Some(item.into()))
    }

    fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
        Ok(None)
    }
}

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
macro_rules! code_impl {
    ($coder: ident) => {
        impl<Item> crate::Code<Item> for $coder<crate::writer::Writer>
        where
            Item: AsRef<[u8]> + Send + 'static,
        {
            type Item = bytes::Bytes;

            fn code(&mut self, item: Item) -> ::std::io::Result<Option<bytes::Bytes>> {
                use ::std::io::Write;

                self.write_all(item.as_ref())?;
                self.flush()?;
                let b = self.get_mut().take();
                if !b.is_empty() {
                    Ok(Some(b))
                } else {
                    Ok(None)
                }
            }

            fn code_eof(&mut self) -> ::std::io::Result<Option<Self::Item>> {
                use ::std::io::Write;

                self.flush()?;
                let b = self.get_mut().take();
                if !b.is_empty() {
                    Ok(Some(b))
                } else {
                    Ok(None)
                }
            }
        }
    };
}
