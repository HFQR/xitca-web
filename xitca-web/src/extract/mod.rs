mod state;

pub use self::state::State;

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

    fn from_request(req: &'a WebRequest<'_, D>) -> Self::Future {
        async move { Ok(req) }
    }
}

macro_rules! tuple_from_req ({$($T:ident),+} => {
    impl<'a, State, Err, $($T),+> FromRequest<'a, State> for ($($T,)+)
    where
        State: 'static,
        Err: 'static,
        $($T: FromRequest<'a, State, Error = Err> + 'a),+
    {
        type Config = ();
        type Error = Err;
        type Future = impl Future<Output = Result<Self, Self::Error>> + 'a;

        fn from_request(req: &'a WebRequest<'_, State>) -> Self::Future {
            async move {
                Ok(($($T::from_request(req).await?,)+))
            }
        }
    }
});

tuple_from_req!(A);
tuple_from_req!(A, B);
tuple_from_req!(A, B, C);
tuple_from_req!(A, B, C, D);
tuple_from_req!(A, B, C, D, E);
tuple_from_req!(A, B, C, D, E, F);
tuple_from_req!(A, B, C, D, E, F, G);
tuple_from_req!(A, B, C, D, E, F, G, H);
tuple_from_req!(A, B, C, D, E, F, G, H, I);
