use std::{
    collections::vec_deque::{self, VecDeque},
    fmt, iter, mem,
    net::{IpAddr, SocketAddr},
};

use crate::uri::Uri;

pub trait Address {
    /// Get hostname part.
    fn hostname(&self) -> &str;

    /// Get optional port part.
    fn port(&self) -> Option<u16> {
        None
    }
}

impl Address for Uri {
    fn hostname(&self) -> &str {
        // TODO: handle the None variant.
        self.host().unwrap_or("")
    }

    fn port(&self) -> Option<u16> {
        self.port_u16()
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub(crate) enum Addrs {
    None,
    One(SocketAddr),
    Multi(VecDeque<SocketAddr>),
}

impl Addrs {
    pub(crate) fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub(crate) fn is_some(&self) -> bool {
        !self.is_none()
    }
}

impl Default for Addrs {
    fn default() -> Self {
        Self::None
    }
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
pub struct Connect {
    pub(crate) uri: Uri,
    pub(crate) port: u16,
    pub(crate) addr: Addrs,
    pub(crate) local_addr: Option<IpAddr>,
}

impl Connect {
    /// Create `Connect` instance by splitting the string by ':' and convert the second part to u16
    pub fn new(uri: Uri) -> Self {
        let (_, port) = parse_host(uri.hostname());

        Self {
            uri,
            port: port.unwrap_or(0),
            addr: Addrs::None,
            local_addr: None,
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

    /// Set local_addr of connect.
    pub fn set_local_addr(&mut self, addr: impl Into<IpAddr>) {
        self.local_addr = Some(addr.into());
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

    /// Take resolved request addresses.
    pub fn take_addrs(&mut self) -> AddrsIter<'static> {
        match mem::take(&mut self.addr) {
            Addrs::None => AddrsIter::None,
            Addrs::One(addr) => AddrsIter::One(addr),
            Addrs::Multi(addrs) => AddrsIter::MultiOwned(addrs.into_iter()),
        }
    }
}

impl fmt::Display for Connect {
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
    MultiOwned(vec_deque::IntoIter<SocketAddr>),
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
            Self::MultiOwned(ref mut iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match *self {
            Self::None => (0, Some(0)),
            Self::One(_) => (1, Some(1)),
            Self::Multi(ref iter) => iter.size_hint(),
            Self::MultiOwned(ref iter) => iter.size_hint(),
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
    use std::net::Ipv4Addr;

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

        let mut iter = AddrsIter::MultiOwned(addrs.into_iter());
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

    #[test]
    fn test_local_addr() {
        let mut conn = Connect::new(Uri::Tcp(http::Uri::from_static("https://example.com")));
        conn.set_local_addr([127, 0, 0, 1]);
        assert_eq!(conn.local_addr.unwrap(), IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
    }
}
