#![no_std]
#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

extern crate alloc;

mod async_closure;
mod factory;
mod service;

pub mod middleware;
pub mod object;
pub mod pipeline;
pub mod ready;

pub use self::{
    async_closure::AsyncClosure,
    factory::{fn_factory, fn_service, ServiceFactory, ServiceFactoryExt},
    pipeline::{EnclosedFactory, EnclosedFnFactory, MapErrorServiceFactory},
    service::Service,
};

type BoxFuture<'a, Res, Err> =
    core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = Result<Res, Err>> + 'a>>;
