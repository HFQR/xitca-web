use std::time::Duration;

#[derive(Copy, Clone)]
pub struct HttpServiceConfig {
    pub(crate) http1_pipeline: bool,
    pub(crate) keep_alive_dur: Duration,
    pub(crate) first_request_dur: Duration,
    pub(crate) tls_accept_dur: Duration,
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
            keep_alive_dur: Duration::from_secs(5),
            first_request_dur: Duration::from_secs(5),
            tls_accept_dur: Duration::from_secs(3),
        }
    }

    pub fn enable_http1_pipeline(mut self) -> Self {
        self.http1_pipeline = true;
        self
    }

    pub fn keep_alive_dur(mut self, dur: Duration) -> Self {
        self.keep_alive_dur = dur;
        self
    }

    pub fn first_request_dur(mut self, dur: Duration) -> Self {
        self.first_request_dur = dur;
        self
    }

    pub fn tls_accept_dur(mut self, dur: Duration) -> Self {
        self.tls_accept_dur = dur;
        self
    }
}
