#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

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
