#![no_std]
#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

extern crate alloc;

mod async_closure;
mod factory;
mod service;

pub mod middleware;
pub mod object;
pub mod ready;

pub use self::{
    async_closure::AsyncClosure,
    factory::{
        fn_factory, fn_service, EnclosedFactory, EnclosedFnFactory, MapErrorServiceFactory, ServiceFactory,
        ServiceFactoryExt,
    },
    service::Service,
};

use core::{future::Future, pin::Pin};

use alloc::boxed::Box;

pub(crate) type BoxFuture<'a, Res, Err> = Pin<Box<dyn Future<Output = Result<Res, Err>> + 'a>>;

/// Macro for generate an enum Service/ServiceFactory with given list of identifiers.
///
/// # Example:
/// ```rust
/// #![feature(generic_associated_types, type_alias_impl_trait)]
///
/// # fn gen() {
/// struct Foo;
/// struct Bar;
///
/// xitca_service::enum_service!(Foo, Bar);
///
/// let _ = EnumService::Foo::<_, Bar>(Foo);
/// let _ = EnumService::Bar::<Foo, _>(Bar);
/// # }
/// ```
#[macro_export]
macro_rules! enum_service {
    ($($factory: ident),*) => {
        #[allow(non_camel_case_types)]
        enum EnumService<$($factory),*> {
            $(
                $factory($factory),
            ) +
        }

        impl<Req, Res, Err, $($factory),*> ::xitca_service::Service<Req> for EnumService<$($factory),*>
        where
            $(
                $factory: ::xitca_service::Service<Req, Response = Res, Error = Err>,
            ) +
        {
            type Response = Res;
            type Error = Err;
            type Future<'f> where Self: 'f = impl ::core::future::Future<Output = Result<Self::Response, Self::Error>>;

            #[inline]
            fn call(&self, req: Req) -> Self::Future<'_> {
                async move {
                    match self {
                        $(
                            Self::$factory(ref s) => s.call(req).await,
                        ) +
                    }
                }
            }
        }

        #[allow(non_camel_case_types)]
        #[derive(Clone)]
        enum EnumServiceFactory<$($factory: Clone),*> {
            $(
                $factory($factory),
            ) +
        }

        impl<Req, Arg, Res, Err, $($factory),*> ::xitca_service::ServiceFactory<Req, Arg> for EnumServiceFactory<$($factory),*>
        where
            $(
                $factory: ::xitca_service::ServiceFactory<Req, Arg, Response = Res, Error = Err> + Clone,
                $factory::Future: 'static,
            ) +
        {
            type Response = Res;
            type Error = Err;
            type Service = EnumService<$($factory::Service),*>;
            type Future = ::core::pin::Pin<Box<dyn ::core::future::Future<Output = Result<Self::Service, Self::Error>>>>;

            fn new_service(&self, arg: Arg) -> Self::Future {
                match self {
                    $(
                        Self::$factory(ref f) => {
                            let fut = f.new_service(arg);
                            Box::pin(async move { fut.await.map(EnumService::$factory) })
                        },
                    ) +
                }
            }
        }
    }
}
