use std::time::Duration;

#[derive(Copy, Clone)]
pub struct HttpServiceConfig {
    pub(crate) http1_pipeline: bool,
    pub(crate) keep_alive_timeout: Duration,
    pub(crate) first_request_timeout: Duration,
    pub(crate) tls_accept_timeout: Duration,
    pub(crate) max_head_size: usize,
}

impl Default for HttpServiceConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpServiceConfig {
    pub const fn new() -> Self {
        Self {
            http1_pipeline: false,
            keep_alive_timeout: Duration::from_secs(5),
            first_request_timeout: Duration::from_secs(5),
            tls_accept_timeout: Duration::from_secs(3),
            max_head_size: 1024 * 1024,
        }
    }

    pub fn enable_http1_pipeline(mut self) -> Self {
        self.http1_pipeline = true;
        self
    }

    pub fn keep_alive_timeout(mut self, dur: Duration) -> Self {
        self.keep_alive_timeout = dur;
        self
    }

    pub fn first_request_timeout(mut self, dur: Duration) -> Self {
        self.first_request_timeout = dur;
        self
    }

    pub fn tls_accept_timeout(mut self, dur: Duration) -> Self {
        self.tls_accept_timeout = dur;
        self
    }

    pub fn max_head_size(mut self, bytes: usize) -> Self {
        self.max_head_size = bytes;
        self
    }
}
