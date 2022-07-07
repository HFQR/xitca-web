#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

mod app;
mod server;

pub mod error;
pub mod handler;
pub mod request;
pub mod response;

#[cfg(feature = "codegen")]
pub mod codegen {
    /// Derive macro for individual struct field extractable through [StateRef](crate::handler::state::StateRef)
    ///
    /// # Example:
    /// ```rust
    /// # use xitca_web::{codegen::State, handler::{handler_service, state::StateRef}, App};
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
    ///     .at("/", handler_service(index));
    /// # }
    ///
    /// // extract u128 typed field from MyState.
    /// async fn index(StateRef(num): StateRef<'_, u128>) {
    ///     assert_eq!(*num, 996);
    /// }
    /// ```
    pub use xitca_codegen::State;
}

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
