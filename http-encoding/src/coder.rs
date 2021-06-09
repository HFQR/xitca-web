use std::{
    fmt,
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// A coder type that can be used for either encode or decode which determined by De type.
    pub struct Coder<S, De, Fut> {
        coder: Option<De>,
        #[pin]
        body: S,
        #[pin]
        in_flight: Option<Fut>,
    }
}

impl<S, De, T, E> Coder<S, De, De::Future>
where
    S: Stream<Item = Result<T, E>>,
    De: AsyncCode<T>,
    T: AsRef<[u8]> + Send + 'static,
    Bytes: From<T>,
{
    /// Construct a new coder.
    pub fn new(body: S, coder: De) -> Self {
        Self {
            coder: Some(coder),
            body,
            in_flight: None,
        }
    }
}

/// Coder Error collection. Error can either from coding process as std::io::Error
/// or input Stream's error type.
pub enum CoderError<E> {
    Io(io::Error),
    Runtime(tokio::task::JoinError),
    Feature(Feature),
    Stream(E),
}

pub enum Feature {
    Br,
    Gzip,
    Deflate,
}

impl<E> fmt::Debug for CoderError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{:?}", e),
            Self::Runtime(ref e) => write!(f, "{:?}", e),
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
            Self::Io(ref e) => write!(f, "{:#?}", e),
            Self::Runtime(ref e) => write!(f, "{:#?}", e),
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

impl<S, De, T, E> Stream for Coder<S, De, De::Future>
where
    S: Stream<Item = Result<T, E>>,
    De: AsyncCode<T>,
    De::Item: From<T>,
{
    type Item = Result<De::Item, CoderError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            // do not attempt new chunk when in_flight process is pending.
            if let Some(fut) = this.in_flight.as_mut().as_pin_mut() {
                let (coder, item) = ready!(fut.poll(cx))?;
                *this.coder = Some(coder);
                this.in_flight.set(None);

                if let Some(item) = item {
                    return Poll::Ready(Some(Ok(item)));
                }
            }

            // only poll stream when coder still alive.
            if this.coder.is_some() {
                match ready!(this.body.as_mut().poll_next(cx)) {
                    Some(Ok(next)) => {
                        // construct next code in_flight future and continue.

                        let coder = this.coder.take().unwrap();

                        let fut = coder.code(next);
                        this.in_flight.set(Some(fut));
                    }
                    Some(Err(e)) => {
                        // drop coder and return error.
                        this.coder.take().unwrap();
                        return Poll::Ready(Some(Err(CoderError::Stream(e))));
                    }
                    None => {
                        // stream is finished. try to code eof and drop it.
                        return match this.coder.take().unwrap().code_eof()? {
                            Some(res) => Poll::Ready(Some(Ok(res))),
                            None => Poll::Ready(None),
                        };
                    }
                }
            } else {
                return Poll::Ready(None);
            }
        }
    }
}

/// An async coding trait that consume self with every method call that can be used for either
/// decode or encode.
///
/// This is useful when cross thread de/encode is desirable in the form of moving objects between
/// threads.
pub trait AsyncCode<Item>: Sized {
    type Item;

    type Future: Future<Output = io::Result<(Self, Option<Self::Item>)>>;

    fn code(self, item: Item) -> Self::Future;

    fn code_eof(self) -> io::Result<Option<Self::Item>>;
}

/// Identity coder serve as a pass through coder that just forward items.
pub struct IdentityCoder;

impl<Item> AsyncCode<Item> for IdentityCoder
where
    Bytes: From<Item>,
{
    type Item = Bytes;
    type Future = impl Future<Output = io::Result<(Self, Option<Self::Item>)>>;

    fn code(self, item: Item) -> Self::Future {
        async move { Ok((self, Some(item.into()))) }
    }

    fn code_eof(self) -> io::Result<Option<Self::Item>> {
        Ok(None)
    }
}

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
macro_rules! async_code_impl {
    ($coder: ident, $in_place_size: path) => {
        impl<Item> crate::AsyncCode<Item> for $coder<crate::writer::Writer>
        where
            Item: AsRef<[u8]> + Send + 'static,
        {
            type Item = bytes::Bytes;
            type Future = impl std::future::Future<Output = std::io::Result<(Self, Option<Self::Item>)>>;

            fn code(self, item: Item) -> Self::Future {
                use std::io::Write;

                fn code(
                    mut this: $coder<crate::writer::Writer>,
                    buf: &[u8],
                ) -> std::io::Result<($coder<crate::writer::Writer>, Option<bytes::Bytes>)> {
                    this.write_all(buf)?;
                    this.flush()?;
                    let b = this.get_mut().take();
                    if !b.is_empty() {
                        Ok((this, Some(b)))
                    } else {
                        Ok((this, None))
                    }
                }

                async move {
                    let buf = item.as_ref();
                    if buf.len() < $in_place_size {
                        code(self, buf)
                    } else {
                        tokio::task::spawn_blocking(move || code(self, item.as_ref())).await?
                    }
                }
            }

            #[allow(unused_mut)]
            fn code_eof(mut self) -> std::io::Result<Option<Self::Item>> {
                let b = self.finish()?.take();

                if !b.is_empty() {
                    Ok(Some(b))
                } else {
                    Ok(None)
                }
            }
        }
    };
}
