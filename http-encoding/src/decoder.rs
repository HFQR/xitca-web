//! Stream decoders.

use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::{ready, Stream};
use http::header::{HeaderMap, CONTENT_ENCODING};
use pin_project_lite::pin_project;

#[cfg(feature = "br")]
use super::brotli::BrotliDecoder;

#[cfg(feature = "gz")]
use super::gzip::GzDecoder;

#[cfg(feature = "de")]
use super::deflate::DeflateDecoder;

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
use super::writer::Writer;

pin_project! {
    pub struct Decoder<S, De, Fut>
    {
        decoder: Option<De>,
        #[pin]
        body: S,
        #[pin]
        in_flight: Option<Fut>,
    }
}

impl<S, De, T, E> Decoder<S, De, De::Future>
where
    S: Stream<Item = Result<T, E>>,
    De: AsyncDecode<T>,
    T: AsRef<[u8]> + Send + 'static,
    Bytes: From<T>,
{
    /// Construct a new decoder.
    pub fn new(body: S, decoder: De) -> Self {
        Self {
            decoder: Some(decoder),
            body,
            in_flight: None,
        }
    }

    /// Construct from headers and stream body.
    pub fn from_parts(
        headers: &HeaderMap,
        body: S,
    ) -> Decoder<S, ContentDecoder, <ContentDecoder as AsyncDecode<T>>::Future> {
        let decoder = from_headers(headers);
        Decoder::new(body, decoder)
    }
}

fn from_headers(headers: &HeaderMap) -> ContentDecoder {
    let encoding = headers.get(&CONTENT_ENCODING).and_then(|val| val.to_str().ok());

    match encoding {
        Some(encoding) => {
            let _encoding = encoding.trim();
            #[cfg(feature = "br")]
            if _encoding.eq_ignore_ascii_case("br") {
                return ContentDecoder {
                    decoder: _ContentDecoder::Br(super::brotli::BrotliDecoder::new(Writer::new())),
                };
            }

            #[cfg(feature = "gz")]
            if _encoding.eq_ignore_ascii_case("gzip") {
                return ContentDecoder {
                    decoder: _ContentDecoder::Gz(super::gzip::GzDecoder::new(Writer::new())),
                };
            }

            #[cfg(feature = "de")]
            if _encoding.eq_ignore_ascii_case("deflate") {
                return ContentDecoder {
                    decoder: _ContentDecoder::De(super::deflate::DeflateDecoder::new(Writer::new())),
                };
            }

            ContentDecoder {
                decoder: _ContentDecoder::Identity(IdentityDecoder),
            }
        }
        None => ContentDecoder {
            decoder: _ContentDecoder::Identity(IdentityDecoder),
        },
    }
}

/// An async decode trait that consume self with every decode method call.
///
/// This is useful when cross thread decode is desirable in the form of moving decode objects
/// between threads.
pub trait AsyncDecode<Item>: Sized {
    type Item;

    type Future: Future<Output = io::Result<(Self, Option<Self::Item>)>>;

    fn decode(self, item: Item) -> Self::Future;

    fn decode_eof(self) -> io::Result<Option<Self::Item>>;
}

/// Decode Error collection. Error can either from decode process as std::io::Error
/// or input Stream's error type.
pub enum DecodeError<E> {
    Io(io::Error),
    Stream(E),
}

