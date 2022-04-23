use std::{future::Future, marker::PhantomData};

use xitca_service::{ready::ReadyService, Service, ServiceFactory};

use crate::request::{BorrowReq, BorrowReqMut};

/// ServiceFactory type for constructing compile time checked stateful service.
///
/// State is roughly doing the same thing as `move ||` style closure capture. The difference comes
/// down to:
///
/// - The captured state is constructed lazily when [ServiceFactory::new_service] method is
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
/// # use xitca_service::{fn_service, Service, ServiceFactory};
///
/// // function service.
/// async fn state_handler(req: &mut Context<'_, String, String>) -> Result<String, Infallible> {
///    let (parent_req, state) = req.borrow_parts_mut();
///    assert_eq!(state, "string_state");
///    Ok(String::from("string_response"))
/// }
///
/// # async fn stateful() {
/// // Construct Stateful service factory with closure.
/// let service = ContextBuilder::new(|| async { Ok::<_, Infallible>(String::from("string_state")) })
///    // Stateful service factory would construct given service factory and pass (&State, Req) to it.
///    .service(fn_service(state_handler))
///    .new_service(())
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
        ContextBuilder<CF, C, SF1>: ServiceFactory<Req, ()>,
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

impl<Req, C> Context<'_, Req, C> {
    /// Destruct request into a tuple of (&state, parent_request).
    #[inline]
    pub fn borrow_parts_mut(&mut self) -> (&mut Req, &C) {
        (&mut self.req, self.state)
    }
}

impl<Req, C, T> BorrowReq<T> for &mut Context<'_, Req, C>
where
    Req: BorrowReq<T>,
{
    fn borrow(&self) -> &T {
        self.req.borrow()
    }
}

impl<Req, C, T> BorrowReqMut<T> for &mut Context<'_, Req, C>
where
    Req: BorrowReqMut<T>,
{
    fn borrow_mut(&mut self) -> &mut T {
        self.req.borrow_mut()
    }
}

impl<CF, Fut, C, CErr, F, S, Req, Arg, Res, Err> ServiceFactory<Req, Arg> for ContextBuilder<CF, C, F>
where
    CF: Fn() -> Fut,
    Fut: Future<Output = Result<C, CErr>> + 'static,
    C: 'static,
    Req: 'static,
    F: for<'c, 's> ServiceFactory<&'c mut Context<'s, Req, C>, Arg, Service = S, Response = Res, Error = Err>,
    S: for<'c, 's> Service<&'c mut Context<'s, Req, C>, Response = Res, Error = Err> + 'static,
    Err: From<CErr>,
{
    type Response = Res;
    type Error = Err;
    type Service = ContextService<C, S>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let state = (self.ctx_factory)();
        let service = self.service_factory.new_service(arg);
        async {
            let state = state.await?;
            let service = service.await?;
            Ok(ContextService { service, state })
        }
    }
}

#[doc(hidden)]
pub struct ContextService<C, S> {
    state: C,
    service: S,
}

impl<Req, C, S, Res, Err> Service<Req> for ContextService<C, S>
where
    Req: 'static,
    C: 'static,
    S: for<'c, 's> Service<&'c mut Context<'s, Req, C>, Response = Res, Error = Err> + 'static,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            self.service
                .call(&mut Context {
                    req,
                    state: &self.state,
                })
                .await
        }
    }
}

impl<Req, C, S, R, Res, Err> ReadyService<Req> for ContextService<C, S>
where
    Req: 'static,
    C: 'static,
    S: for<'c, 's> ReadyService<&'c mut Context<'s, Req, C>, Response = Res, Error = Err, Ready = R> + 'static,
{
    type Ready = R;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>>;

    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}

pub mod object {
    use super::*;

    use std::{boxed::Box, marker::PhantomData};

    use xitca_service::{
        fn_factory,
        object::{
            helpers::{ServiceFactoryObject, ServiceObject, Wrapper},
            ObjectConstructor,
        },
        Service, ServiceFactory,
    };

    pub struct ContextObjectConstructor<Req, C>(PhantomData<(Req, C)>);

    pub type ContextFactoryObject<Req: 'static, C: 'static, Res, Err> = impl for<'c, 's> ServiceFactory<
        &'c mut Context<'s, Req, C>,
        Response = Res,
        Error = Err,
        Service = ContextServiceObject<Req, C, Res, Err>,
    >;

    pub type ContextServiceObject<Req: 'static, C: 'static, Res, Err> =
        impl for<'c, 's> Service<&'c mut Context<'s, Req, C>, Response = Res, Error = Err>;

    impl<C, I, Svc, Req, Res, Err> ObjectConstructor<I> for ContextObjectConstructor<Req, C>
    where
        I: for<'c, 's> ServiceFactory<&'c mut Context<'s, Req, C>, (), Service = Svc, Response = Res, Error = Err>,
        Svc: for<'c, 's> Service<&'c mut Context<'s, Req, C>, Response = Res, Error = Err> + 'static,
        I: 'static,
        C: 'static,
        Req: 'static,
    {
        type Object = ContextFactoryObject<Req, C, Res, Err>;

        fn into_object(inner: I) -> Self::Object {
            let factory = fn_factory(move |_arg: ()| {
                let fut = inner.new_service(());
                async move {
                    let boxed_service = Box::new(Wrapper(fut.await?))
                        as Box<dyn for<'c, 's> ServiceObject<&'c mut Context<'s, Req, C>, Response = _, Error = _>>;
                    Ok(Wrapper(boxed_service))
                }
            });

            let boxed_factory = Box::new(Wrapper(factory))
                as Box<dyn for<'c, 's> ServiceFactoryObject<&'c mut Context<'s, Req, C>, Service = _>>;
            Wrapper(boxed_factory)
        }
    }
}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use xitca_service::{fn_service, ServiceFactoryExt};

    use crate::util::service::GenericRouter;
    use crate::{http::Response, request::Request, util::service::get};

    use super::*;

    struct Context2<'a, ST> {
        req: &'a mut Request<()>,
        state: &'a ST,
    }

    async fn into_context<'c>(
        req: &'c mut Context<'_, Request<()>, String>,
    ) -> Result<Context2<'c, String>, Infallible> {
        let (req, state) = req.borrow_parts_mut();
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
            .new_service(())
            .await
            .ok()
            .unwrap();

        let req = Request::default();

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }

    async fn handler(req: &mut Context<'_, Request<()>, String>) -> Result<Response<()>, Infallible> {
        let (_, state) = req.borrow_parts_mut();
        assert_eq!(state, "string_state");
        Ok(Response::new(()))
    }

    #[tokio::test]
    async fn test_state_in_router() {
        async fn enclosed<S, Req, C, Res, Err>(service: &S, req: &mut Context<'_, Req, C>) -> Result<Res, Err>
        where
            S: for<'c, 's> Service<&'c mut Context<'s, Req, C>, Response = Res, Error = Err>,
        {
            service.call(req).await
        }

        let router = GenericRouter::with_custom_object::<super::object::ContextObjectConstructor<_, _>>()
            .insert("/", get(fn_service(handler)))
            .enclosed_fn(enclosed);

        let service = ContextBuilder::new(|| async { Ok(String::from("string_state")) })
            .service(router)
            .new_service(())
            .await
            .ok()
            .unwrap();

        let req = Request::default();

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }
}
