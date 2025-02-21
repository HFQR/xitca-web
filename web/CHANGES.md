# unreleased 0.7.0
## Add
- add `HttpServer::bind_h3` easing enabling HTTP/3 as transport layer. The API can be enabled with `http3` crate feature  
- add default impl to `handler::state::BorrowState` trait for `Box`, `Rc` and `Arc` types
- add `middleware::WebContext`
- add `WebContext::extract` method
- add `service::ServeFile::new_tokio_uring` API. Guarded by `file-tokio-uring` feature
- add `Pin<&mut RequestStream>` argument to `handler::websocket::Websocket::on_close` method

## Change
- bump MSRV to `1.85` and Rust edition 2024
- change `error::Error` type by removing it's generic type param. Everywhere it had to be written as `Error<C>` can now be written as plain `Error`. Side effect of this change is how error interact with application state(typed data passed into `App::with_state` API). For most cases error type don't interact with app state at all and their impl don't need any change. But in rare case where it's needed it has to be changed in the following pattern:
  ```rust
  struct CustomError;

  // Debug, Display, Error and From impl are ignored there as they don't need change.

  // assuming app state is needed in error impl. the WebContext has to be written in the exact following type
  impl<'r> Service<WebContext<'r, xitca_web::error::Request<'r>>> for CustomError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: xitca_web::error::Request<'r>>) -> Result<Self::Response, Self::Error> {
      // xitca_web::error::Request::request_ref method is able do runtime type casting. and when the type requested
      // is the same as application's state it will return Some(app_state).
      if let Some(state) = ctx.state().request_ref::<String>() {

      }
    }
  }
  ```
- change `codegen::error_impl` marco to coop with the change to `Error` type. the impl method must receive `error::Request` type as `WebContext`'s type param. Example:
  ```rust
  struct CustomError;

  #[error_impl]
  impl CustomError {
    // remove generic types and use concrete type.
    async fn call(&self, ctx: WebContext<'_, Request<'_>>) -> WebResponse {

    }
  }
  ```
- change behavior of scoped app error handling. Back when `Error<AppState>` carries addition type info scoped app's error type has to be eagerly converted to `WebResponse` before the output goes beyond it's own scope due to possible state type difference. Since `Error` type does not carry type state param anymore scoped app's error will pass through beyond scopes unless it hits an error handler that convert it to `WebResponse`
- revert `handler::state::BorrowState` change from `0.6.2`
- update `xitca-codegen` to `0.4.0`
- update `xitca-http` to `0.7.0`
- update `xitca-service` to `0.3.0`
- update `xitca-server` to `0.5.0`
- update `http-file` to `0.2.0`

# 0.6.2
## Fix
- make default impl of `handler::state::BorrowState` forward to `core::borrow::Borrow`. enable backward compat of all previous working patterns of std types

# 0.6.1
## Change
- update `xitca-codegen` to `0.3.1`

# 0.6.0
## Fix
- fix `error::ErrorStatus::internal` wrongly create http status code `400` rather than `500`

## Add
- add `handler::state::BorrowState` trait for partial borrowing sub-typed state
- add `error::Error::upcast` method for enabling error trait upcasting without depending on nightly Rust
  ```rust
  fn handle_error<C>(err: Error<C>) {
    // upcast to trait object of std::error::Error
    let err_obj = err.upcast();
    // try to downcast object to concrete type
    if let Some(..) = err_obj.downcast_ref::<Foo>() {
      ...
    }
  }
  ```

## Remove
- remove `error::StdError` type from public API to reduce possibility of misuse
- remove `std::ops::{Deref, DerefMut}` impl for `error::Error`. `error::Error::upcast` method can be used to achieve similar effect of dereferencing

## Change
- add support for scoped application state
  ```rust
  App::new()
    .with_state(996usize) // global state is usize
    .at(
      "/foo"
      App::new()
        .with_state(996isize) // scoped state is isize
        .at("/bar", handler_service(handler))
  )

  // handler function would receive isize as typed state. overriding the global app state of usize
  async fn handler(StateRef(state): StateRef<'_, isize>) -> .. {
    assert_eq!(*state, 996isize)
  }
  ```
- use `handler::state::BorrowState` trait instead of `std::borrow::Borrow` for borrowing sub-typed state
  ```rust
  use xitca_web::handler::state::{BorrowState, StateRef};

  // arbitrary state type
  struct State {
    field: String 
  }

  // impl trait for borrowing String type from State
  impl BorrowState<String> for State {
    fn borrow(&self) -> &String {
      &self.field
    }
  }

  // handler function use &String as typed state.
  async fn handler(StateRef(state): StateRef<'_, String>) -> String {
    assert_eq!(state, "996");
    state.into()
  }

  App::new().with_state(State { field: String::from("996") }).at("/foo", handler_service(handler));
  ```
