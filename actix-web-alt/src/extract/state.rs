use std::{future::Future, ops::Deref};

use crate::request::WebRequest;

use super::FromRequest;

/// App state extractor.
/// S type must be the same with the type passed to App::with_xxx_state(<S>).
pub struct State<'a, S>(&'a S);

impl<S> Deref for State<'_, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, S> FromRequest<'a, S> for State<'a, S>
where
    S: Clone + 'static,
{
    type Config = ();
    type Error = ();

    type Future = impl Future<Output = Result<Self, Self::Error>> + 'a;

    fn from_request(req: &'a WebRequest<'_, S>) -> Self::Future {
        async move { Ok(State(req.state)) }
    }
}
