use std::{future::Future, marker::PhantomData};

use xitca_service::{pipeline::PipelineE, ready::ReadyService, BuildService, Service};

use crate::request::{BorrowReq, BorrowReqMut};

/// ServiceFactory type for constructing compile time checked stateful service.
///
/// State is roughly doing the same thing as `move ||` style closure capture. The difference comes
/// down to:
///
/// - The captured state is constructed lazily when [BuildService::build] method is
/// called.
///
/// - State can be referenced in nested types and beyond closures.
/// .eg:
/// ```rust(no_run)
/// fn_service(|req: &String| async { Ok(req) }).and_then(|_: &String| ..)
/// ```
///
/// # Example:
///
///```rust
/// # use std::convert::Infallible;
/// # use xitca_http::util::service::context::{ContextBuilder, Context};
/// # use xitca_service::{fn_service, Service, BuildService};
///
/// // function service.
/// async fn state_handler(req: Context<'_, String, String>) -> Result<String, Infallible> {
///    let (parent_req, state) = req.into_parts();
///    assert_eq!(state, "string_state");
///    Ok(String::from("string_response"))
/// }
///
/// # async fn stateful() {
/// // Construct Stateful service factory with closure.
/// let service = ContextBuilder::new(|| async { Ok::<_, Infallible>(String::from("string_state")) })
///    // Stateful service factory would construct given service factory and pass (&State, Req) to it.
///    .service(fn_service(state_handler))
///    .build(())
///    .await
///    .unwrap();
///
/// let req = String::default();
/// let res = service.call(req).await.unwrap();
/// assert_eq!(res, "string_response");
///
/// # }
///```
///
pub struct ContextBuilder<CF, C, SF = ()> {
    ctx_factory: CF,
    service_factory: SF,
    _ctx: PhantomData<C>,
}

impl<CF, Fut, C, CErr> ContextBuilder<CF, C>
where
    CF: Fn() -> Fut,
    Fut: Future<Output = Result<C, CErr>>,
{
    /// Make a stateful service factory with given future.
    pub fn new(ctx_factory: CF) -> Self {
        Self {
            ctx_factory,
            service_factory: (),
            _ctx: PhantomData,
        }
    }
}

impl<CF, C, SF> ContextBuilder<CF, C, SF> {
    /// The constructor of service type that would receive state.
    pub fn service<Req, SF1>(self, factory: SF1) -> ContextBuilder<CF, C, SF1>
    where
        ContextBuilder<CF, C, SF1>: BuildService<Req>,
    {
        ContextBuilder {
            ctx_factory: self.ctx_factory,
            service_factory: factory,
            _ctx: PhantomData,
        }
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

impl<Req, C, T> BorrowReq<T> for Context<'_, Req, C>
where
    Req: BorrowReq<T>,
{
    fn borrow(&self) -> &T {
        self.req.borrow()
    }
}

impl<Req, C, T> BorrowReqMut<T> for Context<'_, Req, C>
where
    Req: BorrowReqMut<T>,
{
    fn borrow_mut(&mut self) -> &mut T {
        self.req.borrow_mut()
    }
}

/// Error type for [ContextBuilder] as [ServiceFactory] and [ContextService] as [Service]
pub type ContextError<A, B> = PipelineE<A, B>;

impl<CF, Fut, C, CErr, F, Arg> BuildService<Arg> for ContextBuilder<CF, C, F>
where
    CF: Fn() -> Fut,
    Fut: Future<Output = Result<C, CErr>>,
    C: 'static,
    F: BuildService<Arg>,
    F::Error: From<CErr>,
{
    type Service = ContextService<C, F::Service>;
    type Error = F::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let state = (self.ctx_factory)();
        let service = self.service_factory.build(arg);
        async {
            let state = state.await?;
            let service = service.await?;
            Ok(ContextService { service, state })
        }
    }
}

pub struct ContextService<C, S> {
    state: C,
    service: S,
}

impl<Req, C, S, Res, Err> Service<Req> for ContextService<C, S>
where
    S: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        self.service.call(Context {
            req,
            state: &self.state,
        })
    }
}

impl<Req, C, S, R, Res, Err> ReadyService<Req> for ContextService<C, S>
where
    S: for<'c> ReadyService<Context<'c, Req, C>, Response = Res, Error = Err, Ready = R>,
{
    type Ready = R;
    type ReadyFuture<'f> = impl Future<Output = Self::Ready> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}

pub mod object {
    use super::*;

    use std::{boxed::Box, marker::PhantomData};

    use xitca_service::{
        fn_build,
        object::{
            helpers::{ServiceObject, Wrapper},
            ObjectConstructor,
        },
        BuildService, BuildServiceExt, Service,
    };

    pub struct ContextObjectConstructor<Req, C>(PhantomData<(Req, C)>);

    pub type ContextFactoryObject<Req, C, BErr, Res, Err> =
        impl BuildService<Error = BErr, Service = ContextServiceObject<Req, C, Res, Err>>;

    pub type ContextServiceObject<Req, C, Res, Err> =
        impl for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err>;

    impl<C, I, Svc, BErr, Req, Res, Err> ObjectConstructor<I> for ContextObjectConstructor<Req, C>
    where
        I: BuildService<Service = Svc, Error = BErr> + 'static,
        Svc: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err> + 'static,
    {
        type Object = ContextFactoryObject<Req, C, BErr, Res, Err>;

        fn into_object(inner: I) -> Self::Object {
            let factory = fn_build(move |_arg: ()| {
                let fut = inner.build(());
                async move {
                    let boxed_service = Box::new(Wrapper(fut.await?))
                        as Box<dyn for<'c> ServiceObject<Context<'c, Req, C>, Response = _, Error = _>>;
                    Ok(Wrapper(boxed_service))
                }
            })
            .boxed_future();

            Box::new(factory) as Box<dyn BuildService<Service = _, Error = _, Future = _>>
        }
    }
}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use xitca_service::{fn_service, BuildServiceExt};

    use crate::{
        http::Response,
        request::Request,
        util::service::{route::get, GenericRouter},
    };

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

    #[tokio::test]
    async fn test_state_and_then() {
        let service = ContextBuilder::new(|| async { Ok::<_, Infallible>(String::from("string_state")) })
            .service(fn_service(into_context).and_then(fn_service(ctx_handler)))
            .build(())
            .await
            .ok()
            .unwrap();

        let req = Request::default();

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }

    async fn handler(req: Context<'_, Request<()>, String>) -> Result<Response<()>, Infallible> {
        let (_, state) = req.into_parts();
        assert_eq!(state, "string_state");
        Ok(Response::new(()))
    }

    #[tokio::test]
    async fn test_state_in_router() {
        async fn enclosed<S, Req, C, Res, Err>(service: &S, req: Context<'_, Req, C>) -> Result<Res, Err>
        where
            S: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err>,
        {
            service.call(req).await
        }

        let router = GenericRouter::with_custom_object::<super::object::ContextObjectConstructor<_, _>>()
            .insert("/", get(fn_service(handler)))
            .enclosed_fn(enclosed);

        let service = ContextBuilder::new(|| async { Ok::<_, Infallible>(String::from("string_state")) })
            .service(router)
            .build(())
            .await
            .ok()
            .unwrap();

        let req = Request::default();

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }
}
