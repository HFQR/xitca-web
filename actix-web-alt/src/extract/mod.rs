use std::future::Future;

use crate::request::WebRequest;

/// Trait implemented by types that can be extracted from request.
///
/// Types that implement this trait can be used with `Route` handlers.
pub trait FromRequest<'a, D>: Sized {
    /// Configuration for this extractor.
    type Config: Default + 'static;

    /// The associated error which can be returned.
    type Error;

    /// Future that resolves to a Self.
    type Future: Future<Output = Result<Self, Self::Error>> + 'a;

    /// Create a Self from request parts asynchronously.
    fn from_request(req: &'a WebRequest<'_, D>) -> Self::Future;
}

impl<'a, D> FromRequest<'a, D> for &'a WebRequest<'a, D>
where
    D: 'static,
{
    type Config = ();
    type Error = ();

    type Future = impl Future<Output = Result<Self, Self::Error>> + 'a;

    fn from_request(req: &'a WebRequest<'a, D>) -> Self::Future {
        async move { Ok(req) }
    }
}

impl<'a, D> FromRequest<'a, D> for State<'a, D>
where
    D: 'static,
{
    type Config = ();
    type Error = ();

    type Future = impl Future<Output = Result<Self, Self::Error>> + 'a;

    fn from_request(req: &'a WebRequest<'a, D>) -> Self::Future {
        async move { Ok(State(req.data)) }
    }
}

struct State<'a, D>(&'a D);
