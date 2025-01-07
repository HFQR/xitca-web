use xitca_web::http::uri::Scheme;
use xitca_web::http::{header, HeaderMap, HeaderName};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardedFor {
    by: String,
    strategy: ForwardedHeaderStrategy,
    override_proto: Option<Scheme>,
}

pub const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
pub const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
pub const X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
pub const X_FORWARDED_BY: HeaderName = HeaderName::from_static("x-forwarded-by");

impl Default for ForwardedFor {
    fn default() -> Self {
        Self {
            by: "xitca-proxy".to_string(),
            strategy: ForwardedHeaderStrategy::Auto,
            override_proto: None,
        }
    }
}

impl ForwardedFor {
    pub fn new_none() -> Self {
        Self {
            by: "".to_string(),
            strategy: ForwardedHeaderStrategy::None,
            override_proto: None,
        }
    }

    pub fn new_auto(by: &str, override_proto: Option<Scheme>) -> Self {
        Self {
            by: by.to_string(),
            strategy: ForwardedHeaderStrategy::Auto,
            override_proto,
        }
    }

    pub fn new_legacy(by: &str, override_proto: Option<Scheme>) -> Self {
        Self {
            by: by.to_string(),
            strategy: ForwardedHeaderStrategy::Legacy,
            override_proto,
        }
    }

    pub fn new_rfc7239(by: &str, override_proto: Option<Scheme>) -> Self {
        Self {
            by: by.to_string(),
            strategy: ForwardedHeaderStrategy::RFC7239,
            override_proto,
        }
    }

