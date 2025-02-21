//! middleware for adding typed state to service request.

use core::{fmt, future::Future};

use xitca_service::Service;

use crate::http::{BorrowReq, BorrowReqMut};

/// ServiceFactory type for constructing compile time checked stateful service.
///
/// State is roughly doing the same thing as `move ||` style closure capture. The difference comes
/// down to:
///
/// - The captured state is constructed lazily when [Service::call] method is called.
/// - State can be referenced in nested types and beyond closures.
///
/// # Example:
///```rust
/// # use std::convert::Infallible;
/// # use xitca_http::util::middleware::context::{ContextBuilder, Context};
/// # use xitca_service::{fn_service, Service, ServiceExt};
///
/// // function service.
/// async fn state_handler(req: Context<'_, String, String>) -> Result<String, Infallible> {
///    let (parent_req, state) = req.into_parts();
///    assert_eq!(state, "string_state");
///    Ok(String::from("string_response"))
/// }
///
/// # async fn stateful() {
/// // Construct Stateful service builder with closure.
/// let service = fn_service(state_handler)
///     // Stateful service builder would construct given service builder and pass (&State, Req) to it's
///     // Service::call method.
///     .enclosed(ContextBuilder::new(|| async { Ok::<_, Infallible>(String::from("string_state")) }))
///     .call(())
///     .await
///     .unwrap();
///
/// let req = String::default();
/// let res = service.call(req).await.unwrap();
/// assert_eq!(res, "string_response");
///
/// # }
///```
pub struct ContextBuilder<CF> {
    builder: CF,
}

impl<CF, Fut, C, CErr> ContextBuilder<CF>
where
    CF: Fn() -> Fut,
    Fut: Future<Output = Result<C, CErr>>,
{
    /// Make a stateful service factory with given future.
    pub fn new(builder: CF) -> Self {
        Self { builder }
    }
}

/// Specialized Request type State service factory.
///
/// This type enables borrow parent service request type as &Req and &mut Req
pub struct Context<'a, Req, C> {
    req: Req,
    state: &'a C,
}

impl<'a, Req, C> Context<'a, Req, C> {
    /// Destruct request into a tuple of (&state, parent_request).
    #[inline]
    pub fn into_parts(self) -> (Req, &'a C) {
        (self.req, self.state)
    }
}

// impls to forward trait from Req type.
// BorrowReq/Mut are traits needed for nesting Router/Route service inside Context service.
impl<T, Req, C> BorrowReq<T> for Context<'_, Req, C>
where
    Req: BorrowReq<T>,
{
    #[inline]
    fn borrow(&self) -> &T {
        self.req.borrow()
    }
}

impl<T, Req, C> BorrowReqMut<T> for Context<'_, Req, C>
where
    Req: BorrowReqMut<T>,
{
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self.req.borrow_mut()
    }
}

type Error = Box<dyn fmt::Debug>;

impl<CF, Fut, C, CErr, S, E> Service<Result<S, E>> for ContextBuilder<CF>
where
    CF: Fn() -> Fut,
    Fut: Future<Output = Result<C, CErr>>,
    C: 'static,
    CErr: fmt::Debug + 'static,
    E: fmt::Debug + 'static,
{
    type Response = service::ContextService<C, S>;
    type Error = Error;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        let service = res.map_err(|e| Box::new(e) as Error)?;
        let state = (self.builder)().await.map_err(|e| Box::new(e) as Error)?;
        Ok(service::ContextService { service, state })
    }
}

mod service {
    use xitca_service::ready::ReadyService;

    use super::*;

    pub struct ContextService<C, S> {
        pub(super) state: C,
        pub(super) service: S,
    }

    impl<Req, C, S, Res, Err> Service<Req> for ContextService<C, S>
    where
        S: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err>,
    {
        type Response = Res;
        type Error = Err;

        #[inline]
        async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
            self.service
                .call(Context {
                    req,
                    state: &self.state,
                })
                .await
        }
    }

    impl<C, S> ReadyService for ContextService<C, S>
    where
        S: ReadyService,
    {
        type Ready = S::Ready;

        #[inline]
        async fn ready(&self) -> Self::Ready {
            self.service.ready().await
        }
    }
}

