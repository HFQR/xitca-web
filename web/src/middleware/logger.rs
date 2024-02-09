use core::marker::PhantomData;

use tracing::Level;
use xitca_http::util::middleware::Logger;

use crate::service::Service;

pub struct LoggerBuilder<S = state::InternalSub> {
    logger: Logger,
    _state: PhantomData<S>,
}

impl<S> LoggerBuilder<S> {
    fn with_logger(logger: Logger) -> Self {
        Self {
            logger,
            _state: PhantomData,
        }
    }
}

// type state for determine if internal tracing subscriber should be created.
mod state {
    pub struct InternalSub;
    pub struct ExternalSub;
    pub struct Finalized;
}

impl LoggerBuilder {
    pub fn new() -> Self {
        Self {
            logger: Logger::new(),
            _state: PhantomData,
        }
    }

    pub fn set_level(mut self, level: Level) -> Self {
        self.logger = self.logger.set_level(level);
        self
    }
}

impl<S> LoggerBuilder<S> {
    pub fn external_sub(self) -> LoggerBuilder<state::ExternalSub> {
        LoggerBuilder::with_logger(self.logger)
    }
}

impl LoggerBuilder<state::InternalSub> {
    /// finalize builder and ready to produce logger middleware service.
    /// a default tracing subscriber is constructed at the same time.
    pub fn init(self) -> LoggerBuilder<state::Finalized> {
        tracing_subscriber::fmt().init();
        LoggerBuilder::with_logger(self.logger)
    }
}

impl LoggerBuilder<state::ExternalSub> {
    /// finalize builder and ready to produce logger middleware service.
    /// no default tracing subscriber would be constructed.
    pub fn init(self) -> LoggerBuilder<state::Finalized> {
        LoggerBuilder::with_logger(self.logger)
    }
}

impl<Arg> Service<Arg> for LoggerBuilder<state::Finalized>
where
    Logger: Service<Arg>,
{
    type Response = <Logger as Service<Arg>>::Response;
    type Error = <Logger as Service<Arg>>::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        self.logger.call(arg).await
    }
}
