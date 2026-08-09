//! Configuration for http service middlewares.

use core::time::Duration;

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

#[derive(Clone)]
pub struct HttpServiceConfig<
    const HEADER_LIMIT: usize = DEFAULT_HEADER_LIMIT,
    const READ_BUF_LIMIT: usize = DEFAULT_READ_BUF_LIMIT,
    const WRITE_BUF_LIMIT: usize = DEFAULT_WRITE_BUF_LIMIT,
> {
    pub(crate) vectored_write: bool,
    pub(crate) keep_alive_timeout: Duration,
    pub(crate) request_head_timeout: Duration,
    pub(crate) tls_accept_timeout: Duration,
    pub(crate) peek_protocol: bool,
    pub(crate) h2_max_concurrent_streams: u32,
    pub(crate) h2_initial_window_size: u32,
    pub(crate) h2_max_frame_size: u32,
    pub(crate) h2_max_header_list_size: u32,
}

impl Default for HttpServiceConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpServiceConfig {
    pub const fn new() -> Self {
        Self {
            vectored_write: true,
            keep_alive_timeout: Duration::from_secs(5),
            request_head_timeout: Duration::from_secs(5),
            tls_accept_timeout: Duration::from_secs(3),
            peek_protocol: false,
            h2_max_concurrent_streams: 256,
            h2_initial_window_size: 65_535,
            h2_max_frame_size: 16_384,
            h2_max_header_list_size: 16 * 1024 * 1024,
        }
    }
}

