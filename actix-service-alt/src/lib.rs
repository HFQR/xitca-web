#![no_std]
#![forbid(unsafe_code)]
#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

extern crate alloc;

mod factory;
mod service;
mod transform;

pub use self::{
    factory::{fn_factory, ServiceFactory, ServiceFactoryExt},
    service::Service,
    transform::{Transform, TransformFactory},
};
