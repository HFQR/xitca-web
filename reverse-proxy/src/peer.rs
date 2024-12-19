use crate::forwarder::forward_header::ForwardedFor;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViaHeader {
    pub(crate) add_in_request: bool,
    pub(crate) add_in_response: bool,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpPeer {
    pub(crate) forward_for: ForwardedFor,
    pub(crate) address: SocketAddr,
    pub(crate) sni_host: String,
    pub(crate) request_host: String,
    pub(crate) via: Option<ViaHeader>,
    pub(crate) allow_invalid_certificates: bool,
    pub(crate) supported_encodings: Option<HashSet<String>>,
    pub(crate) force_close: bool,
    pub(crate) tls: bool,
    pub(crate) request_body_size_limit: usize,
    pub(crate) timeout: Option<Duration>,
}

impl HttpPeer {
    /// Create a new `HttpPeer` with the given address and request host.
    pub fn new(address: SocketAddr, request_host: &str) -> Self {
        Self {
            forward_for: ForwardedFor::default(),
            address,
            sni_host: request_host.to_string(),
            request_host: request_host.to_string(),
            via: None,
            allow_invalid_certificates: false,
            supported_encodings: None,
            force_close: false,
            tls: address.port() == 443,
            request_body_size_limit: 1024 * 1024 * 16, // 16MB
            timeout: None,
        }
    }

    /// Set the `Forwarded-For` or `X-Forwarded-*` headers to the given value.
    pub fn forward_for(mut self, forward_for: ForwardedFor) -> Self {
        self.forward_for = forward_for;
        self
    }

    /// Set the SNI host to the given value. This is the host that will be used for the TLS handshake.
    pub fn sni_host(mut self, sni_host: String) -> Self {
        self.sni_host = sni_host;
        self
    }

    /// Set the `Host` header to the given value. This is the host that will be used for the HTTP request.
    pub fn request_host(mut self, request_host: String) -> Self {
        self.request_host = request_host;
        self
    }

    /// Set the `Via` header to the given value. This header can be added to the request fowarded to the peer and/or to the response returned to the client.
    pub fn via(mut self, via: &str, in_request: bool, in_response: bool) -> Self {
        self.via = Some(ViaHeader {
            add_in_request: in_request,
            add_in_response: in_response,
            name: via.to_string(),
        });

        self
    }

    /// Set whether invalid certificates should be allowed.
    pub fn allow_invalid_certificates(mut self, allow_invalid_certificates: bool) -> Self {
        self.allow_invalid_certificates = allow_invalid_certificates;
        self
    }

    /// A set of supported encodings that this server can handle, this may be useful if you need to
    /// update the response body and you cannot handle some encodings.
    pub fn supported_encodings(mut self, supported_encodings: HashSet<String>) -> Self {
        self.supported_encodings = Some(supported_encodings);
        self
    }

    /// Set whether the connection should be closed after the request has been forwarded to the peer.
    pub fn force_close(mut self, force_close: bool) -> Self {
        self.force_close = force_close;
        self
    }

    /// Set whether the connection should be encrypted using TLS when connecting to the peer.
    pub fn tls(mut self, tls: bool) -> Self {
        self.tls = tls;
        self
    }

    /// Set the timeout for the request to the peer.
    /// If the timeout is reached, the request will be aborted and an error will be returned to the client.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the maximum size of the request body that can be stored in memory.
    ///
    /// This limit should not be used in most cases as the request body is streamed and not stored in memory.
    /// However, there is some cases where the request body needs to be stored in memory, think
    /// of an old client sending HTTP/1.0 requests with a body. In this case the body will be stored
    /// in memory before being forwarded to the peer.
    ///
    /// This limit prevents the server from storing too much data in memory.
    /// The default value is 16MB.
    pub fn request_body_size_limit(mut self, request_body_size_limit: usize) -> Self {
        self.request_body_size_limit = request_body_size_limit;
        self
    }
}
