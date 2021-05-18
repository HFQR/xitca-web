#![forbid(unsafe_code)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

mod app;
mod error;
mod extract;
mod request;
mod response;
mod server;
mod service;

pub use app::App;
pub use request::WebRequest;
pub use response::{Responder, WebResponse};
pub use server::HttpServer;
pub use service::EnumService;
