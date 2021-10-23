use std::time::Duration;

/// Configuration for various timeout setting for http client.
pub struct TimeoutConfig {
    /// Timeout for establishing http connection for the first time.
    /// Default to 5 seconds.
    pub(crate) connect_timeout: Duration,
    /// Timeout for tls handshake when tls features enabled.
    /// Default to 3 seconds.
    pub(crate) tls_connect_timeout: Duration,
    /// Timeout for request go through keep-alived connection.
    /// Default to 10 seconds.
    pub(crate) request_timeout: Duration,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            tls_connect_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(10),
        }
    }
}
