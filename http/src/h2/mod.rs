//! http/2 specific module for types and protocol utilities.

mod body;
mod builder;
mod error;
mod proto;
mod service;
mod util;

pub(crate) mod dispatcher;

pub use self::body::RequestBody;
pub use self::error::Error;
pub use self::service::H2Service;

use crate::http::header::{CONNECTION, HeaderMap, HeaderName, TE, TRANSFER_ENCODING, UPGRADE};

const CONNECTION_HEADERS: [HeaderName; 4] = [
    HeaderName::from_static("keep-alive"),
    HeaderName::from_static("proxy-connection"),
    TRANSFER_ENCODING,
    UPGRADE,
];

pub fn strip_connection_headers<const IS_REQ: bool>(headers: &mut HeaderMap) {
    for header in &CONNECTION_HEADERS {
        headers.remove(header);
    }

    if IS_REQ {
        if headers.get(TE).is_some_and(|te_header| te_header != "trailers") {
            headers.remove(TE);
        }
    } else {
        headers.remove(TE);
    }

    if let Some(header) = headers.remove(CONNECTION) {
        // A `Connection` header may have a comma-separated list of names of other headers that
        // are meant for only this specific connection.
        //
        // Iterate these names and remove them as headers. Connection-specific headers are
        // forbidden in HTTP2, as that information has been moved into frame types of the h2
        // protocol.
        if let Ok(header_contents) = header.to_str() {
            for name in header_contents.split(',') {
                let name = name.trim();
                headers.remove(name);
            }
        }
    }
}
