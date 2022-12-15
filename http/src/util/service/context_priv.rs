use std::{future::Future, marker::PhantomData};

use xitca_service::{pipeline::PipelineE, ready::ReadyService, Service};

use crate::http::{BorrowReq, BorrowReqMut};

/// ServiceFactory type for constructing compile time checked stateful service.
///
/// State is roughly doing the same thing as `move ||` style closure capture. The difference comes
/// down to:
///
/// - The captured state is constructed lazily when [Service::call] method is
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
/// # use xitca_service::{fn_service, Service};
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
///    .call(())
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
        ContextBuilder<CF, C, SF1>: Service<Req>,
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

/// Error type for [ContextBuilder] and it's service type.
pub type ContextError<A, B> = PipelineE<A, B>;

impl<CF, Fut, C, CErr, F, Arg> Service<Arg> for ContextBuilder<CF, C, F>
where
    CF: Fn() -> Fut,
    Fut: Future<Output = Result<C, CErr>>,
    C: 'static,
    F: Service<Arg>,
{
    type Response = ContextService<C, F::Response>;
    type Error = ContextError<CErr, F::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s>(&'s self, arg: Arg) -> Self::Future<'s>
    where
        Arg: 's,
    {
        async {
            let state = (self.ctx_factory)().await.map_err(ContextError::First)?;
            let service = self.service_factory.call(arg).await.map_err(ContextError::Second)?;
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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
        self.service.call(Context {
            req,
            state: &self.state,
        })
    }
}

impl<C, S> ReadyService for ContextService<C, S>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}

pub mod object {
    use super::*;

    use std::{boxed::Box, marker::PhantomData};

    use xitca_service::{
        object::{Object, ObjectConstructor, ServiceObject, Wrapper},
        Service,
    };

    pub struct ContextObjectConstructor<Req, C>(PhantomData<(Req, C)>);

    pub type ContextServiceAlias<Req, C, Res, Err> =
        impl for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err>;

    impl<C, I, Svc, BErr, Req, Res, Err> ObjectConstructor<I> for ContextObjectConstructor<Req, C>
    where
        C: 'static,
        Req: 'static,
        I: Service<Response = Svc, Error = BErr> + 'static,
        Svc: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err> + 'static,
    {
        type Object = Object<(), ContextServiceAlias<Req, C, Res, Err>, BErr>;

        fn into_object(inner: I) -> Self::Object {
            struct ContextObjBuilder<I, Req, C>(I, PhantomData<(Req, C)>);

            impl<C, I, Svc, BErr, Req, Res, Err> Service for ContextObjBuilder<I, Req, C>
            where
                I: Service<Response = Svc, Error = BErr>,
                Svc: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err> + 'static,
            {
                type Response =
                    Wrapper<Box<dyn for<'c> ServiceObject<Context<'c, Req, C>, Response = Res, Error = Err>>>;
                type Error = BErr;
                type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f;

                fn call<'s>(&'s self, arg: ()) -> Self::Future<'s>
                where
                    (): 's,
                {
                    async move {
                        let service = self.0.call(arg).await?;
                        Ok(Wrapper(Box::new(service) as _))
                    }
                }
            }

            Object::from_service(ContextObjBuilder(inner, PhantomData))
        }
    }
}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use xitca_service::{fn_service, ServiceExt};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        http::{Request, RequestExt, Response},
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

    #[test]
    fn test_state_and_then() {
        let res = ContextBuilder::new(|| async { Ok::<_, Infallible>(String::from("string_state")) })
            .service(fn_service(into_context).and_then(fn_service(ctx_handler)))
            .call(())
            .now_or_panic()
            .ok()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }

    async fn handler(req: Context<'_, Request<RequestExt<()>>, String>) -> Result<Response<()>, Infallible> {
        let (_, state) = req.into_parts();
        assert_eq!(state, "string_state");
        Ok(Response::new(()))
    }

    #[test]
    fn test_state_in_router() {
        async fn enclosed<S, Req, C, Res, Err>(service: &S, req: Context<'_, Req, C>) -> Result<Res, Err>
        where
            S: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err>,
        {
            service.call(req).await
        }

        let router = GenericRouter::with_custom_object::<object::ContextObjectConstructor<_, _>>()
            .insert("/", get(fn_service(handler)))
            .enclosed_fn(enclosed);

        let res = ContextBuilder::new(|| async { Ok::<_, Infallible>(String::from("string_state")) })
            .service(router)
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
