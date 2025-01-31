//! client ip address based rate limiting.

use core::time::Duration;

use http_rate::Quota;

use crate::service::Service;

/// builder for client ip address based rate limiting middleware.
///
/// # Examples
/// ```rust
/// # use xitca_web::{handler::handler_service, middleware::rate_limit::RateLimit, route::get, App, WebContext};
/// App::new()
///     .at("/", get(handler_service(|| async { "hello,world!" })))
///     # .at("/infer", handler_service(|_: &WebContext<'_>| async{ "infer type" }))
///     // rate limit to 60 rps for one ip address.
///     .enclosed(RateLimit::per_minute(60));
/// ```
pub struct RateLimit(Quota);

macro_rules! constructor {
    ($method: tt) => {
        #[doc = concat!("Construct a RateLimit for a number of cells ",stringify!($method)," period. The given number of cells is")]
        /// also assumed to be the maximum burst size.
        ///
        /// # Panics
        /// - When max_burst is zero.
        pub fn $method(max_burst: u32) -> Self {
            Self(Quota::$method(max_burst))
        }
    };
}

impl RateLimit {
    constructor!(per_second);
    constructor!(per_minute);
    constructor!(per_hour);

    /// Construct a RateLimit that replenishes one cell in a given
    /// interval.
    ///
    /// # Panics
    /// - When the Duration is zero.
    pub fn with_period(replenish_1_per: Duration) -> Self {
        Self(Quota::with_period(replenish_1_per).unwrap())
    }
}

impl<S, E> Service<Result<S, E>> for RateLimit {
    type Response = service::RateLimitService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| service::RateLimitService {
            service,
            rate_limit: http_rate::RateLimit::new(self.0),
        })
    }
}

mod service {
    use core::convert::Infallible;

    use crate::{
        WebContext,
        body::ResponseBody,
        error::Error,
        http::WebResponse,
        service::{Service, ready::ReadyService},
    };

    pub struct RateLimitService<S> {
        pub(super) service: S,
        pub(super) rate_limit: http_rate::RateLimit,
    }

    impl<'r, C, B, S, ResB> Service<WebContext<'r, C, B>> for RateLimitService<S>
    where
        S: for<'r2> Service<WebContext<'r2, C, B>, Response = WebResponse<ResB>, Error = Error>,
    {
        type Response = WebResponse<ResB>;
        type Error = Error;

        async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
            let headers = ctx.req().headers();
            let addr = ctx.req().body().socket_addr();
            let snap = self.rate_limit.rate_limit(headers, addr).map_err(Error::from_service)?;
            self.service.call(ctx).await.map(|mut res| {
                snap.extend_response(&mut res);
                res
            })
        }
    }

    impl<'r, C, B> Service<WebContext<'r, C, B>> for http_rate::TooManyRequests {
        type Response = WebResponse;
        type Error = Infallible;

        async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
            let mut res = ctx.into_response(ResponseBody::empty());
            self.extend_response(&mut res);
            Ok(res)
        }
    }

    impl<S> ReadyService for RateLimitService<S>
    where
        S: ReadyService,
    {
        type Ready = S::Ready;

        #[inline]
        async fn ready(&self) -> Self::Ready {
            self.service.ready().await
        }
    }
}
