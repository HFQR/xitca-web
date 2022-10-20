use std::{boxed::Box, future::Future, marker::PhantomData};

use xitca_service::{
    object::{
        helpers::{ServiceObject, Wrapper},
        Object, ObjectConstructor,
    },
    Service,
};

use crate::request::WebRequest;

pub struct WebObjectConstructor<C, B>(PhantomData<(C, B)>);

pub type WebFactoryObject<Arg, C, B, BErr, Res, Err> = Object<Arg, WebServiceObject<C, B, Res, Err>, BErr>;

pub type WebServiceObject<C, B, Res, Err> = impl for<'r> Service<WebRequest<'r, C, B>, Response = Res, Error = Err>;

impl<C, B, I, Svc, BErr, Res, Err> ObjectConstructor<I> for WebObjectConstructor<C, B>
where
    C: 'static,
    B: 'static,
    I: Service<Response = Svc, Error = BErr> + 'static,
    Svc: for<'r> Service<WebRequest<'r, C, B>, Response = Res, Error = Err> + 'static,
{
    type Object = WebFactoryObject<(), C, B, I::Error, Res, Err>;

    fn into_object(inner: I) -> Self::Object {
        struct WebObjBuilder<I, C, B>(I, PhantomData<(C, B)>);

        impl<C, I, Svc, BErr, B, Res, Err> Service for WebObjBuilder<I, C, B>
        where
            I: Service<Response = Svc, Error = BErr> + 'static,
            Svc: for<'r> Service<WebRequest<'r, C, B>, Response = Res, Error = Err> + 'static,
        {
            type Response = Wrapper<Box<dyn for<'r> ServiceObject<WebRequest<'r, C, B>, Response = Res, Error = Err>>>;
            type Error = BErr;
            type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

            fn call(&self, arg: ()) -> Self::Future<'_> {
                async move {
                    let service = self.0.call(arg).await?;
                    Ok(Wrapper(Box::new(service) as _))
                }
            }
        }

        Wrapper(Box::new(WebObjBuilder(inner, PhantomData)))
    }
}
