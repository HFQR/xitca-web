use tracing::Level;
use xitca_service::Service;

/// a builder for logger service.
#[derive(Clone)]
pub struct Logger {
    level: Level,
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}

impl Logger {
    /// construct a default logger with [`Level::WARN`]
    pub fn new() -> Self {
        Self::with_level(Level::WARN)
    }

    /// construct a logger with given [`Level`] verbosity.
    pub fn with_level(level: Level) -> Self {
        Self { level }
    }
}

impl<S, E> Service<Result<S, E>> for Logger {
    type Response = service::LoggerService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| service::LoggerService {
            service,
            level: self.level,
        })
    }
}

mod service {
    use std::error;

    use tracing::{Instrument, event, span};
    use xitca_service::ready::ReadyService;

    use crate::http::{BorrowReq, Method, Uri, header::HeaderMap};

    use super::*;

    pub struct LoggerService<S> {
        pub(super) service: S,
        pub(super) level: Level,
    }

    impl<S, Req> Service<Req> for LoggerService<S>
    where
        S: Service<Req>,
        Req: BorrowReq<Method> + BorrowReq<Uri> + BorrowReq<HeaderMap>,
        S::Error: error::Error,
    {
        type Response = S::Response;
        type Error = S::Error;

        #[inline]
        async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
            let method: &Method = req.borrow();
            let uri: &Uri = req.borrow();

            macro_rules! span2 {
                ($lvl:expr, $name:expr, $($fields:tt)*) => {
                    match $lvl {
                        Level::TRACE => span!(Level::TRACE, $name, $($fields)*),
                        Level::DEBUG => span!(Level::DEBUG, $name, $($fields)*),
                        Level::INFO => span!(Level::INFO, $name, $($fields)*),
                        Level::WARN => span!(Level::WARN, $name, $($fields)*),
                        Level::ERROR => span!(Level::ERROR, $name, $($fields)*),
                    }
                }
            }

            let span = span2!(
                self.level,
                "request",
                method = %method,
                uri = %uri
            );

            async {
                event!(target: "on_request", Level::INFO, "serving request");
                match self.service.call(req).await {
                    Ok(res) => {
                        event!(target: "on_response", Level::INFO, "sending response");
                        Ok(res)
                    }
                    Err(e) => {
                        event!(target: "on_error", Level::WARN, "{}", e);
                        Err(e)
                    }
                }
            }
            .instrument(span)
            .await
        }
    }

    impl<S> ReadyService for LoggerService<S>
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
