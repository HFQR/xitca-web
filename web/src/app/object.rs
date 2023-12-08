use core::marker::PhantomData;

use xitca_http::util::service::router::IntoObject;
use xitca_service::{
    object::{BoxedSyncServiceObject, ServiceObject},
    Service,
};

use crate::context::WebContext;

pub type WebObject<C, B, Res, Err> = Box<dyn for<'r> ServiceObject<WebContext<'r, C, B>, Response = Res, Error = Err>>;

impl<C, B, I, Res, Err> IntoObject<I, ()> for WebContext<'_, C, B>
where
    C: 'static,
    B: 'static,
    I: Service + Send + Sync + 'static,
    I::Response: for<'r> Service<WebContext<'r, C, B>, Response = Res, Error = Err> + 'static,
{
    type Object = BoxedSyncServiceObject<(), WebObject<C, B, Res, Err>, I::Error>;

    fn into_object(inner: I) -> Self::Object {
        struct Builder<I, C, B>(I, PhantomData<fn(C, B)>);

        impl<C, I, B, Res, Err> Service for Builder<I, C, B>
        where
            I: Service + 'static,
            I::Response: for<'r> Service<WebContext<'r, C, B>, Response = Res, Error = Err> + 'static,
        {
            type Response = WebObject<C, B, Res, Err>;
            type Error = I::Error;

            async fn call(&self, arg: ()) -> Result<Self::Response, Self::Error> {
                self.0.call(arg).await.map(|s| Box::new(s) as _)
            }
        }

        Box::new(Builder(inner, PhantomData))
    }
}
