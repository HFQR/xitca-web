use tracing::{Level, warn};
use xitca_http::util::middleware;

use crate::service::Service;

/// builder for tracing log middleware.
///
/// # Examples
/// ```rust
/// # use xitca_web::{handler::handler_service, middleware::Logger, route::get, App, WebContext};
/// App::new()
///     .at("/", get(handler_service(|| async { "hello,world!" })))
///     # .at("/infer", handler_service(|_: &WebContext<'_>| async{ "infer type" }))
///     // log http request and error with default setting.
///     .enclosed(Logger::new());
/// ```
pub struct Logger {
    logger: middleware::Logger,
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}

impl Logger {
    /// construct a new logger middleware builder with [`Level::INFO`] of verbosity it generate and captures.
    /// would try to initialize global trace dispatcher.
    pub fn new() -> Self {
        Self::with_level(Level::INFO)
    }

    /// construct a new logger middleware builder with given [Level] of verbosity it generate and captures.
    /// would try to initialize global trace dispatcher.
    pub fn with_level(level: Level) -> Self {
        if let Err(e) = tracing_subscriber::fmt().with_max_level(level).try_init() {
            // the most likely case is trace dispatcher has already been set by user. log the warning and move on.
            warn!("failed to initialize global trace dispatcher: {}", e);
        }

        Self {
            logger: middleware::Logger::with_level(level),
        }
    }
}

impl<Arg> Service<Arg> for Logger
where
    middleware::Logger: Service<Arg>,
{
    type Response = <middleware::Logger as Service<Arg>>::Response;
    type Error = <middleware::Logger as Service<Arg>>::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        self.logger.call(arg).await
    }
}
