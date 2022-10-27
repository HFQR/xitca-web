use std::{
    fmt,
    ops::{Deref, DerefMut},
    pin::Pin,
    time::Duration,
};

use futures_util::StreamExt;
use tokio::time::{Instant, Sleep};
use tracing::debug;
use xitca_http::{bytes::BytesMut, http};

use crate::{
    body::ResponseBody,
    error::{Error, TimeoutError},
    timeout::Timeout,
};

const DEFAULT_PAYLOAD_LIMIT: usize = 1024 * 1024 * 8;

pub struct Response<'a, const PAYLOAD_LIMIT: usize = DEFAULT_PAYLOAD_LIMIT> {
    res: http::Response<ResponseBody<'a>>,
    timer: Pin<Box<Sleep>>,
    timeout: Duration,
}

impl<'a, const PAYLOAD_LIMIT: usize> Deref for Response<'a, PAYLOAD_LIMIT> {
    type Target = http::Response<ResponseBody<'a>>;

    fn deref(&self) -> &Self::Target {
        &self.res
    }
}

impl<const PAYLOAD_LIMIT: usize> DerefMut for Response<'_, PAYLOAD_LIMIT> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.res
    }
}

impl<const PAYLOAD_LIMIT: usize> fmt::Debug for Response<'_, PAYLOAD_LIMIT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.res)
    }
}

impl<'a, const PAYLOAD_LIMIT: usize> Response<'a, PAYLOAD_LIMIT> {
    #[cfg(any(feature = "http1", feature = "http2", feature = "http3"))]
    pub(crate) fn new(res: http::Response<ResponseBody<'a>>, timer: Pin<Box<Sleep>>, timeout: Duration) -> Self {
        Self { res, timer, timeout }
    }

    /// Get a reference of the inner response type.
    pub fn inner(&self) -> &http::Response<ResponseBody<'a>> {
        &self.res
    }

    /// Get a mutable reference of the inner response type.
    pub fn inner_mut(&mut self) -> &mut http::Response<ResponseBody<'a>> {
        &mut self.res
    }

    /// Set payload size limit in bytes. Payload size beyond limit would be discarded.
    ///
    /// Default to 8 Mb.
    #[inline]
    pub fn limit<const PAYLOAD_LIMIT_2: usize>(self) -> Response<'a, PAYLOAD_LIMIT_2> {
        Response {
            res: self.res,
            timer: self.timer,
            timeout: self.timeout,
        }
    }

    /// Set response body collecting timeout duration. A response body failed to be collect
    /// in time would be canceled.
    ///
    /// Default to 15 seconds.
    #[inline]
    pub fn timeout(self, dur: Duration) -> Response<'a, PAYLOAD_LIMIT> {
        Response {
            res: self.res,
            timer: self.timer,
            timeout: dur,
        }
    }

    /// Collect response body as String. Response is consumed.
    #[inline]
    pub async fn string(self) -> Result<String, Error> {
        self.collect().await
    }

    /// Collect response body as Vec<u8>. Response is consumed.
    #[inline]
    pub async fn body(self) -> Result<Vec<u8>, Error> {
        self.collect().await
    }

    #[cfg(feature = "json")]
    /// Collect response body as json object. Response is consumed.
    ///
    /// The output type must impl [serde::de::DeserializeOwned] trait.
    pub async fn json<T>(self) -> Result<T, Error>
    where
        T: serde::de::DeserializeOwned,
    {
        use xitca_http::bytes::Buf;

        let bytes = self.collect::<BytesMut>().await?;
        Ok(serde_json::from_slice(bytes.chunk())?)
    }

    #[cfg(feature = "websocket")]
    pub fn ws(self) -> Result<crate::ws::WebSocket<'a>, Error> {
        let body = self.res.into_body();
        crate::ws::WebSocket::try_from_body(body)
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

        timer.as_mut().reset(Instant::now() + self.timeout);

        loop {
            match body.next().timeout(timer.as_mut()).await {
                Ok(Some(res)) => {
                    let buf = match res {
                        Ok(buf) => buf,
                        // all error path should destroy connection on drop.
                        Err(e) => {
                            body.destroy_on_drop();
                            return Err(e.into());
                        }
                    };

                    b.try_extend_from_slice(&buf)?;

                    if buf.len() > limit {
                        debug!("PAYLOAD_LIMIT reached and only part of the response body is collected.");
                        body.destroy_on_drop();
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    body.destroy_on_drop();
                    return Err(TimeoutError::Response.into());
                }
            }
        }

        Ok(b)
    }

    // TODO: use a type to collect all information needed for testing.
    #[doc(hidden)]
    #[cold]
    #[inline(never)]
    /// Public API for test purpose.
    ///
    /// Used for testing server implementation to make sure it follows spec.
    pub fn can_close_connection(&mut self) -> bool {
        self.res.body_mut().can_destroy_on_drop()
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
