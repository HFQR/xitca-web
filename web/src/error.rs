//! web error types.
//!
//! In xitca-web error is treated as high level type and handled lazily.
//!
//! - high level:
//! An error type is represented firstly and mostly as a Rust type with useful trait bounds.It doesn't
//! necessarily mapped and/or converted into http response immediately. User is encouraged to pass the
//! error value around and convert it to http response on condition they prefer.
//!
//! - lazy:
//! Since an error is passed as value mostly the error is handled lazily when the value is needed.
//! Including but not limiting to: formatting, logging, generating http response.
//!
//! # Example
//! ```rust
//! # use xitca_web::{
//! #   error::Error,
//! #   handler::{handler_service, html::Html, Responder},
//! #   http::{StatusCode, WebResponse},
//! #   service::Service,
//! #   App, WebContext};
//! // a handler function always produce error.
//! async fn handler() -> Error {
//!     Error::from(StatusCode::BAD_REQUEST)
//! }
//!
//! // construct application with handler function and middleware.
//! App::new()
//!     .at("/", handler_service(handler))
//!     .enclosed_fn(error_handler);
//!
//! // a handler middleware observe route services output.
//! async fn error_handler<S>(service: &S, mut ctx: WebContext<'_>) -> Result<WebResponse, Error>
//! where
//!     S: for<'r> Service<WebContext<'r>, Response = WebResponse, Error = Error>
//! {
//!     // unlike WebResponse which is already a valid http response. the error is treated as it's
//!     // onw type on the other branch of the Result enum.  
//!
//!     // since the handler function at the start of example always produce error. our middleware
//!     // will always observe the Error type value so let's unwrap it.
//!     let err = service.call(ctx.reborrow()).await.err().unwrap();
//!     
//!     // now we have the error value we can start to interact with it and add our logic of
//!     // handling it.
//!
//!     // we can print the error.
//!     println!("{err}");
//!
//!     // we can log the error.
//!     tracing::error!("{err}");
//!
//!     // we can render the error to html and convert it to http response.
//!     let html = format!("<!DOCTYPE html>\
//!         <html>\
//!         <body>\
//!         <h1>{err}</h1>\
//!         </body>\
//!         </html>");
//!     return Ok(Html(html).respond(ctx).await?);
//!
//!     // or by default the error value is returned in Result::Err and passed to parent services
//!     // of App or other middlewares where eventually it would be converted to WebResponse.
//!     
//!     // "eventually" can either mean a downstream user provided error handler middleware/service
//!     // or the implicit catch all error middleware xitca-web offers. In the latter case a default
//!     // WebResponse is generated with minimal information describing the reason of error.
//!
//!     Err(err)
//! }
//! ```

use core::{
    any::Any,
    convert::Infallible,
    fmt,
    ops::{Deref, DerefMut},
};

use std::{backtrace::Backtrace, error, io, sync::Mutex};

pub use xitca_http::{
    error::BodyError,
    util::service::{
        route::MethodNotAllowed,
        router::{MatchError, RouterError},
    },
};

use crate::{
    body::ResponseBody,
    context::WebContext,
    http::{
        header::{InvalidHeaderValue, ALLOW},
        StatusCode, WebResponse,
    },
    service::{pipeline::PipelineE, Service},
};

use self::service_impl::ErrorService;

