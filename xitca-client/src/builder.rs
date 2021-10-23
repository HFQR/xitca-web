use std::time::Duration;

use crate::client::Client;
use crate::pool::Pool;
use crate::resolver::{Resolve, Resolver};
use crate::timeout::TimeoutConfig;
use crate::tls::connector::Connector;

pub struct ClientBuilder {
    connector: Connector,
    resolver: Resolver,
    pool_capacity: usize,
    timeout_config: TimeoutConfig,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        ClientBuilder {
            connector: Connector::default(),
            resolver: Resolver::default(),
            pool_capacity: 128,
            timeout_config: TimeoutConfig::default(),
        }
    }
}

impl ClientBuilder {
    /// Use custom DNS resolver for domain look up.
    ///
    /// See [Resolve] for detail.
    pub fn recsolver(mut self, resolver: impl Resolve + 'static) -> Self {
        self.resolver = Resolver::custom(resolver);
        self
    }

    /// Set timeout for establishing connection.
    ///
    /// See [TimeoutConfig] for detail.
    pub fn set_connect_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.connect_timeout = dur;
        self
    }

    /// Set timeout for tls handshake.
    ///
    /// See [TimeoutConfig] for detail.
    pub fn set_tls_connect_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.tls_connect_timeout = dur;
        self
    }

    /// Set timeout for request.
    ///
    /// See [TimeoutConfig] for detail.
    pub fn set_request_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.request_timeout = dur;
        self
    }

    /// Set capacity of the connection pool for re-useable connection.
    ///
    /// Default to 128
    ///
    /// # Panics:
    /// When pass 0 as pool capacity.
    pub fn pool_capacity(mut self, cap: usize) -> Self {
        assert_ne!(cap, 0);
        self.pool_capacity = cap;
        self
    }

    pub fn finish(self) -> Client {
        Client {
            pool: Pool::with_capacity(self.pool_capacity),
            connector: self.connector,
            resolver: self.resolver,
            timeout_config: self.timeout_config,
        }
    }
}
