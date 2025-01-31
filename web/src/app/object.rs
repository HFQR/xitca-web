use core::marker::PhantomData;

use xitca_http::util::service::router::{IntoObject, PathGen, RouteGen, RouteObject};
use xitca_service::{Service, object::ServiceObject};

use crate::context::WebContext;

pub type WebObject<C, B, Res, Err> = Box<dyn for<'r> ServiceObject<WebContext<'r, C, B>, Response = Res, Error = Err>>;

impl<C, B, I, Res, Err> IntoObject<I, ()> for WebContext<'_, C, B>
where
    C: 'static,
    B: 'static,
    I: Service + RouteGen + Send + Sync + 'static,
    I::Response: for<'r> Service<WebContext<'r, C, B>, Response = Res, Error = Err> + 'static,
{
    type Object = RouteObject<(), WebObject<C, B, Res, Err>, I::Error>;

    fn into_object(inner: I) -> Self::Object {
        struct Builder<I, C, B>(I, PhantomData<fn(C, B)>);

        impl<I, C, B> PathGen for Builder<I, C, B>
        where
            I: PathGen,
        {
            fn path_gen(&mut self, prefix: &str) -> String {
                self.0.path_gen(prefix)
            }
        }

        impl<I, C, B> RouteGen for Builder<I, C, B>
        where
            I: RouteGen,
        {
            type Route<R> = I::Route<R>;

            fn route_gen<R>(route: R) -> Self::Route<R> {
                I::route_gen(route)
            }
        }

        impl<C, I, B, Res, Err> Service for Builder<I, C, B>
        where
            I: Service + RouteGen + 'static,
            I::Response: for<'r> Service<WebContext<'r, C, B>, Response = Res, Error = Err> + 'static,
        {
            type Response = WebObject<C, B, Res, Err>;
            type Error = I::Error;

            async fn call(&self, arg: ()) -> Result<Self::Response, Self::Error> {
                self.0.call(arg).await.map(|s| Box::new(s) as _)
            }
        }

        RouteObject(Box::new(Builder(inner, PhantomData)))
    }
}