/// type erased error object. can be used for dynamic access to error's debug/display info.
/// it also support upcasting and downcasting.
///
/// # Examples:
/// ```rust
/// use std::{convert::Infallible, error, fmt};
///
/// use xitca_web::{error::Error, http::WebResponse, service::Service, WebContext};
///
/// // concrete error type
/// #[derive(Debug)]
/// struct Foo;
///
/// // implement debug and display format.
/// impl fmt::Display for Foo {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         f.write_str("Foo")
///     }
/// }
///
/// // implement Error trait
/// impl error::Error for Foo {}
///
/// // implement Service trait for http response generating.
/// impl<'r, C> Service<WebContext<'r, C>> for Foo {
///     type Response = WebResponse;
///     type Error = Infallible;
///
///     async fn call(&self, _: WebContext<'r, C>) -> Result<Self::Response, Self::Error> {
///         Ok(WebResponse::default())
///     }
/// }
///
/// async fn handle_error<C>(ctx: WebContext<'_, C>) {
///     // construct error object.
///     let e = Error::<C>::from_service(Foo);
///
///     // format and display error
///     println!("{e:?}");
///     println!("{e}");
///
///     // generate http response.
///     let res = Service::call(&e, ctx).await.unwrap();
///     assert_eq!(res.status().as_u16(), 200);
///
///     // upcast and downcast to concrete error type again.
///     // *. trait upcast is a nightly feature.
///     // see https://github.com/rust-lang/rust/issues/65991 for detail
///     
///     // let e = &*e as &dyn error::Error;
///     // assert!(e.downcast_ref::<Foo>().is_some());
/// }
/// ```
pub struct Error<C = ()>(Box<dyn for<'r> ErrorService<WebContext<'r, C>>>);

impl<C> Error<C> {
    pub fn from_service<S>(s: S) -> Self
    where
        S: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible>
            + error::Error
            + Send
            + Sync
            + 'static,
    {
        Self(Box::new(s))
    }
}

impl<C> fmt::Debug for Error<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl<C> fmt::Display for Error<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<C> error::Error for Error<C> {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.0.source()
    }

    #[cfg(feature = "nightly")]
    fn provide<'a>(&'a self, request: &mut error::Request<'a>) {
        self.0.provide(request)
    }
}

impl<C> Deref for Error<C> {
    type Target = dyn for<'r> ErrorService<WebContext<'r, C>>;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl<C> DerefMut for Error<C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl<'r, C> Service<WebContext<'r, C>> for Error<C> {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C>) -> Result<Self::Response, Self::Error> {
        crate::service::object::ServiceObject::call(self.deref(), ctx).await
    }
}

macro_rules! error_from_service {
    ($tt: ty) => {
        impl<C> From<$tt> for crate::error::Error<C> {
            fn from(e: $tt) -> Self {
                Self::from_service(e)
            }
        }
    };
}

pub(crate) use error_from_service;

macro_rules! blank_error_service {
    ($type: ty, $status: path) => {
        impl<'r, C, B> Service<WebContext<'r, C, B>> for $type {
            type Response = WebResponse;
            type Error = Infallible;

            async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                let mut res = ctx.into_response(ResponseBody::empty());
                *res.status_mut() = $status;
                Ok(res)
            }
        }
    };
}

macro_rules! forward_blank_internal {
    ($type: ty) => {
        impl<'r, C, B> crate::service::Service<WebContext<'r, C, B>> for $type {
            type Response = WebResponse;
            type Error = Infallible;

            async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                crate::http::StatusCode::INTERNAL_SERVER_ERROR.call(ctx).await
            }
        }
    };
}

pub(crate) use forward_blank_internal;

macro_rules! forward_blank_bad_request {
    ($type: ty) => {
        impl<'r, C, B> crate::service::Service<WebContext<'r, C, B>> for $type {
            type Response = WebResponse;
            type Error = ::core::convert::Infallible;

            async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                crate::http::StatusCode::BAD_REQUEST.call(ctx).await
            }
        }
    };
}

pub(crate) use forward_blank_bad_request;

impl<C> From<Infallible> for Error<C> {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for Infallible {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, _: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        unreachable!()
    }
}

/// error type derive from http status code. produce minimal "StatusCode Reason" response and stack backtrace
/// of the location status code error occurs.
pub struct ErrorStatus {
    status: StatusCode,
    _back_trace: Backtrace,
}

