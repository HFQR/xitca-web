#![no_std]
#![forbid(unsafe_code)]

mod async_closure;
mod service;

pub mod middleware;
pub mod pipeline;
pub mod ready;

pub use self::{
    async_closure::AsyncClosure,
    pipeline::{EnclosedFactory, EnclosedFnFactory, MapErrorServiceFactory},
    service::{fn_build, fn_build_nop, fn_service, FnService, Service, ServiceExt},
};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
pub mod object;

#[cfg(feature = "alloc")]
pub type BoxFuture<'a, Res, Err> =
    core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = Result<Res, Err>> + 'a>>;

#[cfg(feature = "std")]
extern crate std;
