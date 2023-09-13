use core::{future::Future, marker::PhantomData};

use xitca_http::util::service::router::IntoObject;
use xitca_service::{
    object::{BoxedServiceObject, ServiceObject},
    Service,
};

use crate::request::WebRequest;

pub type WebObject<C, B, Res, Err> = Box<dyn for<'r> ServiceObject<WebRequest<'r, C, B>, Response = Res, Error = Err>>;

impl<C, B, I, Svc, BErr, Res, Err> IntoObject<I, ()> for WebRequest<'_, C, B>
where
    C: 'static,
    B: 'static,
    I: Service<Response = Svc, Error = BErr> + 'static,
    Svc: for<'r> Service<WebRequest<'r, C, B>, Response = Res, Error = Err> + 'static,
{
    type Object = BoxedServiceObject<(), WebObject<C, B, Res, Err>, BErr>;

    fn into_object(inner: I) -> Self::Object {
        struct Builder<I, C, B>(I, PhantomData<(C, B)>);

        impl<C, I, Svc, BErr, B, Res, Err> Service for Builder<I, C, B>
        where
            I: Service<Response = Svc, Error = BErr> + 'static,
            Svc: for<'r> Service<WebRequest<'r, C, B>, Response = Res, Error = Err> + 'static,
        {
            type Response = WebObject<C, B, Res, Err>;
            type Error = BErr;
            type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f;

            fn call<'s>(&'s self, arg: ()) -> Self::Future<'s>
            where
                (): 's,
            {
                async move { self.0.call(arg).await.map(|s| Box::new(s) as _) }
            }
        }

        Box::new(Builder(inner, PhantomData))
    }
}
