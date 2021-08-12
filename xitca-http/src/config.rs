use std::time::Duration;

/// The default maximum read buffer size. If the head gets this big and
/// a message is still not complete, a `TooLarge` error is triggered.
///
/// When handing request body ff the buffer gets this big a force yield
/// from Io stream read would happen.
pub const DEFAULT_READ_BUF_LIMIT: usize = 1024 * 1024;

/// The default maximum write buffer size. If the buffer gets this big and
/// a message is still not complete, a force draining of Io stream write
/// would happen.
pub const DEFAULT_WRITE_BUF_LIMIT: usize = 8192 + 4096 * 100;

/// The default maximum request header fields possible for one request.
///
/// 64 chosen for no particular reason.
pub const DEFAULT_HEADER_LIMIT: usize = 64;

#[derive(Copy, Clone)]
pub struct HttpServiceConfig<const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> {
    pub(crate) force_flat_buf: bool,
    pub(crate) keep_alive_timeout: Duration,
    pub(crate) first_request_timeout: Duration,
    pub(crate) tls_accept_timeout: Duration,
    pub(crate) peek_protocol: bool,
}

impl Default for HttpServiceConfig<DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT> {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpServiceConfig<DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT> {
    pub const fn new() -> Self {
        Self {
            force_flat_buf: false,
            keep_alive_timeout: Duration::from_secs(5),
            first_request_timeout: Duration::from_secs(5),
            tls_accept_timeout: Duration::from_secs(3),
            peek_protocol: false,
        }
    }
}

impl<const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    /// Force IO write always use a flat buffer where extra data copy is preferred.
    ///
    /// This is beneficial when dealing with small size of response body.
    pub fn force_flat_buf(mut self) -> Self {
        self.force_flat_buf = true;
        self
    }

    /// Define duration of how long a connection is kept alive.
    pub fn keep_alive_timeout(mut self, dur: Duration) -> Self {
        self.keep_alive_timeout = dur;
        self
    }

    /// Define duration of how long a connection must finish it's first request.
    ///
    /// Connection too slow to make a request after this duration would be closed.
    pub fn first_request_timeout(mut self, dur: Duration) -> Self {
        self.first_request_timeout = dur;
        self
    }

    /// Define duration of how long a connection must finish it's tls handshake.
    /// (If tls is enabled)
    ///
    /// Connection too slow to do handshake after this duration would be closed.
    pub fn tls_accept_timeout(mut self, dur: Duration) -> Self {
        self.tls_accept_timeout = dur;
        self
    }

    /// Define max read buffer size for a connection.
    ///
    /// See [DEFAULT_READ_BUF_LIMIT](DEFAULT_READ_BUF_LIMIT) for default value
    /// and behavior.
    pub fn max_read_buf_size<const READ_BUF_LIMIT_2: usize>(
        self,
    ) -> HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT> {
        self.mutate_const_generic::<HEADER_LIMIT, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT>()
    }

    /// Define max write buffer size for a connection.
    ///
    /// See [DEFAULT_WRITE_BUF_LIMIT](DEFAULT_WRITE_BUF_LIMIT) for default value
    /// and behavior.
    pub fn max_write_buf_size<const WRITE_BUF_LIMIT_2: usize>(
        self,
    ) -> HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT_2> {
        self.mutate_const_generic::<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT_2>()
    }

    /// Define max request header count for a connection.    
    ///
    /// See [DEFAULT_HEADER_LIMIT](DEFAULT_HEADER_LIMIT) for default value
    /// and behavior.
    pub fn max_request_headers<const HEADER_LIMIT_2: usize>(
        self,
    ) -> HttpServiceConfig<HEADER_LIMIT_2, READ_BUF_LIMIT, WRITE_BUF_LIMIT> {
        self.mutate_const_generic::<HEADER_LIMIT_2, READ_BUF_LIMIT, WRITE_BUF_LIMIT>()
    }

    /// Enable peek into connection to figure out it's protocol regardless the outcome
    /// of alpn negotiation.
    ///
    /// This API is used to bypass alpn setting from tls and enable Http/2 protocol over
    /// plain Tcp connection.
    pub fn peek_protocol(mut self) -> Self {
        self.peek_protocol = true;
        self
    }

    #[doc(hidden)]
    /// A shortcut for mutating const generic params.
    pub fn mutate_const_generic<
        const HEADER_LIMIT2: usize,
        const READ_BUF_LIMIT2: usize,
        const WRITE_BUF_LIMIT2: usize,
    >(
        self,
    ) -> HttpServiceConfig<HEADER_LIMIT2, READ_BUF_LIMIT2, WRITE_BUF_LIMIT2> {
        HttpServiceConfig {
            force_flat_buf: self.force_flat_buf,
            keep_alive_timeout: self.keep_alive_timeout,
            first_request_timeout: self.first_request_timeout,
            tls_accept_timeout: self.tls_accept_timeout,
            peek_protocol: self.peek_protocol,
        }
    }
}