impl<const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    /// Disable vectored write even when IO is able to perform it.
    ///
    /// This is beneficial when dealing with small size of response body.
    pub const fn disable_vectored_write(mut self) -> Self {
        self.vectored_write = false;
        self
    }

    /// Define duration of how long an idle connection is kept alive.
    ///
    /// connection have not done any IO after duration would be closed. IO operation
    /// can possibly result in reset of the duration.
    pub const fn keep_alive_timeout(mut self, dur: Duration) -> Self {
        self.keep_alive_timeout = dur;
        self
    }

    /// Define duration of how long a connection must finish it's request head transferring.
    /// starting from first byte(s) of current request(s) received from peer.
    ///
    /// connection can not make a single request after duration would be closed.
    pub const fn request_head_timeout(mut self, dur: Duration) -> Self {
        self.request_head_timeout = dur;
        self
    }

    /// Define duration of how long a connection must finish it's tls handshake.
    /// (If tls is enabled)
    ///
    /// Connection can not finish handshake after duration would be closed.
    pub const fn tls_accept_timeout(mut self, dur: Duration) -> Self {
        self.tls_accept_timeout = dur;
        self
    }

    /// Define max read buffer size for a connection.
    ///
    /// See [DEFAULT_READ_BUF_LIMIT] for default value and behavior.
    ///
    /// # Panics
    /// Panics when `READ_BUF_LIMIT_2 < 32` or when `READ_BUF_LIMIT_2 < h2_max_frame_size + 9`.
    /// The read buffer must be large enough to hold a full HTTP/2 frame (max payload + 9 byte
    /// header), otherwise the dispatcher would deadlock waiting for a frame that can never fit.
    ///
    /// When called in a const context (e.g. `const CFG: _ = HttpServiceConfig::new()...`)
    /// a violation is reported as a compile-time error instead of a runtime panic.
    pub const fn max_read_buf_size<const READ_BUF_LIMIT_2: usize>(
        self,
    ) -> HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT> {
        assert!(READ_BUF_LIMIT_2 >= 32, "READ_BUF_LIMIT must be no less than 32 bytes");
        h2_frame_read_buf_check(READ_BUF_LIMIT_2, self.h2_max_frame_size as _);
        self.mutate_const_generic::<HEADER_LIMIT, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT>()
    }

    /// Define max write buffer size for a connection.
    ///
    /// See [DEFAULT_WRITE_BUF_LIMIT] for default value
    /// and behavior.
    pub const fn max_write_buf_size<const WRITE_BUF_LIMIT_2: usize>(
        self,
    ) -> HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT_2> {
        self.mutate_const_generic::<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT_2>()
    }

    /// Define max request header count for a connection.    
    ///
    /// See [DEFAULT_HEADER_LIMIT] for default value
    /// and behavior.
    pub const fn max_request_headers<const HEADER_LIMIT_2: usize>(
        self,
    ) -> HttpServiceConfig<HEADER_LIMIT_2, READ_BUF_LIMIT, WRITE_BUF_LIMIT> {
        self.mutate_const_generic::<HEADER_LIMIT_2, READ_BUF_LIMIT, WRITE_BUF_LIMIT>()
    }

    /// Define the maximum number of concurrent HTTP/2 streams per connection.
    pub const fn h2_max_concurrent_streams(mut self, val: u32) -> Self {
        self.h2_max_concurrent_streams = val;
        self
    }

    /// Define the initial flow-control window size for HTTP/2 streams.
    ///
    /// Must not exceed 2^31-1 (2,147,483,647).
    pub const fn h2_initial_window_size(mut self, val: u32) -> Self {
        self.h2_initial_window_size = val;
        self
    }

    /// Define the maximum HTTP/2 frame size the server is willing to receive.
    ///
    /// Must be between 16,384 and 16,777,215 (inclusive).
    ///
    /// # Panics
    /// Panics when `READ_BUF_LIMIT < val + 9`. The read buffer must be large enough to hold
    /// a full HTTP/2 frame (payload + 9 byte header), otherwise the dispatcher would deadlock
    /// waiting for a frame that can never fit. Raise the read buffer first via
    /// [`max_read_buf_size`](Self::max_read_buf_size) when increasing the frame size.
    ///
    /// When called in a const context (e.g. `const CFG: _ = HttpServiceConfig::new()...`)
    /// a violation is reported as a compile-time error instead of a runtime panic.
    pub const fn h2_max_frame_size(mut self, val: u32) -> Self {
        h2_frame_read_buf_check(READ_BUF_LIMIT, val as _);
        self.h2_max_frame_size = val;
        self
    }

    /// Define the maximum size of HTTP/2 header list the server is willing to accept.
    pub const fn h2_max_header_list_size(mut self, val: u32) -> Self {
        self.h2_max_header_list_size = val;
        self
    }

    /// Enable peek into connection to figure out it's protocol regardless the outcome
    /// of alpn negotiation.
    ///
    /// This API is used to bypass alpn setting from tls and enable Http/2 protocol over
    /// plain Tcp connection.
    pub const fn peek_protocol(mut self) -> Self {
        self.peek_protocol = true;
        self
    }

    #[doc(hidden)]
    /// A shortcut for mutating const generic params.
    pub const fn mutate_const_generic<
        const HEADER_LIMIT2: usize,
        const READ_BUF_LIMIT2: usize,
        const WRITE_BUF_LIMIT2: usize,
    >(
        self,
    ) -> HttpServiceConfig<HEADER_LIMIT2, READ_BUF_LIMIT2, WRITE_BUF_LIMIT2> {
        HttpServiceConfig {
            vectored_write: self.vectored_write,
            keep_alive_timeout: self.keep_alive_timeout,
            request_head_timeout: self.request_head_timeout,
            tls_accept_timeout: self.tls_accept_timeout,
            peek_protocol: self.peek_protocol,
            h2_max_concurrent_streams: self.h2_max_concurrent_streams,
            h2_initial_window_size: self.h2_initial_window_size,
            h2_max_frame_size: self.h2_max_frame_size,
            h2_max_header_list_size: self.h2_max_header_list_size,
        }
    }
}

const fn h2_frame_read_buf_check(read_buf_size: usize, h2_max_frame_size: usize) {
    assert!(
        read_buf_size >= (h2_max_frame_size + 9),
        "max_read_buf_size must be at least h2_max_frame_size + 9 for HTTP2 to work"
    );
}
