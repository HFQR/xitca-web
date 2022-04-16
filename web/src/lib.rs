#![forbid(unsafe_code)]
#![allow(incomplete_features)]
#![feature(generic_associated_types)]
#![feature(type_alias_impl_trait)]
#![feature(adt_const_params)]

mod app;
mod server;

pub mod error;
pub mod extract;
pub mod request;
pub mod response;
pub mod service;

pub use app::App;
pub use server::HttpServer;

pub use xitca_http::http;

pub mod dev {
    pub use xitca_http::bytes;
    pub use xitca_service::{fn_service, Service, ServiceFactory, ServiceFactoryExt};
}

pub mod object {

    use std::{boxed::Box, marker::PhantomData};

    use xitca_service::{
        fn_factory,
        object::{
            helpers::{ServiceFactoryObject, ServiceObject, Wrapper},
            ObjectConstructor,
        },
        Service, ServiceFactory,
    };

    use crate::request::WebRequest;

    pub struct WebObjectConstructor<S>(PhantomData<S>);

    pub type WebFactoryObject<S: 'static, Res, Err> = impl for<'r, 's> ServiceFactory<
        &'r mut WebRequest<'s, S>,
        Response = Res,
        Error = Err,
        Service = WebServiceObject<S, Res, Err>,
    >;

    pub type WebServiceObject<S: 'static, Res, Err> =
        impl for<'r, 's> Service<&'r mut WebRequest<'s, S>, Response = Res, Error = Err>;

    impl<S, I, Svc, Res, Err> ObjectConstructor<I> for WebObjectConstructor<S>
    where
        I: for<'rb, 'r> ServiceFactory<&'rb mut WebRequest<'r, S>, (), Service = Svc, Response = Res, Error = Err>,
        Svc: for<'rb, 'r> Service<&'rb mut WebRequest<'r, S>, Response = Res, Error = Err> + 'static,
        I: 'static,
        S: 'static,
    {
        type Object = WebFactoryObject<S, Res, Err>;

        fn into_object(inner: I) -> Self::Object {
            let factory = fn_factory(move |_arg: ()| {
                let fut = inner.new_service(());
                async move {
                    let boxed_service = Box::new(Wrapper(fut.await?))
                        as Box<dyn for<'r, 's> ServiceObject<&'r mut WebRequest<'s, S>, Response = _, Error = _>>;
                    Ok(Wrapper(boxed_service))
                }
            });

            let boxed_factory = Box::new(Wrapper(factory))
                as Box<dyn for<'r, 's> ServiceFactoryObject<&'r mut WebRequest<'s, S>, Service = _>>;
            Wrapper(boxed_factory)
        }
    }
}
