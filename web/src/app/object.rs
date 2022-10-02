use std::{boxed::Box, marker::PhantomData};

use xitca_service::{
    fn_build,
    object::{
        helpers::{ServiceObject, Wrapper},
        Object, ObjectConstructor,
    },
    BuildService, BuildServiceExt, Service,
};

use crate::request::WebRequest;

pub struct WebObjectConstructor<C, B>(PhantomData<(C, B)>);

pub type WebFactoryObject<Arg, C, B, BErr, Res, Err> = Object<Arg, WebServiceObject<C, B, Res, Err>, BErr>;

pub type WebServiceObject<C, B, Res, Err> = impl for<'r> Service<WebRequest<'r, C, B>, Response = Res, Error = Err>;

impl<C, B, I, Svc, BErr, Res, Err> ObjectConstructor<I> for WebObjectConstructor<C, B>
where
    I: BuildService<Service = Svc, Error = BErr> + 'static,
    Svc: for<'r> Service<WebRequest<'r, C, B>, Response = Res, Error = Err> + 'static,
{
    type Object = WebFactoryObject<(), C, B, I::Error, Res, Err>;

    fn into_object(inner: I) -> Self::Object {
        let factory = fn_build(move |_arg: ()| {
            let fut = inner.build(());
            async move {
                let boxed_service = Box::new(fut.await?)
                    as Box<dyn for<'r> ServiceObject<WebRequest<'r, C, B>, Response = _, Error = _>>;
                Ok(Wrapper(boxed_service))
            }
        })
        .boxed_future();

        Box::new(factory)
    }
}
