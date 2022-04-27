use std::{boxed::Box, marker::PhantomData};

use xitca_service::{
    fn_build,
    object::{
        helpers::{ServiceFactoryObject, ServiceObject, Wrapper},
        ObjectConstructor,
    },
    BuildService, Service,
};

use crate::request::WebRequest;

pub struct WebObjectConstructor<S>(PhantomData<S>);

pub type WebFactoryObject<S: 'static, BErr, Res, Err> =
    impl BuildService<Error = BErr, Service = WebServiceObject<S, Res, Err>>;

pub type WebServiceObject<S: 'static, Res, Err> =
    impl for<'r, 's> Service<&'r mut WebRequest<'s, S>, Response = Res, Error = Err>;

impl<S, I, Svc, BErr, Res, Err> ObjectConstructor<I> for WebObjectConstructor<S>
where
    I: BuildService<Service = Svc, Error = BErr>,
    Svc: for<'rb, 'r> Service<&'rb mut WebRequest<'r, S>, Response = Res, Error = Err> + 'static,
    I: 'static,
    S: 'static,
{
    type Object = WebFactoryObject<S, BErr, Res, Err>;

    fn into_object(inner: I) -> Self::Object {
        let factory = fn_build(move |_arg: ()| {
            let fut = inner.build(());
            async move {
                let boxed_service = Box::new(Wrapper(fut.await?))
                    as Box<dyn for<'r, 's> ServiceObject<&'r mut WebRequest<'s, S>, Response = _, Error = _>>;
                Ok(Wrapper(boxed_service))
            }
        });

        let boxed_factory = Box::new(Wrapper(factory)) as Box<dyn ServiceFactoryObject<Service = _, Error = _>>;
        Wrapper(boxed_factory)
    }
}