impl ErrorStatus {
    /// construct an ErrorStatus type from [`StatusCode::INTERNAL_SERVER_ERROR`]
    pub fn internal() -> Self {
        // verbosity of constructor is desired here so back trace capture
        // can direct capture the call site.
        Self {
            status: StatusCode::BAD_REQUEST,
            _back_trace: Backtrace::capture(),
        }
    }

    /// construct an ErrorStatus type from [`StatusCode::BAD_REQUEST`]
    pub fn bad_request() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            _back_trace: Backtrace::capture(),
        }
    }
}

impl fmt::Debug for ErrorStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.status, f)
    }
}

impl fmt::Display for ErrorStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.status, f)
    }
}

impl error::Error for ErrorStatus {
    #[cfg(feature = "nightly")]
    fn provide<'a>(&'a self, request: &mut error::Request<'a>) {
        request.provide_ref(&self._back_trace);
    }
}

impl From<StatusCode> for ErrorStatus {
    fn from(status: StatusCode) -> Self {
        Self {
            status,
            _back_trace: Backtrace::capture(),
        }
    }
}

impl<C> From<StatusCode> for Error<C> {
    fn from(e: StatusCode) -> Self {
        Error::from(ErrorStatus::from(e))
    }
}

impl<C> From<ErrorStatus> for Error<C> {
    fn from(e: ErrorStatus) -> Self {
        Error::from_service(e)
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for ErrorStatus {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.status.call(ctx).await
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for StatusCode {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(ResponseBody::empty());
        *res.status_mut() = *self;
        Ok(res)
    }
}

error_from_service!(io::Error);
forward_blank_internal!(io::Error);

error_from_service!(MatchError);
blank_error_service!(MatchError, StatusCode::NOT_FOUND);

error_from_service!(MethodNotAllowed);

error_from_service!(InvalidHeaderValue);
forward_blank_bad_request!(InvalidHeaderValue);

impl<'r, C, B> Service<WebContext<'r, C, B>> for MethodNotAllowed {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(ResponseBody::empty());

        let allowed = self.allowed_methods();

        let len = allowed.iter().fold(0, |a, m| a + m.as_str().len() + 1);

        let mut methods = String::with_capacity(len);

        for method in allowed {
            methods.push_str(method.as_str());
            methods.push(',');
        }
        methods.pop();

        res.headers_mut().insert(ALLOW, methods.parse().unwrap());
        *res.status_mut() = StatusCode::METHOD_NOT_ALLOWED;

        Ok(res)
    }
}

impl<E, C> From<RouterError<E>> for Error<C>
where
    E: Into<Self>,
{
    fn from(e: RouterError<E>) -> Self {
        match e {
            RouterError::Match(e) => e.into(),
            RouterError::NotAllowed(e) => e.into(),
            RouterError::Service(e) => e.into(),
        }
    }
}

type StdErr = Box<dyn error::Error + Send + Sync>;

impl<C> From<StdErr> for Error<C> {
    fn from(e: StdErr) -> Self {
        Self(Box::new(StdError(e)))
    }
}

forward_blank_internal!(StdErr);

/// new type for `Box<dyn std::error::Error + Send + Sync>`. produce minimal
/// "500 InternalServerError" response and forward formatting, error handling
/// to inner type.
///
/// In other word it's an error type keep it's original formatting and error
/// handling methods without a specific `Service` impl for generating complex
/// http response.
pub struct StdError(pub StdErr);

impl fmt::Debug for StdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for StdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl error::Error for StdError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.0.source()
    }

    #[cfg(feature = "nightly")]
    fn provide<'a>(&'a self, request: &mut error::Request<'a>) {
        self.0.provide(request);
    }
}

error_from_service!(StdError);

impl<'r, C, B> Service<WebContext<'r, C, B>> for StdError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.0.call(ctx).await
    }
}

