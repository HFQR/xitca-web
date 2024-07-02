# unreleased 0.6.0
## Add
- add `handler::state::BorrowState` trait for partial borrowing sub-typed state

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
