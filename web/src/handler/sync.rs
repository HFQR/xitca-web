#![allow(non_snake_case)]

use core::{
    convert::Infallible,
    future::{Ready, ready},
    marker::PhantomData,
};

use super::{FromRequest, Responder};

use xitca_service::{FnService, Service, fn_build};

/// synchronous version of [handler_service]
///
/// `handler_sync_service` run given function on a separate thread pool from the event loop and async
/// logic of xitca-web itself. Therefore it comes with different requirement of function argument
/// compared to [handler_service] where the arguments must be types that impl [FromRequest] trait,
/// being thread safe with `Send` trait bound and with `'static` lifetime.
///
/// # Examples:
/// ```rust
/// # use xitca_web::{
/// #     handler::{
/// #         {handler_service, handler_sync_service},
/// #         uri::{UriOwn, UriRef},
/// #     },
/// #     App,
/// #     WebContext
/// # };
///
/// App::new()
///     .at("/valid", handler_sync_service(|_: UriOwn| "uri is thread safe and owned value"))
///     // uncomment the line below would result in compile error.
///     // .at("/invalid1", handler_sync_service(|_: UriRef<'_>| { "uri ref is borrowed value" }))
///     // uncomment the line below would result in compile error.
///     // .at("/invalid2", handler_sync_service(|_: &WebContext<'_>| { "web request is borrowed value and not thread safe" }))
///     # .at("/nah", handler_service(|_: &WebContext<'_>| async { "" }));
/// ```
///
/// [handler_service]: super::handler_service
pub fn handler_sync_service<Arg, F, T, O>(func: F) -> FnService<impl Fn(Arg) -> Ready<FnServiceOutput<F, T, O>>>
where
    F: Closure<T> + Send + Clone,
{
    fn_build(move |_| {
        ready(Ok(HandlerServiceSync {
            func: func.clone(),
            _p: PhantomData,
        }))
    })
}

type FnServiceOutput<F, T, O> = Result<HandlerServiceSync<F, T, O>, Infallible>;

pub struct HandlerServiceSync<F, T, O> {
    func: F,
    _p: PhantomData<(T, O)>,
}

impl<F, Req, T, O> Service<Req> for HandlerServiceSync<F, T, O>
where
    // for borrowed extractors, `T` is the `'static` version of the extractors
    T: FromRequest<'static, Req>,
    // just to assist type inference to pinpoint `T`
    F: Closure<T> + Send + Clone + 'static,
    F: for<'a> Closure<T::Type<'a>, Output = O>,
    O: Responder<Req> + Send + 'static,
    for<'a> T::Type<'a>: Send + 'static,
    T::Error: From<O::Error>,
{
    type Response = O::Response;
    type Error = T::Error;

    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        let extract = T::Type::<'_>::from_request(&req).await?;
        let func = self.func.clone();
        let res = tokio::task::spawn_blocking(move || func.call(extract)).await.unwrap();
        res.respond(req).await.map_err(Into::into)
    }
}

#[doc(hidden)]
/// sync version of xitca_service::AsyncClosure trait.
pub trait Closure<Arg> {
    type Output;

    fn call(&self, arg: Arg) -> Self::Output;
}

macro_rules! closure_impl {
    ($($arg: ident),*) => {
        impl<Func, O, $($arg,)*> Closure<($($arg,)*)> for Func
        where
            Func: Fn($($arg),*) -> O,
        {
            type Output = O;

            #[inline]
            fn call(&self, ($($arg,)*): ($($arg,)*)) -> Self::Output {
                self($($arg,)*)
            }
        }
    }
}

closure_impl! {}
closure_impl! { A }
closure_impl! { A, B }
closure_impl! { A, B, C }
closure_impl! { A, B, C, D }
closure_impl! { A, B, C, D, E }
closure_impl! { A, B, C, D, E, F }
closure_impl! { A, B, C, D, E, F, G }
closure_impl! { A, B, C, D, E, F, G, H }
closure_impl! { A, B, C, D, E, F, G, H, I }
