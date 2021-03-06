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

pub struct WebObjectConstructor<S>(PhantomData<S>);

pub type WebFactoryObject<Arg, S, BErr, Res, Err> = Object<Arg, WebServiceObject<S, Res, Err>, BErr>;

pub type WebServiceObject<S, Res, Err> = impl for<'r> Service<WebRequest<'r, S>, Response = Res, Error = Err>;

impl<S, I, Svc, BErr, Res, Err> ObjectConstructor<I> for WebObjectConstructor<S>
where
    I: BuildService<Service = Svc, Error = BErr> + 'static,
    Svc: for<'r> Service<WebRequest<'r, S>, Response = Res, Error = Err> + 'static,
{
    type Object = WebFactoryObject<(), S, I::Error, Res, Err>;

    fn into_object(inner: I) -> Self::Object {
        let factory = fn_build(move |_arg: ()| {
            let fut = inner.build(());
            async move {
                let boxed_service = Box::new(Wrapper(fut.await?))
                    as Box<dyn for<'r> ServiceObject<WebRequest<'r, S>, Response = _, Error = _>>;
                Ok(Wrapper(boxed_service))
            }
        })
        .boxed_future();

        Box::new(factory)
    }
}