/// error happens when joining a thread. typically caused by code panic inside thread.
/// [`CatchUnwind`] middleware is able to produce this error type.
///
/// # Examples:
/// ```rust
/// # use xitca_web::error::ThreadJoinError;
/// fn handle_error(e: &ThreadJoinError) {
///     // debug and display format thread join error. can only provide basic error message if the error
///     // source is typical string.(for example generated by panic! macro or unwrap/expect methods)
///     println!("{e:?}");
///     println!("{e}");
///
///     // for arbitrary thread join error manual type downcast is needed.(for example generated by std::panic::panic_any)
///     // the mutex lock inside is to satisfy xitca-web's error type's thread safe guarantee: Send and Sync auto traits.
///     //
///     // rust's std library only offers Send bound for thread join error and the mutex is solely for the purpose of making
///     // the error bound to Send + Sync.
///     let any = e.0.lock().unwrap();
///
///     // an arbitrary type we assume possibly being used as panic message.
///     struct Foo;
///
///     if let Some(_foo) = any.downcast_ref::<Foo>() {
///         // if downcast is succeed it's possible to handle the typed panic message.
///     }
///
///     // otherwise there is basically no way to retrieve any meaningful information and it's best to just ignore the error.
///     // xitca-web is able to generate minimal http response from it anyway.
/// }
/// ```
///
/// [`CatchUnwind`]: crate::middleware::CatchUnwind
pub struct ThreadJoinError(pub Mutex<Box<dyn Any + Send>>);

impl fmt::Debug for ThreadJoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThreadJoinError").finish()
    }
}

impl fmt::Display for ThreadJoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        {
            let any = self.0.lock().unwrap();
            // only try to catch typical panic message. currently the cases covered are
            // format string and string reference generated by panic! macro.
            if let Some(msg) = any.downcast_ref::<String>() {
                return write!(f, "error joining thread: {msg}");
            }
            if let Some(msg) = any.downcast_ref::<&str>() {
                return write!(f, "error joining thread: {msg}");
            }
        }

        // arbitrary panic message type has to be handled by user manually.
        f.write_str("error joining thread: unknown. please consider downcast ThreadJoinError.0")
    }
}

impl error::Error for ThreadJoinError {}

impl ThreadJoinError {
    pub(crate) fn new(e: Box<dyn Any + Send>) -> Self {
        Self(Mutex::new(e))
    }
}

error_from_service!(ThreadJoinError);
forward_blank_internal!(ThreadJoinError);

impl<F, S, C> From<PipelineE<F, S>> for Error<C>
where
    F: Into<Error<C>>,
    S: Into<Error<C>>,
{
    fn from(pipe: PipelineE<F, S>) -> Self {
        match pipe {
            PipelineE::First(f) => f.into(),
            PipelineE::Second(s) => s.into(),
        }
    }
}

mod service_impl {
    use crate::service::object::ServiceObject;

    use super::*;

    pub trait ErrorService<Req>:
        ServiceObject<Req, Response = WebResponse, Error = Infallible> + error::Error + Send + Sync
    {
    }

    impl<S, Req> ErrorService<Req> for S where
        S: ServiceObject<Req, Response = WebResponse, Error = Infallible> + error::Error + Send + Sync
    {
    }
}

#[cfg(test)]
mod test {
    use core::fmt;

    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::body::ResponseBody;

    use super::*;

    #[test]
    fn cast() {
        #[derive(Debug)]
        struct Foo;

        impl fmt::Display for Foo {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("Foo")
            }
        }

        impl error::Error for Foo {}

        impl<'r, C> Service<WebContext<'r, C>> for Foo {
            type Response = WebResponse;
            type Error = Infallible;

            async fn call(&self, _: WebContext<'r, C>) -> Result<Self::Response, Self::Error> {
                Ok(WebResponse::new(ResponseBody::None))
            }
        }

        let foo = Error::<()>::from_service(Foo);

        println!("{foo:?}");
        println!("{foo}");

        let mut ctx = WebContext::new_test(());
        let res = Service::call(&foo, ctx.as_web_ctx()).now_or_panic().unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }
}
