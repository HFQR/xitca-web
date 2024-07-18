use core::marker::PhantomData;

use crate::service::Service;

/// middleware builder type for grouping and nesting multiple middlewares.
///
/// # Examples
/// ```rust
/// # use core::convert::Infallible;
/// # use xitca_service::{fn_service, middleware::Group, Service, ServiceExt};
/// // a simple passthrough middleware function.
/// async fn mw<S, Req>(svc: &S, req: Req) -> Result<S::Response, S::Error>
/// where
///     S: Service<Req>
/// {
///     svc.call(req).await
/// }
///
/// fn_service(|_: ()| async { Ok::<_, Infallible>(()) })
///     .enclosed(
///         // group multiple middlewares and apply it to fn_service.
///         Group::new()
///             .enclosed_fn(mw)
///             .enclosed_fn(mw)
///     );
/// ```
pub struct Group<S, E>(PhantomData<fn(S, E)>);

impl<S, E> Clone for Group<S, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S, E> Copy for Group<S, E> {}

impl<S, E> Default for Group<S, E> {
    fn default() -> Self {
        unimplemented!("please use Group::new");
    }
}

impl<S, E> Group<S, E> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<S, E> Service<Result<S, E>> for Group<S, E> {
    type Response = S;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res
    }
}
