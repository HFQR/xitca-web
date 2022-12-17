#![forbid(unsafe_code)]
#![feature(type_alias_impl_trait)]

mod app;
#[cfg(feature = "server")]
mod server;

pub mod error;
pub mod handler;
pub mod middleware;
pub mod request;
pub mod response;
pub mod service;
pub mod stream;
pub mod test;

#[cfg(feature = "codegen")]
pub mod codegen {
    /// Derive macro for individual struct field extractable through [StateRef](crate::handler::state::StateRef)
    ///
    /// # Example:
    /// ```rust
    /// # use xitca_web::{codegen::State, handler::{handler_service, state::StateRef}, request::WebRequest, App};
    ///
    /// // use derive macro and attribute to mark the field that can be extracted.
    /// #[derive(State, Clone)]
    /// struct MyState {
    ///     #[borrow]
    ///     field: u128
    /// }
    ///
    /// # async fn app() {
    /// // construct App with MyState type.
    /// App::with_current_thread_state(MyState { field: 996 })
    ///     .at("/", handler_service(index))
    /// #   .at("/nah", handler_service(nah));
    /// # }
    ///
    /// // extract u128 typed field from MyState.
    /// async fn index(StateRef(num): StateRef<'_, u128>) {
    ///     assert_eq!(*num, 996);
    /// }
    /// # async fn nah(_: &WebRequest<'_, MyState>) {
    /// #   // needed to infer the body type of request
    /// # }
    /// ```
    pub use xitca_codegen::State;
}

pub mod route {
    pub use xitca_http::util::service::route::{connect, delete, get, head, options, patch, post, put, trace, Route};
}

pub mod dev {
    pub use xitca_http::bytes;

    pub use xitca_service as service;
}

pub use app::App;
#[cfg(feature = "server")]
pub use server::HttpServer;
pub use stream::WebStream;

pub use xitca_http::http;