impl<E> From<io::Error> for DecodeError<E> {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl<S, De, T, E> Stream for Decoder<S, De, De::Future>
where
    S: Stream<Item = Result<T, E>>,
    De: AsyncDecode<T>,
    De::Item: From<T>,
{
    type Item = Result<De::Item, DecodeError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            // do not attempt new chunk when in_flight process is pending.
            if let Some(fut) = this.in_flight.as_mut().as_pin_mut() {
                let (decoder, item) = ready!(fut.poll(cx))?;
                *this.decoder = Some(decoder);
                this.in_flight.set(None);

                if let Some(item) = item {
                    return Poll::Ready(Some(Ok(item)));
                }
            }

            // only poll stream when decoder still alive.
            if this.decoder.is_some() {
                match ready!(this.body.as_mut().poll_next(cx)) {
                    Some(Ok(next)) => {
                        // construct next decode in_flight future and continue.

                        let decoder = this.decoder.take().unwrap();

                        let fut = decoder.decode(next);
                        this.in_flight.set(Some(fut));
                    }
                    Some(Err(e)) => {
                        // drop decoder and return error.
                        this.decoder.take().unwrap();
                        return Poll::Ready(Some(Err(DecodeError::Stream(e))));
                    }
                    None => {
                        // stream is finished. try to decode eof and drop it.
                        return match this.decoder.take().unwrap().decode_eof()? {
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

pub struct ContentDecoder {
    decoder: _ContentDecoder,
}

enum _ContentDecoder {
    Identity(IdentityDecoder),
    #[cfg(feature = "br")]
    Br(BrotliDecoder<Writer>),
    #[cfg(feature = "gz")]
    Gz(GzDecoder<Writer>),
    #[cfg(feature = "de")]
    De(DeflateDecoder<Writer>),
}

impl From<IdentityDecoder> for ContentDecoder {
    fn from(decoder: IdentityDecoder) -> Self {
        Self {
            decoder: _ContentDecoder::Identity(decoder),
        }
    }
}

#[cfg(feature = "br")]
impl From<BrotliDecoder<Writer>> for ContentDecoder {
    fn from(decoder: BrotliDecoder<Writer>) -> Self {
        Self {
            decoder: _ContentDecoder::Br(decoder),
        }
    }
}

#[cfg(feature = "gz")]
impl From<GzDecoder<Writer>> for ContentDecoder {
    fn from(decoder: GzDecoder<Writer>) -> Self {
        Self {
            decoder: _ContentDecoder::Gz(decoder),
        }
    }
}

#[cfg(feature = "de")]
impl From<DeflateDecoder<Writer>> for ContentDecoder {
    fn from(decoder: DeflateDecoder<Writer>) -> Self {
        Self {
            decoder: _ContentDecoder::De(decoder),
        }
    }
}

impl<Item> AsyncDecode<Item> for ContentDecoder
where
    Item: AsRef<[u8]> + Send + 'static,
    Bytes: From<Item>,
{
    type Item = Bytes;
    type Future = impl Future<Output = io::Result<(Self, Option<Self::Item>)>>;

    fn decode(self, item: Item) -> Self::Future {
        async move {
            match self.decoder {
                _ContentDecoder::Identity(decoder) => <IdentityDecoder as AsyncDecode<Item>>::decode(decoder, item)
                    .await
                    .map(|(decoder, item)| (decoder.into(), item)),
                #[cfg(feature = "br")]
                _ContentDecoder::Br(decoder) => <BrotliDecoder<Writer> as AsyncDecode<Item>>::decode(decoder, item)
                    .await
                    .map(|(decoder, item)| (decoder.into(), item)),
                #[cfg(feature = "gz")]
                _ContentDecoder::Gz(decoder) => <GzDecoder<Writer> as AsyncDecode<Item>>::decode(decoder, item)
                    .await
                    .map(|(decoder, item)| (decoder.into(), item)),
                #[cfg(feature = "de")]
                _ContentDecoder::De(decoder) => <DeflateDecoder<Writer> as AsyncDecode<Item>>::decode(decoder, item)
                    .await
                    .map(|(decoder, item)| (decoder.into(), item)),
            }
        }
    }

    fn decode_eof(self) -> io::Result<Option<Self::Item>> {
        match self.decoder {
            _ContentDecoder::Identity(decoder) => <IdentityDecoder as AsyncDecode<Item>>::decode_eof(decoder),
            #[cfg(feature = "br")]
            _ContentDecoder::Br(decoder) => <BrotliDecoder<Writer> as AsyncDecode<Item>>::decode_eof(decoder),
            #[cfg(feature = "gz")]
            _ContentDecoder::Gz(decoder) => <GzDecoder<Writer> as AsyncDecode<Item>>::decode_eof(decoder),
            #[cfg(feature = "de")]
            _ContentDecoder::De(decoder) => <DeflateDecoder<Writer> as AsyncDecode<Item>>::decode_eof(decoder),
        }
    }
}

struct IdentityDecoder;

impl<Item> AsyncDecode<Item> for IdentityDecoder
where
    Bytes: From<Item>,
{
    type Item = Bytes;
    type Future = impl Future<Output = io::Result<(Self, Option<Self::Item>)>>;

    fn decode(self, item: Item) -> Self::Future {
        async move { Ok((self, Some(item.into()))) }
    }

    fn decode_eof(self) -> io::Result<Option<Self::Item>> {
        Ok(None)
    }
}
