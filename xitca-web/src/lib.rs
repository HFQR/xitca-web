#![forbid(unsafe_code)]
#![allow(incomplete_features)]
#![feature(generic_associated_types, type_alias_impl_trait)]

mod app;
mod error;
mod extract;
mod guard;
mod server;
mod service;

pub mod request;
pub mod response;

pub use app::App;
pub use server::HttpServer;

pub mod dev {
    pub use xitca_service::{fn_service, Service, ServiceFactory, ServiceFactoryExt, Transform};

    // pub use crate::service::EnumService;
}
