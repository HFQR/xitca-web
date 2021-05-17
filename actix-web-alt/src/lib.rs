#![forbid(unsafe_code)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

mod app;
mod error;
mod extract;
mod request;
mod response;
mod server;
mod service;

pub use server::HttpServer;