#[cfg(feature = "router")]
mod router_impl {
    use xitca_service::object::ServiceObject;

    use crate::util::service::router::{IntoObject, PathGen, RouteGen, RouteObject};

    use super::*;

    pub type ContextObject<Req, C, Res, Err> =
        Box<dyn for<'c> ServiceObject<Context<'c, Req, C>, Response = Res, Error = Err>>;

    impl<C, I, Arg, Req, Res, Err> IntoObject<I, Arg> for Context<'_, Req, C>
    where
        C: 'static,
        Req: 'static,
        I: Service<Arg> + RouteGen + Send + Sync + 'static,
        I::Response: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err> + 'static,
    {
        type Object = RouteObject<Arg, ContextObject<Req, C, Res, Err>, I::Error>;

        fn into_object(inner: I) -> Self::Object {
            struct Builder<I, Req, C>(I, core::marker::PhantomData<fn(Req, C)>);

            impl<I, Req, C> PathGen for Builder<I, Req, C>
            where
                I: PathGen,
            {
                fn path_gen(&mut self, prefix: &str) -> String {
                    self.0.path_gen(prefix)
                }
            }

            impl<I, Req, C> RouteGen for Builder<I, Req, C>
            where
                I: RouteGen,
            {
                type Route<R> = I::Route<R>;

                fn route_gen<R>(route: R) -> Self::Route<R> {
                    I::route_gen(route)
                }
            }

            impl<C, I, Arg, Req, Res, Err> Service<Arg> for Builder<I, Req, C>
            where
                I: Service<Arg> + RouteGen,
                I::Response: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err> + 'static,
            {
                type Response = ContextObject<Req, C, Res, Err>;
                type Error = I::Error;

                async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
                    self.0.call(arg).await.map(|s| Box::new(s) as _)
                }
            }

            RouteObject(Box::new(Builder(inner, core::marker::PhantomData)))
        }
    }
}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use xitca_service::{ServiceExt, fn_service};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::http::{Request, Response};

    use super::*;

    struct Context2<'a, ST> {
        req: Request<()>,
        state: &'a ST,
    }

    async fn into_context(req: Context<'_, Request<()>, String>) -> Result<Context2<'_, String>, Infallible> {
        let (req, state) = req.into_parts();
        assert_eq!(state, "string_state");
        Ok(Context2 { req, state })
    }

    async fn ctx_handler(ctx: Context2<'_, String>) -> Result<Response<()>, Infallible> {
        assert_eq!(ctx.state, "string_state");
        assert_eq!(ctx.req.method().as_str(), "GET");
        Ok(Response::new(()))
    }

    #[test]
    fn test_state_and_then() {
        let res = fn_service(into_context)
            .and_then(fn_service(ctx_handler))
            .enclosed(ContextBuilder::new(|| async {
                Ok::<_, Infallible>(String::from("string_state"))
            }))
            .call(())
            .now_or_panic()
            .ok()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }

    #[cfg(feature = "router")]
    #[test]
    fn test_state_in_router() {
        use crate::{
            http::RequestExt,
            util::service::{Router, route::get},
        };

        async fn handler(req: Context<'_, Request<RequestExt<()>>, String>) -> Result<Response<()>, Infallible> {
            let (_, state) = req.into_parts();
            assert_eq!(state, "string_state");
            Ok(Response::new(()))
        }

        async fn enclosed<S, Req, C, Res, Err>(service: &S, req: Context<'_, Req, C>) -> Result<Res, Err>
        where
            S: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err>,
        {
            service.call(req).await
        }

        let router = || Router::new().insert("/", get(fn_service(handler)));

        let router_with_ctx = || {
            router().enclosed_fn(enclosed).enclosed(ContextBuilder::new(|| async {
                Ok::<_, Infallible>(String::from("string_state"))
            }))
        };

        fn bound_check<T: Send + Sync>(_: T) {}

        bound_check(router_with_ctx());

        let res = router_with_ctx()
            .call(())
            .now_or_panic()
            .ok()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status().as_u16(), 200);

        let res = router()
            .insert("/nest", router())
            .enclosed(ContextBuilder::new(|| async {
                Ok::<_, Infallible>(String::from("string_state"))
            }))
            .call(())
            .now_or_panic()
            .ok()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }
}