- remove generic type param from `IntoCtx` trait
- bump MSRV to `1.79`
- update `xitca-http` to `0.6.0`
- update `xitca-server` to `0.4.0`
- update `xitca-service` to `0.2.0`
- update `xitca-tls` to `0.4.0`
- update `xitca-unsafe-collection` to `0.2.0`

# 0.5.0
## Add
- add `file-raw` feature to enable file serving when no runtime is available
- add `service::file::ServeDir::with_fs` to complement with `file-raw` feature

## Change
- update `xitca-http` to `0.5.0`
- update `xitca-server` to `0.3.0`
- update `xitca-tls` to `0.3.0`

# 0.4.1
## Fix
- fix panic when using `rustls/ring` feature together with `xitca-web/rustls`

## Change
- remove direct dependent on `openssl` and `rustls` crates

# 0.4.0
## Add
- enable nightly rust `async_iterator` feature when `nightly` crate feature is active. Add `body::AsyncBody` type to bridge `futures::Stream` and `std::async_iter::AsyncIterator` traits. This change enables async generator usage. Example:
    ```rust
    // *. this is a rust 2024 edition feature.
    let body = AsyncBody::from(async gen {
        for _ in 0..1 {
            yield Ok::<_, Infallible>("async generator");
        }
    })
    ```
- add `handler::text::Text` for responder and service producing plain text http response 
- expose `error::HeaderNameNotFound` type for interacting with internal header name error
- expose `error::InvalidHeaderValue` type for interacting with internal header value error
- expose `error::ExtensionNotFound` type for interacting with extension typed map error
- expose `error::BodyOverflow` type for interacting with request body over size error

## Change
- rename `handler::string` module to `handler::text`
- `error::InvalidHeaderValue` displays it's associated header name in `Debug` and `Display` format
- update `xitca-http` to `0.4.0`
- update `xitca-server` to `0.2.0`
- update `xitca-io` to `0.2.0`
- update `xitca-codegen` to `0.2.0`

# 0.3.0
## Add
- enable Rust nightly feature `error_generic_member_access` when `xitca-web`'s `nightly` crate feature is enabled. this enables runtime context type interaction like `std::backtrace::Backtrace` for enhanced error handling
- `ErrorStatus` error type would try to capture `Backtrace`
- `Redirect`, `Html`, `Json` handler types can be used as standalone route service. Example:
    ```rust
    App::new()
        .at("/", Redirect::see_other("/index.html"))
        .at("/index.html", get(Html("<h1>Hello,World!</h1>")))
        .at("/hello.json", Json("{\"hello\":\"world!\"}"));
    ```
- add `middleware::CatchUnwind` for catching panic and keep the service running. `error::ThreadJoinError` type is added for manual error handling of `CatchUnwind`

## Remove
- remove `middleware::decompress::DecompressServiceError` type. `error::Error` is used as decompress service error type for easier error handling
- remove `error::{BadRequest, Internal}` types. `error::ErrorStatus` replace their roles where `ErrorStatus::bad_request` and `ErrorStatus::internal` would generate identical error information as `BadRequest` and `Internal` types. this change would simplify runtime error type casting a bit with two less possible error types
- remove default logger middleware from `HttpServer::{bind_xxx, listen_xxx}` APIs

## Change
- change `middleware::eraser::TypeEraser::error`'s trait bound. `From` trait is used for conversion between generic error type and `error::Error`. With this change `Error` does not double boxing itself therefore removing the need of nested type casting when handling typed error
- `ErrorStatus::{bad_request, internal}` can't be used in const context anymore as they are tasked with capture thread backtrace.
- make `middleware::logger` optional behind `logger` feature flag. extend it's construction which by default bypass manual `tracing-subscriber` import
- update `xitca-http` to `0.3.0`

# 0.2.2
## Add
- `StateRef` can used for extracting `?Sized` type from application state.

## Change
- update `xitca-http` to `0.2.1`.

# 0.2.1
## Add
- `RateLimit` middleware with optional feature `rate-limit`.
- implement `Responder` trait for `serde_json::Value`.
- re-export `http_ws::{ResponseSender, ResponseWeakSender}` types in `xitca_web::handler::websocket` module.

## Change
- `App::with_state` and `App::with_async_state` expect `Self`. Enables more flexible application state construction. Example:
    ```rust
    // delayed state attachment:
    App::new().at("/", ...).enclosed(...).with_state(996);

    // modular application configuration before attaching state:
    use xitca_web::NestApp;

    fn configure(app: NestApp<String>) -> NestApp<String> {
        app.at("/", ...)
    }

    let mut app = App::new();
    app = configure(app);
    app.with_state(String::from("996"));
    ```
- update `xitca-http` to version `0.2.0`.
- update `http-encoding` to version `0.2.0`.
- update `http-ws` to version `0.2.0`.

## Fix
- fix nested App routing. `App::new().at("/foo", App::new().at("/"))` would be successfully matching against `/foo/`
- fix bug where certain tower-http layers causing compile issue.
- fix bug where multiple tower-http layers can't be chained together with `ServiceExt::enclosed`.
