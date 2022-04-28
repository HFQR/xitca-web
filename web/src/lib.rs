#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

mod app;
mod server;

pub mod error;
pub mod handler;
pub mod request;
pub mod response;

pub mod route {
    pub use xitca_http::util::service::route::{connect, delete, get, head, options, patch, post, put, trace, Route};
}

pub mod dev {
    pub use xitca_http::bytes;
    pub use xitca_service::{fn_service, BuildService, BuildServiceExt, Service};
}

pub use app::App;
pub use server::HttpServer;

pub use xitca_http::http;
