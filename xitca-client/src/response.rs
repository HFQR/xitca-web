use std::pin::Pin;

use futures_util::StreamExt;
use tokio::time::Sleep;
use xitca_http::{bytes::BytesMut, error::BodyError, http};

use crate::{body::ResponseBody, error::Error};

const DEFAULT_PAYLOAD_LIMIT: usize = 1024 * 1024 * 8;

pub(crate) type DefaultResponse<'a> = Response<'a, DEFAULT_PAYLOAD_LIMIT>;

pub struct Response<'a, const PAYLOAD_LIMIT: usize> {
    res: http::Response<ResponseBody<'a>>,
    timer: Pin<Box<Sleep>>,
}

impl<'a, const PAYLOAD_LIMIT: usize> Response<'a, PAYLOAD_LIMIT> {
    pub(crate) fn new(res: http::Response<ResponseBody<'a>>, timer: Pin<Box<Sleep>>) -> Self {
        Self { res, timer }
    }

    /// Set payload size limit in bytes. Payload size beyond limit would be discarded.
    ///
    /// Default to 8 Mb.
    pub fn limit<const PAYLOAD_LIMIT_2: usize>(self) -> Response<'a, PAYLOAD_LIMIT_2> {
        Response {
            res: self.res,
            timer: self.timer,
        }
    }

    #[inline]
    pub async fn string(self) -> Result<String, Error> {
        self.collect().await
    }

    #[inline]
    pub async fn body(self) -> Result<Vec<u8>, Error> {
        self.collect().await
    }

    #[cfg(feature = "json")]
    pub async fn json<T>(self) -> Result<T, Error>
    where
        T: serde::de::DeserializeOwned,
    {
        use xitca_http::bytes::Buf;

        let bytes = self.collect::<BytesMut>().await?;
        Ok(serde_json::from_slice(bytes.chunk())?)
    }

    async fn collect<B>(self) -> Result<B, Error>
    where
        B: Collectable,
    {
        let (res, body) = self.res.into_parts();
        let mut timer = self.timer;

        tokio::pin!(body);

        let limit = res
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok().and_then(|str| str.parse::<usize>().ok()))
            .unwrap_or(PAYLOAD_LIMIT);

        let limit = std::cmp::min(limit, PAYLOAD_LIMIT);

        // TODO: use a meaningful capacity.
        let mut b = B::with_capacity(1024);

        loop {
            tokio::select! {
                res = body.next() => {
                    match res {
                        Some(res) => {
                            let buf = res?;
                            if buf.len() + b.len() > limit {
                                return Err(BodyError::OverFlow.into());
                            }
                            b.try_extend_from_slice(&buf)?;
                        },
                        None => break,
                    }
                },
                _ = &mut timer => todo!()
            }
        }

        Ok(b)
    }
}

trait Collectable {
    fn with_capacity(cap: usize) -> Self;

    fn try_extend_from_slice(&mut self, slice: &[u8]) -> Result<(), Error>;

    fn len(&self) -> usize;
}

impl Collectable for BytesMut {
    #[inline]
    fn with_capacity(cap: usize) -> Self {
        Self::with_capacity(cap)
    }

    #[inline]
    fn try_extend_from_slice(&mut self, slice: &[u8]) -> Result<(), Error> {
        self.extend_from_slice(slice);
        Ok(())
    }

    #[inline]
    fn len(&self) -> usize {
        Self::len(self)
    }
}

impl Collectable for Vec<u8> {
    #[inline]
    fn with_capacity(cap: usize) -> Self {
        Self::with_capacity(cap)
    }

    #[inline]
    fn try_extend_from_slice(&mut self, slice: &[u8]) -> Result<(), Error> {
        self.extend_from_slice(slice);
        Ok(())
    }

    #[inline]
    fn len(&self) -> usize {
        Self::len(self)
    }
}

impl Collectable for String {
    #[inline]
    fn with_capacity(cap: usize) -> Self {
        Self::with_capacity(cap)
    }

    fn try_extend_from_slice(&mut self, slice: &[u8]) -> Result<(), Error> {
        let str = std::str::from_utf8(slice)?;
        self.push_str(str);
        Ok(())
    }

    #[inline]
    fn len(&self) -> usize {
        Self::len(self)
    }
}
