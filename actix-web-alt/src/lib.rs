#![forbid(unsafe_code)]
#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

mod app;
mod error;
mod extract;
mod guard;
mod request;
mod response;
mod server;
mod service;

pub use app::App;
pub use request::WebRequest;
pub use response::{Responder, WebResponse};
pub use server::HttpServer;

pub mod dev {
    pub use actix_service_alt::{Service, ServiceFactory, ServiceFactoryExt, Transform};

    pub use crate::service::EnumService;
}
