use http::uri::Uri;

use crate::error::InvalidUri;

pub(crate) fn try_parse_uri(url: &str) -> Result<Uri, InvalidUri> {
    let uri = Uri::try_from(url)?;

    match (uri.scheme_str(), uri.host()) {
        (_, None) => Err(InvalidUri::MissingHost),
        (None, _) => Err(InvalidUri::MissingScheme),
        (Some("http" | "ws" | "https" | "wss"), _) => Ok(uri),
        (Some(_), _) => Err(InvalidUri::UnknownScheme),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn uri_parse() {
        let err = try_parse_uri("http2://example.com").err().unwrap();
        assert!(matches!(err, InvalidUri::UnknownScheme));

        let err = try_parse_uri("/hello/world").err().unwrap();
        assert!(matches!(err, InvalidUri::MissingHost));

        let err = try_parse_uri("example.com").err().unwrap();
        assert!(matches!(err, InvalidUri::MissingScheme));

        let _ = try_parse_uri("http://example.com").unwrap();
    }
}
