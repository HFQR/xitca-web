#![no_std]
#![forbid(unsafe_code)]
#![feature(type_alias_impl_trait)]

mod async_closure;
mod build;
mod service;

pub mod middleware;
pub mod pipeline;
pub mod ready;

pub use self::{
    async_closure::AsyncClosure,
    build::{fn_build, fn_service, BuildService, BuildServiceExt},
    pipeline::{EnclosedFactory, EnclosedFnFactory, MapErrorServiceFactory},
    service::Service,
};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
pub mod object;

#[cfg(feature = "alloc")]
pub type BoxFuture<'a, Res, Err> =
    core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = Result<Res, Err>> + 'a>>;
