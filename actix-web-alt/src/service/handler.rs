use std::{
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use actix_service_alt::Service;

use crate::extract::FromRequest;
use crate::request::WebRequest;
use crate::response::{Responder, WebResponse};

/// A request handler is an async function that accepts zero or more parameters that can be
/// extracted from a request (i.e., [`impl FromRequest`](crate::FromRequest)) and returns a type
/// that can be converted into an [`HttpResponse`] (that is, it impls the [`Responder`] trait).
///
/// If you got the error `the trait Handler<_, _, _> is not implemented`, then your function is not
/// a valid handler. See [Request Handlers](https://actix.rs/docs/handlers/) for more information.
pub trait Handler<State, T, R>: Clone + 'static
where
    R: Future,
{
    fn call(&self, param: T) -> R;
}

#[doc(hidden)]
/// Extract arguments from request, run factory function and make response.
pub struct HandlerService<State, F, T, R>
where
    State: 'static,
    F: Handler<State, T, R>,
    R: Future,
    R::Output: Responder<State>,
{
    hnd: F,
    _phantom: PhantomData<(State, T, R)>,
}

impl<'r, State, F, T, R, Err> Service<&'r WebRequest<'r, State>> for HandlerService<State, F, T, R>
where
    State: 'static,
    F: Handler<State, T, R>,
    R: Future + 'static,
    R::Output: Responder<State>,
    T: for<'f> FromRequest<'f, State, Error = Err>,
    Err: 'static,
{
    type Response = WebResponse;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'c>(&'c self, req: &'r WebRequest<'_, State>) -> Self::Future<'c>
    where
        'r: 'c,
    {
        async move {
            let extract = T::from_request(req).await?;
            let res = self.hnd.call(extract).await.respond_to(req);
            Ok(res)
        }
    }
}

/// FromRequest trait impl for tuples
macro_rules! factory_tuple ({ $($param:ident)* } => {
    impl<State, Func, $($param,)* Res> Handler<State, ($($param,)*), Res> for Func
    where
        State: 'static,
        Func: Fn($($param),*) -> Res + Clone + 'static,
        Res: Future,
        Res::Output: Responder<State>,
    {
        #[allow(non_snake_case)]
        fn call(&self, ($($param,)*): ($($param,)*)) -> Res {
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
    use super::*;

    use crate::extract::State;

    async fn handler(req: State<String>) -> WebResponse {
        unimplemented!();
    }

    #[test]
    fn test_handler_service() {
        let service = HandlerService {
            hnd: handler,
            _phantom: PhantomData,
        };

        let data = String::from("123");

        let web_req = WebRequest::with_state(&data);

        let _ = async move {
            let _ = service.call(&web_req).await;
        };
    }
}
