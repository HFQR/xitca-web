//! gRPC timeout middleware.
//!
//! Parses the `grpc-timeout` request header and enforces it as a deadline on the inner service call.
//! If the deadline is exceeded, a trailers-only response with `grpc-status: 4` (DeadlineExceeded) is returned.
//!
//! The parsed deadline [`Instant`] is inserted into request extensions so downstream extractors and
//! handlers can observe the remaining time.
//!
//! # Example
//! ```rust
//! # use xitca_web::{handler::handler_service, App, WebContext};
//! use xitca_web::middleware::grpc_timeout::GrpcTimeout;
//!
//! App::new()
//!     .at("/my.Service/Method", handler_service(handler))
//!     .enclosed(GrpcTimeout);
//!
//! # async fn handler(_: &WebContext<'_>) -> &'static str { "" }
//! ```

use core::time::Duration;

use tokio::time::Instant;

use crate::{
    body::{BodyExt, Empty, ResponseBody, Trailers},
    bytes::Bytes,
    context::WebContext,
    error::{GrpcError, GrpcStatus},
    http::{
        WebResponse,
        const_header_name::GRPC_TIMEOUT,
        const_header_value::GRPC,
        header::{CONTENT_TYPE, HeaderValue},
    },
    service::Service,
};

/// Middleware that enforces the `grpc-timeout` deadline on the inner service call.
///
/// If the header is absent, no timeout is applied.
/// The deadline [`Instant`] is inserted into request extensions.
pub struct GrpcTimeout;

impl<S, E> Service<Result<S, E>> for GrpcTimeout {
    type Response = GrpcTimeoutService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| GrpcTimeoutService { service })
    }
}

pub struct GrpcTimeoutService<S> {
    service: S,
}

impl<'r, S, C, B> Service<WebContext<'r, C, B>> for GrpcTimeoutService<S>
where
    S: for<'r2> Service<WebContext<'r2, C, B>, Response = WebResponse, Error = crate::error::Error>,
{
    type Response = WebResponse;
    type Error = crate::error::Error;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let timeout = ctx.req().headers().get(GRPC_TIMEOUT).and_then(parse_grpc_timeout);

        match timeout {
            Some(duration) => {
                let deadline = Instant::now() + duration;

                match tokio::time::timeout_at(deadline, self.service.call(ctx)).await {
                    Ok(result) => result,
                    Err(_elapsed) => {
                        let err = GrpcError::new(GrpcStatus::DeadlineExceeded, "deadline exceeded");
                        let body = Empty::<Bytes>::new().chain(Trailers::new(err.trailers()));
                        let mut res = WebResponse::new(ResponseBody::boxed(body));
                        res.headers_mut().insert(CONTENT_TYPE, GRPC);
                        Ok(res)
                    }
                }
            }
            None => self.service.call(ctx).await,
        }
    }
}

/// Parse the `grpc-timeout` header value into a [`Duration`].
///
/// Format: `{value}{unit}` where value is 1-8 ASCII digits and unit is one of:
/// - `H` (hours), `M` (minutes), `S` (seconds)
/// - `m` (milliseconds), `u` (microseconds), `n` (nanoseconds)
fn parse_grpc_timeout(value: &HeaderValue) -> Option<Duration> {
    let bytes = value.as_bytes();
    if bytes.len() < 2 {
        return None;
    }

    let (digits, unit) = bytes.split_at(bytes.len() - 1);

    // spec says max 8 digits
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }

    let mut val: u64 = 0;
    for &b in digits {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val * 10 + (b - b'0') as u64;
    }

    match unit[0] {
        b'H' => Some(Duration::from_secs(val * 3600)),
        b'M' => Some(Duration::from_secs(val * 60)),
        b'S' => Some(Duration::from_secs(val)),
        b'm' => Some(Duration::from_millis(val)),
        b'u' => Some(Duration::from_micros(val)),
        b'n' => Some(Duration::from_nanos(val)),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_timeout_values() {
        assert_eq!(
            parse_grpc_timeout(&HeaderValue::from_static("1H")),
            Some(Duration::from_secs(3600))
        );
        assert_eq!(
            parse_grpc_timeout(&HeaderValue::from_static("5M")),
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            parse_grpc_timeout(&HeaderValue::from_static("10S")),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            parse_grpc_timeout(&HeaderValue::from_static("100m")),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            parse_grpc_timeout(&HeaderValue::from_static("5000u")),
            Some(Duration::from_micros(5000))
        );
        assert_eq!(
            parse_grpc_timeout(&HeaderValue::from_static("999n")),
            Some(Duration::from_nanos(999))
        );
    }

    #[test]
    fn parse_timeout_invalid() {
        assert_eq!(parse_grpc_timeout(&HeaderValue::from_static("H")), None); // no digits
        assert_eq!(parse_grpc_timeout(&HeaderValue::from_static("5")), None); // no unit
        assert_eq!(parse_grpc_timeout(&HeaderValue::from_static("5x")), None); // bad unit
        assert_eq!(parse_grpc_timeout(&HeaderValue::from_static("abc")), None); // non-digit
        assert_eq!(parse_grpc_timeout(&HeaderValue::from_static("123456789S")), None); // 9 digits > max 8
    }

    #[test]
    fn parse_timeout_max_digits() {
        // exactly 8 digits should work
        assert_eq!(
            parse_grpc_timeout(&HeaderValue::from_static("99999999S")),
            Some(Duration::from_secs(99999999))
        );
    }
}
