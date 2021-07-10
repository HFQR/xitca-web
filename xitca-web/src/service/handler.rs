use std::{
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use xitca_service::{Service, ServiceFactory};

use crate::extract::FromRequest;
use crate::request::WebRequest;
use crate::response::Responder;

pub trait Handler<T, R>: Clone + 'static
where
    R: Future,
{
    fn call(&self, param: T) -> R;
}

#[doc(hidden)]
/// Extract arguments from request, run handler function and make response.
pub struct HandlerService<State, F, T, R>
where
    State: 'static,
    F: Handler<T, R>,
    R: Future,
    R::Output: Responder<State>,
{
    hnd: F,
    _phantom: PhantomData<(State, T, R)>,
}

impl<State, F, T, R> Clone for HandlerService<State, F, T, R>
where
    State: 'static,
    F: Handler<T, R>,
    R: Future,
    R::Output: Responder<State>,
{
    fn clone(&self) -> Self {
        Self {
            hnd: self.hnd.clone(),
            _phantom: PhantomData,
        }
    }
}

#[cfg(test)]
impl<State, F, T, R> HandlerService<State, F, T, R>
where
    State: 'static,
    F: Handler<T, R>,
    R: Future,
    R::Output: Responder<State>,
{
    pub(crate) fn new(hnd: F) -> Self {
        Self {
            hnd,
            _phantom: PhantomData,
        }
    }
}

impl<'r, State, F, T, R, Err> ServiceFactory<&'r mut WebRequest<'_, State>> for HandlerService<State, F, T, R>
where
    F: Handler<T, R>,
    R: Future,
    R::Output: Responder<State>,
    T: FromRequest<'r, State, Error = Err>,
{
    type Response = R::Output;
    type Error = Err;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let this = Self::clone(self);
        async { Ok(this) }
    }
}

impl<'r, 'rb, State, F, T, R, Err> Service<&'r mut WebRequest<'rb, State>> for HandlerService<State, F, T, R>
where
    F: Handler<T, R>,
    R: Future,
    R::Output: Responder<State>,
    T: FromRequest<'r, State, Error = Err>,
{
    type Response = R::Output;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&self, req: &'r mut WebRequest<'rb, State>) -> Self::Future<'_> {
        async move {
            let extract = T::from_request(req).await?;
            Ok(self.hnd.call(extract).await)
        }
    }
}

/// FromRequest trait impl for tuples
macro_rules! factory_tuple ({ $($param:ident)* } => {
    impl<Func, $($param,)* R> Handler<($($param,)*), R> for Func
    where
        Func: Fn($($param),*) -> R + Clone + 'static,
        R: Future,
    {
        #[allow(non_snake_case)]
        fn call(&self, ($($param,)*): ($($param,)*)) -> R {
            (self)($($param,)*)
        }
    }
});

factory_tuple! {}
factory_tuple! { A }
factory_tuple! { A B }
factory_tuple! { A B C }
factory_tuple! { A B C D }
factory_tuple! { A B C D E }
factory_tuple! { A B C D E F }
factory_tuple! { A B C D E F G }
factory_tuple! { A B C D E F G H }
factory_tuple! { A B C D E F G H I }
factory_tuple! { A B C D E F G H I J }
factory_tuple! { A B C D E F G H I J K }

#[cfg(test)]
mod test {

    use crate::extract::State;
    use crate::request::WebRequest;
    use crate::response::{Responder, WebResponse};
    use crate::service::HandlerService;

    use xitca_http::ResponseBody;
    use xitca_service::{Service, ServiceFactory};

    async fn handler(req: &WebRequest<'_, String>, state: State<'_, String>) -> WebResponse {
        let state2 = req.state();
        assert_eq!(state2, &*state);
        assert_eq!("123", state2.as_str());
        WebResponse::new(ResponseBody::None)
    }

    #[tokio::test]
    async fn handler_service() {
        let service = HandlerService::new(handler).new_service(()).await.ok().unwrap();

        let data = String::from("123");

        let mut req = WebRequest::with_state(&data);

        let _ = service.call(&mut req).await.ok().unwrap().respond_to(&mut req);
    }
}
