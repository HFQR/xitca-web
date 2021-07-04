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

macro_rules! tuple_from_req ({$(($n:tt, $T:ident)),+} => {
    impl<'a, State, $($T),+> FromRequest<'a, State> for ($($T,)+)
    where
        State: 'static,
        $($T: FromRequest<'a, State> + 'a),+
    {
        type Config = ();
        type Error = ();
        type Future = impl Future<Output = Result<Self, Self::Error>> + 'a;

        fn from_request(req: &'a WebRequest<'_, State>) -> Self::Future {
            let mut items = <($(Option<$T>,)+)>::default();

            async move {
                 $(
                    let res = $T::from_request(req).await.map_err(|_| ())?;
                    items.$n = Some(res);
                )+

                Ok(($(items.$n.take().unwrap(),)+))
            }
        }
    }
});

tuple_from_req!((0, A));
tuple_from_req!((0, A), (1, B));
tuple_from_req!((0, A), (1, B), (2, C));
tuple_from_req!((0, A), (1, B), (2, C), (3, D));
tuple_from_req!((0, A), (1, B), (2, C), (3, D), (4, E));
tuple_from_req!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
tuple_from_req!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
tuple_from_req!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
tuple_from_req!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