    pub(crate) fn apply(&self, headers: &mut HeaderMap, client_ip: &str, host: String, proto: Scheme) {
        let proto = self.override_proto.clone().unwrap_or(proto);

        self.strategy.apply(headers, self.by.as_str(), client_ip, host, proto);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ForwardedHeaderStrategy {
    None,
    Auto,
    Legacy,
    RFC7239,
}

impl ForwardedHeaderStrategy {
    pub fn apply(&self, headers: &mut HeaderMap, by: &str, client_ip: &str, host: String, proto: Scheme) {
        match self {
            Self::None => (),
            Self::Legacy => Self::apply_legacy(headers, by, client_ip, host, proto),
            Self::RFC7239 => Self::apply_forwarded(headers, by, client_ip, host, proto),
            Self::Auto => {
                if headers.contains_key(X_FORWARDED_FOR) {
                    if headers.contains_key(X_FORWARDED_PROTO)
                        || headers.contains_key(X_FORWARDED_HOST)
                        || headers.contains_key("x-forwarded-by")
                    {
                        // Cannot transition use legacy headers
                        Self::apply_legacy(headers, by, client_ip, host, proto);

                        return;
                    }

                    // Transition to RFC7239
                    // Transform x forwarded for to Forwarded
                    let mut forwarded_for_value = "".to_string();

                    for value in headers.get_all(X_FORWARDED_FOR).iter() {
                        if forwarded_for_value.is_empty() {
                            forwarded_for_value = format!("for={}", value.to_str().unwrap());
                        } else {
                            forwarded_for_value = format!("{}, for={}", forwarded_for_value, value.to_str().unwrap());
                        }
                    }

                    headers.insert(header::FORWARDED, forwarded_for_value.parse().unwrap());
                    headers.remove(X_FORWARDED_FOR);
                }

                // Apply forwarded
                Self::apply_forwarded(headers, by, client_ip, host, proto);
            }
        }
    }

    fn apply_legacy(headers: &mut HeaderMap, by: &str, client_ip: &str, host: String, proto: Scheme) {
        let by_value = match headers.get("x-forwarded-by") {
            Some(value) => format!("{}, {}", value.to_str().unwrap(), by),
            None => by.to_string(),
        };

        let mut forwarded_for_existing_value = headers
            .get_all(X_FORWARDED_FOR)
            .iter()
            .map(|value| value.to_str().unwrap().to_string())
            .collect::<Vec<String>>()
            .join(", ");

        if !forwarded_for_existing_value.is_empty() {
            forwarded_for_existing_value.push_str(", ");
            forwarded_for_existing_value.push_str(client_ip);

            headers.insert(X_FORWARDED_FOR, forwarded_for_existing_value.parse().unwrap());
        } else {
            headers.insert(X_FORWARDED_FOR, client_ip.parse().unwrap());
        }

        headers.insert(X_FORWARDED_BY, by_value.parse().unwrap());

        if !headers.contains_key(X_FORWARDED_PROTO) {
            let proto_value = match headers.get(X_FORWARDED_PROTO) {
                Some(value) => format!("{}, {}", value.to_str().unwrap(), proto.as_str()),
                None => proto.to_string(),
            };

            headers.insert(X_FORWARDED_PROTO, proto_value.parse().unwrap());
        }

        if !headers.contains_key(X_FORWARDED_HOST) {
            headers.insert(X_FORWARDED_HOST, host.parse().unwrap());
        }
    }

    fn apply_forwarded(headers: &mut HeaderMap, by: &str, client_ip: &str, host: String, proto: Scheme) {
        let forwarded_for_value = format!("for={client_ip};by={by};host={host};proto={proto}");

        if headers.contains_key(header::FORWARDED) {
            let forwarded_existing_value = headers
                .get_all(header::FORWARDED)
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<&str>>()
                .join(", ");
            let forwarded_value = format!("{forwarded_existing_value}, {forwarded_for_value}");

            headers.insert(header::FORWARDED, forwarded_value.parse().unwrap());
        } else {
            headers.insert(header::FORWARDED, forwarded_for_value.parse().unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forward_legacy() {
        let mut headers = HeaderMap::new();

        headers.insert(X_FORWARDED_FOR, "127.0.0.1".parse().unwrap());
        headers.insert(X_FORWARDED_PROTO, "https".parse().unwrap());
        headers.insert(X_FORWARDED_HOST, "localhost".parse().unwrap());

        let strategy = ForwardedHeaderStrategy::Legacy;
        strategy.apply(&mut headers, "by", "192.168.0.1", "test.com".to_string(), Scheme::HTTP);

        assert_eq!(
            headers.get(X_FORWARDED_FOR).unwrap().to_str().unwrap(),
            "127.0.0.1, 192.168.0.1"
        );
        assert_eq!(headers.get(X_FORWARDED_PROTO).unwrap().to_str().unwrap(), "https");
        assert_eq!(headers.get(X_FORWARDED_HOST).unwrap().to_str().unwrap(), "localhost");
    }

    #[test]
    fn test_forward_none() {
        let mut headers = HeaderMap::new();

        let strategy = ForwardedHeaderStrategy::None;
        strategy.apply(&mut headers, "by", "192.168.0.1", "test.com".to_string(), Scheme::HTTP);

        assert_eq!(headers.get(header::FORWARDED), None);
        assert_eq!(headers.get(X_FORWARDED_FOR), None);
        assert_eq!(headers.get(X_FORWARDED_PROTO), None);
        assert_eq!(headers.get(X_FORWARDED_HOST), None);
    }

    #[test]
    fn test_forward_rfc7239() {
        let mut headers = HeaderMap::new();

        let strategy = ForwardedHeaderStrategy::RFC7239;
        strategy.apply(&mut headers, "by", "192.168.0.1", "test.com".to_string(), Scheme::HTTP);

        assert_eq!(
            headers.get(header::FORWARDED).unwrap().to_str().unwrap(),
            "for=192.168.0.1;by=by;host=test.com;proto=http"
        );
    }

    #[test]
    fn test_forward_legacy_multiple() {
        let mut headers = HeaderMap::new();

        headers.append(X_FORWARDED_FOR, "127.0.0.1".parse().unwrap());
        headers.append(X_FORWARDED_FOR, "127.0.0.2".parse().unwrap());
        headers.append(X_FORWARDED_PROTO, "https".parse().unwrap());
        headers.append(X_FORWARDED_HOST, "localhost".parse().unwrap());

        let strategy = ForwardedHeaderStrategy::Legacy;
        strategy.apply(&mut headers, "by", "192.168.0.1", "test.com".to_string(), Scheme::HTTP);

        assert_eq!(
            headers.get(X_FORWARDED_FOR).unwrap().to_str().unwrap(),
            "127.0.0.1, 127.0.0.2, 192.168.0.1"
        );
        assert_eq!(headers.get(X_FORWARDED_PROTO).unwrap().to_str().unwrap(), "https");
        assert_eq!(headers.get(X_FORWARDED_HOST).unwrap().to_str().unwrap(), "localhost");
    }

    #[test]
    fn test_forward_legacy_no_proto() {
        let mut headers = HeaderMap::new();

        headers.insert(X_FORWARDED_FOR, "127.0.0.1".parse().unwrap());
        headers.insert(X_FORWARDED_HOST, "localhost".parse().unwrap());

        let strategy = ForwardedHeaderStrategy::Legacy;
        strategy.apply(&mut headers, "by", "192.168.0.1", "test.com".to_string(), Scheme::HTTP);

        assert_eq!(
            headers.get(X_FORWARDED_FOR).unwrap().to_str().unwrap(),
            "127.0.0.1, 192.168.0.1"
        );
        assert_eq!(headers.get(X_FORWARDED_PROTO).unwrap().to_str().unwrap(), "http");
        assert_eq!(headers.get(X_FORWARDED_HOST).unwrap().to_str().unwrap(), "localhost");
        assert!(!headers.contains_key(header::FORWARDED));
    }

    #[test]
    fn test_forward_legacy_no_host() {
        let mut headers = HeaderMap::new();

        headers.insert(X_FORWARDED_FOR, "127.0.0.1".parse().unwrap());

        let strategy = ForwardedHeaderStrategy::Legacy;
        strategy.apply(&mut headers, "by", "192.168.0.1", "test.com".to_string(), Scheme::HTTP);

        assert_eq!(
            headers.get(X_FORWARDED_FOR).unwrap().to_str().unwrap(),
            "127.0.0.1, 192.168.0.1"
        );
        assert_eq!(headers.get(X_FORWARDED_PROTO).unwrap().to_str().unwrap(), "http");
        assert_eq!(headers.get(X_FORWARDED_HOST).unwrap().to_str().unwrap(), "test.com");
        assert!(!headers.contains_key(header::FORWARDED));
    }

    #[test]
    fn test_forward_auto_forwarded() {
        let mut headers = HeaderMap::new();

        let strategy = ForwardedHeaderStrategy::Auto;
        strategy.apply(&mut headers, "by", "192.168.0.1", "test.com".to_string(), Scheme::HTTP);

        assert!(headers.contains_key(header::FORWARDED));
        assert_eq!(
            headers.get(header::FORWARDED).unwrap().to_str().unwrap(),
            "for=192.168.0.1;by=by;host=test.com;proto=http"
        );
    }

    #[test]
    fn test_forward_auto_forwarded_transition() {
        let mut headers = HeaderMap::new();

        headers.insert(X_FORWARDED_FOR, "127.0.0.1".parse().unwrap());

        let strategy = ForwardedHeaderStrategy::Auto;
        strategy.apply(&mut headers, "by", "192.168.0.1", "test.com".to_string(), Scheme::HTTP);

        assert!(headers.contains_key(header::FORWARDED));
        assert_eq!(
            headers.get(header::FORWARDED).unwrap().to_str().unwrap(),
            "for=127.0.0.1, for=192.168.0.1;by=by;host=test.com;proto=http"
        );
    }

    #[test]
    fn test_forward_auto_legacy() {
        let mut headers = HeaderMap::new();

        headers.insert(X_FORWARDED_FOR, "127.0.0.1".parse().unwrap());
        headers.insert(X_FORWARDED_PROTO, "https".parse().unwrap());
        headers.insert(X_FORWARDED_HOST, "localhost".parse().unwrap());

        let strategy = ForwardedHeaderStrategy::Auto;
        strategy.apply(&mut headers, "by", "192.168.0.1", "test.com".to_string(), Scheme::HTTP);

        assert_eq!(
            headers.get(X_FORWARDED_FOR).unwrap().to_str().unwrap(),
            "127.0.0.1, 192.168.0.1"
        );
        assert_eq!(headers.get(X_FORWARDED_PROTO).unwrap().to_str().unwrap(), "https");
        assert_eq!(headers.get(X_FORWARDED_HOST).unwrap().to_str().unwrap(), "localhost");
        assert!(!headers.contains_key(header::FORWARDED));
    }
}
