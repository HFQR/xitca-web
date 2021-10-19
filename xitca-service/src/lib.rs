#![no_std]
#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

extern crate alloc;

mod factory;
mod service;
mod transform;

pub use self::{
    factory::{fn_service, ServiceFactory, ServiceFactoryExt},
    service::Service,
    transform::{Transform, TransformFactory},
};
