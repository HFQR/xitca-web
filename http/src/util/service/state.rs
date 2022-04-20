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
/// # use xitca_http::util::service::state::{State, StateRequest};
/// # use xitca_service::{fn_service, Service, ServiceFactory};
///
/// // function service.
/// async fn state_handler(req: StateRequest<'_, String, String>) -> Result<String, Infallible> {
///    let (state, parent_req) = req.into_parts();
///    assert_eq!(state, "string_state");
///    Ok(String::from("string_response"))
/// }
///
/// # async fn stateful() {
/// // Construct Stateful service factory with closure.
/// let service = State::new(|| async { Ok::<_, Infallible>(String::from("string_state")) })
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
pub struct State<SF, ST, F = ()> {
    state_factory: SF,
    factory: F,
    _state: PhantomData<ST>,
}

impl<SF, Fut, ST, E> State<SF, ST>
where
    SF: Fn() -> Fut,
    Fut: Future<Output = Result<ST, E>>,
{
    /// Make a stateful service factory with given future.
    pub fn new(state_factory: SF) -> Self {
        Self {
            state_factory,
            factory: (),
            _state: PhantomData,
        }
    }
}

impl<SF, ST, F> State<SF, ST, F> {
    /// The constructor of service type that would receive state.
    pub fn service<Req, F1>(self, factory: F1) -> State<SF, ST, F1>
    where
        State<SF, ST, F1>: ServiceFactory<Req, ()>,
    {
        State {
            state_factory: self.state_factory,
            factory,
            _state: PhantomData,
        }
    }
}

/// Specialized Request type State service factory.
///
/// This type enables borrow parent service request type as &Req and &mut Req
pub struct StateRequest<'a, ST, Req> {
    state: &'a ST,
    req: Req,
}

impl<'a, ST, Req> StateRequest<'a, ST, Req> {
    /// Destruct request into a tuple of (&state, parent_request).
    #[inline]
    pub fn into_parts(self) -> (&'a ST, Req) {
        (self.state, self.req)
    }
}

impl<ST, Req, T> BorrowReq<T> for StateRequest<'_, ST, Req>
where
    Req: BorrowReq<T>,
{
    fn borrow(&self) -> &T {
        self.req.borrow()
    }
}

impl<ST, Req, T> BorrowReqMut<T> for StateRequest<'_, ST, Req>
where
    Req: BorrowReqMut<T>,
{
    fn borrow_mut(&mut self) -> &mut T {
        self.req.borrow_mut()
    }
}

impl<SF, Fut, ST, STErr, F, S, Req, Arg, Res, Err> ServiceFactory<Req, Arg> for State<SF, ST, F>
where
    SF: Fn() -> Fut,
    Fut: Future<Output = Result<ST, STErr>> + 'static,
    ST: 'static,
    F: for<'r> ServiceFactory<StateRequest<'r, ST, Req>, Arg, Service = S, Response = Res, Error = Err>,
    S: for<'r> Service<StateRequest<'r, ST, Req>, Response = Res, Error = Err> + 'static,
    Err: From<STErr>,
{
    type Response = Res;
    type Error = Err;
    type Service = StateService<ST, S>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let state = (self.state_factory)();
        let service = self.factory.new_service(arg);
        async {
            let state = state.await?;
            let service = service.await?;
            Ok(StateService { service, state })
        }
    }
}

#[doc(hidden)]
pub struct StateService<ST, S> {
    state: ST,
    service: S,
}

impl<Req, ST, S, Res, Err> Service<Req> for StateService<ST, S>
where
    ST: 'static,
    S: for<'r> Service<StateRequest<'r, ST, Req>, Response = Res, Error = Err> + 'static,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, req: Req) -> Self::Future<'_> {
        self.service.call(StateRequest {
            state: &self.state,
            req,
        })
    }
}

impl<Req, ST, S, R, Res, Err> ReadyService<Req> for StateService<ST, S>
where
    ST: 'static,
    S: for<'r> ReadyService<StateRequest<'r, ST, Req>, Response = Res, Error = Err, Ready = R> + 'static,
{
    type Ready = R;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>>;

    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}

mod object {
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

    pub struct StateObjectConstructor<S, Req>(PhantomData<(S, Req)>);

    pub type StateFactoryObject<S: 'static, Req: 'static, Res, Err> = impl for<'r> ServiceFactory<
        StateRequest<'r, S, Req>,
        Response = Res,
        Error = Err,
        Service = StateServiceObject<S, Req, Res, Err>,
    >;

    pub type StateServiceObject<S: 'static, Req: 'static, Res, Err> =
        impl for<'r> Service<StateRequest<'r, S, Req>, Response = Res, Error = Err>;

    impl<S, I, Svc, Req, Res, Err> ObjectConstructor<I> for StateObjectConstructor<S, Req>
    where
        I: for<'r> ServiceFactory<StateRequest<'r, S, Req>, (), Service = Svc, Response = Res, Error = Err>,
        Svc: for<'r> Service<StateRequest<'r, S, Req>, Response = Res, Error = Err> + 'static,
        I: 'static,
        S: 'static,
        Req: 'static,
    {
        type Object = StateFactoryObject<S, Req, Res, Err>;

        fn into_object(inner: I) -> Self::Object {
            let factory = fn_factory(move |_arg: ()| {
                let fut = inner.new_service(());
                async move {
                    let boxed_service = Box::new(Wrapper(fut.await?))
                        as Box<dyn for<'r> ServiceObject<StateRequest<'r, S, Req>, Response = _, Error = _>>;
                    Ok(Wrapper(boxed_service))
                }
            });

            let boxed_factory = Box::new(Wrapper(factory))
                as Box<dyn for<'r> ServiceFactoryObject<StateRequest<'r, S, Req>, Service = _>>;
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

    struct Context<'a, ST> {
        req: Request<()>,
        state: &'a ST,
    }

    async fn into_context(req: StateRequest<'_, String, Request<()>>) -> Result<Context<'_, String>, Infallible> {
        let (state, req) = req.into_parts();
        assert_eq!(state, "string_state");
        Ok(Context { req, state })
    }

    async fn ctx_handler(ctx: Context<'_, String>) -> Result<Response<()>, Infallible> {
        assert_eq!(ctx.state, "string_state");
        assert_eq!(ctx.req.method().as_str(), "GET");
        Ok(Response::new(()))
    }

    #[tokio::test]
    async fn test_state_and_then() {
        let service = State::new(|| async { Ok::<_, Infallible>(String::from("string_state")) })
            .service(fn_service(into_context).and_then(fn_service(ctx_handler)))
            .new_service(())
            .await
            .ok()
            .unwrap();

        let req = Request::default();

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }

    async fn handler(req: StateRequest<'_, String, Request<()>>) -> Result<Response<()>, Infallible> {
        let (state, _) = req.into_parts();
        assert_eq!(state, "string_state");
        Ok(Response::new(()))
    }

    #[tokio::test]
    async fn test_state_in_router() {
        let router = GenericRouter::with_custom_object::<super::object::StateObjectConstructor<_, _>>()
            .insert("/", get(fn_service(handler)));

        let service = State::new(|| async { Ok(String::from("string_state")) })
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
