use core::{
    convert::Infallible,
    future::{ready, Future, Ready},
};

use super::Service;

/// Shortcut for transform a given Fn into type impl [Service] trait.
pub fn fn_build<F, Arg, Fut, Svc, Err>(f: F) -> FnService<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
{
    FnService(f)
}

/// Shortcut for infering generic type param of [Service] trait implements.
/// Often used to help type infering when nesting [ServiceExt::enclosed] or
/// [ServiceExt::enclosed_fn]
///
/// # Examples
/// ```rust
/// # use core::convert::Infallible;
/// # use xitca_service::{fn_build_nop, fn_service, Service, ServiceExt};
///
/// // a simple passthrough middleware function.
/// async fn mw<S, Req>(svc: &S, req: Req) -> Result<S::Response, S::Error>
/// where
///     S: Service<Req>
/// {
///     svc.call(req).await
/// }
///
/// # fn nest() {
/// fn_service(|_: ()| async { Ok::<_, Infallible>(()) })
///     .enclosed(
///         fn_build_nop() // use nop build service for nested middleware function.
///             .enclosed_fn(mw)
///             .enclosed_fn(mw)
///     );
/// # }
/// ```
///
/// [ServiceExt::enclosed]: super::ServiceExt::enclosed
/// [ServiceExt::enclosed_fn]: super::ServiceExt::enclosed_fn
pub fn fn_build_nop<Arg>() -> FnService<impl Fn(Arg) -> Ready<Result<Arg, Infallible>>> {
    fn_build(|arg| ready(Ok(arg)))
}

/// Shortcut for transform a given Fn into type impl [Service] trait.
pub fn fn_service<F, Req, Fut, Res, Err>(f: F) -> FnService<impl Fn(()) -> Ready<Result<FnService<F>, Infallible>>>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn_build(move |_| ready(Ok(FnService(f.clone()))))
}

/// new type for implementing [Service] trait to a `Fn(Req) -> impl Future<Output = Result<Res, Err>>` type.
#[derive(Clone)]
pub struct FnService<F>(F);

impl<F, Req, Fut, Res, Err> Service<Req> for FnService<F>
where
    F: Fn(Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;

    #[inline]
    fn call(&self, req: Req) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        (self.0)(req)
    }
}
