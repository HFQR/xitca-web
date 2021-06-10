use std::time::Duration;

pub const DEFAULT_HEAD_LIMIT: usize = 1024 * 1024;

#[derive(Copy, Clone)]
pub struct HttpServiceConfig<const HEAD_LIMIT: usize> {
    pub(crate) http1_pipeline: bool,
    pub(crate) keep_alive_timeout: Duration,
    pub(crate) first_request_timeout: Duration,
    pub(crate) tls_accept_timeout: Duration,
}

impl Default for HttpServiceConfig<DEFAULT_HEAD_LIMIT> {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpServiceConfig<DEFAULT_HEAD_LIMIT> {
    pub const fn new() -> Self {
        Self {
            http1_pipeline: false,
            keep_alive_timeout: Duration::from_secs(5),
            first_request_timeout: Duration::from_secs(5),
            tls_accept_timeout: Duration::from_secs(3),
        }
    }
}

impl<const HEAD_LIMIT: usize> HttpServiceConfig<HEAD_LIMIT> {
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

    pub fn max_head_size<const NEW_LIMIT: usize>(self) -> HttpServiceConfig<NEW_LIMIT> {
        HttpServiceConfig {
            http1_pipeline: self.http1_pipeline,
            keep_alive_timeout: self.keep_alive_timeout,
            first_request_timeout: self.first_request_timeout,
            tls_accept_timeout: self.tls_accept_timeout,
        }
    }
}
