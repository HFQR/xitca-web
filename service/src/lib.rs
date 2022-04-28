#![no_std]
#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

extern crate alloc;

mod async_closure;
mod build;
mod service;

pub mod middleware;
pub mod object;
pub mod pipeline;
pub mod ready;

pub use self::{
    async_closure::AsyncClosure,
    build::{fn_build, fn_service, BuildService, BuildServiceExt},
    pipeline::{EnclosedFactory, EnclosedFnFactory, MapErrorServiceFactory},
    service::Service,
};

type BoxFuture<'a, Res, Err> =
    core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = Result<Res, Err>> + 'a>>;
