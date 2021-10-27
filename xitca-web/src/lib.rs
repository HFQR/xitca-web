#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

mod app;
mod extract;
mod guard;
mod server;
mod service;

pub mod error;
pub mod request;
pub mod response;

pub use app::App;
pub use server::HttpServer;

pub use xitca_http::http;

pub mod dev {
    pub use xitca_http::bytes;
    pub use xitca_service::{fn_service, Service, ServiceFactory, ServiceFactoryExt, Transform};
    // pub use crate::service::EnumService;
}
