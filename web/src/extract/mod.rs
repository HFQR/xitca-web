mod body;
mod extension;
mod header;
mod path;
mod request;
mod state;
mod uri;

#[cfg(feature = "json")]
mod json;

pub use self::body::Body;
pub use self::header::{HeaderName, HeaderRef};
pub use self::path::PathRef;
pub use self::request::RequestRef;
pub use self::state::StateRef;
pub use self::uri::UriRef;

#[cfg(feature = "json")]
pub use self::json::Json;

use std::convert::Infallible;

pub enum ExtractError {}

impl From<Infallible> for ExtractError {
    fn from(_: Infallible) -> Self {
        unreachable!()
    }
}
