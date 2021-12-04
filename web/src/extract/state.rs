use std::{convert::Infallible, future::Future, ops::Deref};

use xitca_http::util::service::FromRequest;

use crate::request::WebRequest;

/// App state extractor.
/// S type must be the same with the type passed to App::with_xxx_state(<S>).
pub struct State<'a, S>(&'a S);

impl<S> Deref for State<'_, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, 's, S> FromRequest<'a, &'r mut WebRequest<'s, S>> for State<'a, S>
where
    S: 'static,
{
    type Type<'b> = State<'b, S>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(State(req.state())) }
    }
}
