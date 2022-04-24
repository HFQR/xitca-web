mod body;
mod extension;
mod path;
mod request;
mod state;
mod uri;

pub mod header;

#[cfg(feature = "json")]
mod json;

pub use self::body::Body;
pub use self::path::PathRef;
pub use self::request::RequestRef;
pub use self::state::StateRef;
pub use self::uri::UriRef;

#[cfg(feature = "json")]
pub use self::json::Json;

use std::{convert::Infallible, future::Future};

use xitca_http::util::service::FromRequest;

use crate::request::WebRequest;

pub enum ExtractError {}

impl From<Infallible> for ExtractError {
    fn from(_: Infallible) -> Self {
        unreachable!()
    }
}

impl<'a, 'r, 's, S> FromRequest<'a, &'r mut WebRequest<'s, S>> for &'a WebRequest<'a, S>
where
    S: 'static,
{
    type Type<'b> = &'b WebRequest<'b, S>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(&**req) }
    }
}
