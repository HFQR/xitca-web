use std::future::Future;

use crate::request::WebRequest;

use super::FromRequest;

/// App state extractor.
/// S type must be the same with the type passed to App::with_xxx_state(<S>).
pub struct State<S>(S);

impl<'a, S> FromRequest<'a, S> for State<S>
where
    S: Clone + 'static,
{
    type Config = ();
    type Error = ();

    type Future = impl Future<Output = Result<Self, Self::Error>> + 'a;

    fn from_request(req: &'a WebRequest<'_, S>) -> Self::Future {
        let state = req.state.clone();
        async move { Ok(State(state)) }
    }
}
