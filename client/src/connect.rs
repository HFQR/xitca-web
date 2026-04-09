use core::{fmt, iter, net::SocketAddr};

use std::collections::vec_deque::{self, VecDeque};

use crate::uri::Uri;

pub trait Address {
    /// Get hostname part.
    fn hostname(&self) -> &str;

    /// Get optional port part.
    fn port(&self) -> Option<u16> {
        None
    }
}

impl Address for Uri<'_> {
    fn hostname(&self) -> &str {
        self.host().unwrap_or("")
    }

    fn port(&self) -> Option<u16> {
        match self.port_u16() {
            Some(port) => Some(port),
            None => scheme_to_port(self.scheme_str()),
        }
    }
}

// Get port from well-known URL schemes.
fn scheme_to_port(scheme: Option<&str>) -> Option<u16> {
    match scheme {
        // HTTP
        Some("http" | "ws") => Some(80),

        // HTTP Tls
        Some("https" | "wss") => Some(443),

        // Advanced Message Queuing Protocol (AMQP)
        Some("amqp") => Some(5672),
        Some("amqps") => Some(5671),

        // Message Queuing Telemetry Transport (MQTT)
        Some("mqtt") => Some(1883),
        Some("mqtts") => Some(8883),

        // File Transfer Protocol (FTP)
        Some("ftp") => Some(1883),
        Some("ftps") => Some(990),

        _ => None,
    }
}

#[derive(Debug, Default, Eq, PartialEq, Hash)]
pub(crate) enum Addrs {
    #[default]
    None,
    One(SocketAddr),
    Multi(VecDeque<SocketAddr>),
}

impl From<Option<SocketAddr>> for Addrs {
    fn from(addr: Option<SocketAddr>) -> Self {
        match addr {
            Some(addr) => Self::One(addr),
            None => Self::None,
        }
    }
}

/// Connection info.
#[derive(Debug)]
pub struct Connect<'a> {
    pub(crate) uri: Uri<'a>,
    pub(crate) port: u16,
    pub(crate) addr: Addrs,
}

impl<'a> Connect<'a> {
    /// Create `Connect` instance by splitting the string by ':' and convert the second part to u16
    pub fn new(uri: Uri<'a>, address: Option<SocketAddr>) -> Self {
        let (_, port) = parse_host(uri.hostname());

        Self {
            uri,
            port: port.unwrap_or(0),
            addr: match address {
                Some(address) => Addrs::One(address),
                None => Addrs::None,
            },
        }
    }

    /// Set list of addresses.
    pub fn set_addrs<I>(&mut self, addrs: I)
    where
        I: IntoIterator<Item = SocketAddr>,
    {
        let mut addrs = VecDeque::from_iter(addrs);
        self.addr = if addrs.len() < 2 {
            Addrs::from(addrs.pop_front())
        } else {
            Addrs::Multi(addrs)
        };
    }

    /// Get hostname.
    pub fn hostname(&self) -> &str {
        self.uri.hostname()
    }

    /// Get request port.
    pub fn port(&self) -> u16 {
        Address::port(&self.uri).unwrap_or(self.port)
    }

    /// Get resolved request addresses.
    pub fn addrs(&self) -> AddrsIter<'_> {
        match self.addr {
            Addrs::None => AddrsIter::None,
            Addrs::One(addr) => AddrsIter::One(addr),
            Addrs::Multi(ref addrs) => AddrsIter::Multi(addrs.iter()),
        }
    }

    /// Check if address is resolved.
    pub fn is_resolved(&self) -> bool {
        match self.addr {
            Addrs::None => false,
            Addrs::One(_) => true,
            Addrs::Multi(ref addrs) => !addrs.is_empty(),
        }
    }
}

impl fmt::Display for Connect<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.hostname(), self.port())
    }
}

/// Iterator over addresses in a [`Connect`] request.
#[derive(Clone)]
pub enum AddrsIter<'a> {
    None,
    One(SocketAddr),
    Multi(vec_deque::Iter<'a, SocketAddr>),
}

impl Iterator for AddrsIter<'_> {
    type Item = SocketAddr;

    fn next(&mut self) -> Option<Self::Item> {
        match *self {
            Self::None => None,
            Self::One(addr) => {
                *self = Self::None;
                Some(addr)
            }
            Self::Multi(ref mut iter) => iter.next().copied(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match *self {
            Self::None => (0, Some(0)),
            Self::One(_) => (1, Some(1)),
            Self::Multi(ref iter) => iter.size_hint(),
        }
    }
}

impl fmt::Debug for AddrsIter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl iter::ExactSizeIterator for AddrsIter<'_> {}

impl iter::FusedIterator for AddrsIter<'_> {}

fn parse_host(host: &str) -> (&str, Option<u16>) {
    let mut parts_iter = host.splitn(2, ':');

    match parts_iter.next() {
        Some(hostname) => {
            let port_str = parts_iter.next().unwrap_or("");
            let port = port_str.parse::<u16>().ok();
            (hostname, port)
        }

        None => (host, None),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn test_host_parser() {
        assert_eq!(parse_host("example.com"), ("example.com", None));
        assert_eq!(parse_host("example.com:8080"), ("example.com", Some(8080)));
        assert_eq!(parse_host("example:8080"), ("example", Some(8080)));
        assert_eq!(parse_host("example.com:false"), ("example.com", None));
        assert_eq!(parse_host("example.com:false:false"), ("example.com", None));
    }

    #[test]
    fn test_addr_iter_multi() {
        let localhost = SocketAddr::from((IpAddr::from(Ipv4Addr::LOCALHOST), 8080));
        let unspecified = SocketAddr::from((IpAddr::from(Ipv4Addr::UNSPECIFIED), 8080));

        let mut addrs = VecDeque::new();
        addrs.push_back(localhost);
        addrs.push_back(unspecified);

        let mut iter = AddrsIter::Multi(addrs.iter());
        assert_eq!(iter.next(), Some(localhost));
        assert_eq!(iter.next(), Some(unspecified));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_addr_iter_single() {
        let localhost = SocketAddr::from((IpAddr::from(Ipv4Addr::LOCALHOST), 8080));

        let mut iter = AddrsIter::One(localhost);
        assert_eq!(iter.next(), Some(localhost));
        assert_eq!(iter.next(), None);

        let mut iter = AddrsIter::None;
        assert_eq!(iter.next(), None);
    }
}
