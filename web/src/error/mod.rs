//! web error types.
//!
//! In xitca-web error is treated as high level type and handled lazily.
//!
//! - high level:
//!   An error type is represented firstly and mostly as a Rust type with useful trait bounds.It doesn't
//!   necessarily mapped and/or converted into http response immediately. User is encouraged to pass the
//!   error value around and convert it to http response on condition they prefer.
//!
//! - lazy:
//!   Since an error is passed as value mostly the error is handled lazily when the value is needed.
//!   Including but not limiting to: formatting, logging, generating http response.
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
//!     return (Html(html), StatusCode::BAD_REQUEST).respond(ctx).await;
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

mod body;
mod extension;
mod header;
mod router;
mod status;

pub use body::*;
pub use extension::*;
pub use header::*;
pub use router::*;
pub use status::*;

use core::{any::Any, convert::Infallible, fmt};

use std::{error, io, sync::Mutex};

use crate::{
    context::WebContext,
    http::WebResponse,
    service::{Service, pipeline::PipelineE},
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
/// async fn handle_error<C>(ctx: WebContext<'_, C>) where C: 'static {
///     // construct error object.
///     let e = Error::from_service(Foo);
///
///     // format and display error
///     println!("{e:?}");
///     println!("{e}");
///
///     // generate http response.
///     let res = Service::call(&e, ctx).await.unwrap();
///     assert_eq!(res.status().as_u16(), 200);
///
///     // upcast error to trait object of std::error::Error
///     let e = e.upcast();
///     
///     // downcast error object to concrete type again
///     assert!(e.downcast_ref::<Foo>().is_some());
/// }
/// ```
pub struct Error(Box<dyn for<'r> ErrorService<WebContext<'r, Request<'r>>>>);

// TODO: this is a temporary type to mirror std::error::request_ref API and latter should
// be used when it's stabled.
/// container for dynamic type provided by Error's default Service impl
pub struct Request<'a> {
    inner: &'a dyn Any,
}

impl Request<'_> {
    /// request a reference of concrete type from dynamic container.
    /// [Error] would provide your application's global state to it.
    ///
    /// # Examples
    /// ```rust
    /// use std::{convert::Infallible, fmt};
    ///
    /// use xitca_web::{
    ///     error::{Error, Request},
    ///     handler::handler_service,
    ///     http::WebResponse,
    ///     service::Service,
    ///     App, WebContext
    /// };
    ///
    /// let app = App::new()
    ///     .at("/", handler_service(handler))
    ///     .with_state(String::from("996")); // application has a root state of String type.
    ///
    /// // handler function returns custom error type
    /// async fn handler(_: &WebContext<'_, String>) -> Result<&'static str, MyError> {
    ///     Err(MyError)
    /// }
    ///
    /// // a self defined error type and necessary error implements.
    /// #[derive(Debug)]
    /// struct MyError;
    ///
    /// impl fmt::Display for MyError {
    ///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    ///         f.write_str("my error")
    ///     }
    /// }
    ///
    /// impl std::error::Error for MyError {}
    ///
    /// impl From<MyError> for Error {
    ///     fn from(e: MyError) -> Self {
    ///         Self::from_service(e)
    ///     }
    /// }
    ///
    /// // Assuming application state is needed in error handler then this is the Service impl
    /// // you want to write
    /// impl<'r> Service<WebContext<'r, Request<'r>>> for MyError {
    ///     type Response = WebResponse;
    ///     type Error = Infallible;
    ///
    ///     async fn call(&self, ctx: WebContext<'r, Request<'r>>) -> Result<Self::Response, Self::Error> {
    ///         // error::Request is able to provide application's state reference with runtime type casting.
    ///         if let Some(state) = ctx.state().request_ref::<String>() {
    ///             assert_eq!(state, "996");
    ///         }
    ///         todo!()
    ///     }
    /// }
    /// ```
    pub fn request_ref<C>(&self) -> Option<&C>
    where
        C: 'static,
    {
        self.inner.downcast_ref()
    }
}

impl Error {
    // construct an error object from given service type.
    pub fn from_service<S>(s: S) -> Self
    where
        S: for<'r> Service<WebContext<'r, Request<'r>>, Response = WebResponse, Error = Infallible>
            + error::Error
            + Send
            + Sync
            + 'static,
    {
        Self(Box::new(s))
    }

    /// upcast Error to trait object for advanced error handling.
    /// See [std::error::Error] for usage
    pub fn upcast(&self) -> &(dyn error::Error + 'static) {
        let e = self.0.dyn_err();
        // due to Rust's specialization limitation Box<dyn std::error::Error> can impl neither
        // std::error::Error nor service_impl::DynError traits. Therefore a StdError new type
        // wrapper is introduced to work around it. When upcasting the error this new type is manually
        // removed so the trait object have correct std::error::Error trait impl for the inner
        // type
        if let Some(e) = e.downcast_ref::<StdError>() {
            return &*e.0;
        }
        e
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.0.source()
    }

    #[cfg(feature = "nightly")]
    fn provide<'a>(&'a self, request: &mut error::Request<'a>) {
        self.0.provide(request)
    }
}

impl<'r, C> Service<WebContext<'r, C>> for Error
where
    C: 'static,
{
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C>) -> Result<Self::Response, Self::Error> {
        let WebContext { req, body, ctx } = ctx;
        crate::service::object::ServiceObject::call(
            &self.0,
            WebContext {
                req,
                body,
                ctx: &Request { inner: ctx as _ },
            },
        )
        .await
    }
}

macro_rules! error_from_service {
    ($tt: ty) => {
        impl From<$tt> for crate::error::Error {
            fn from(e: $tt) -> Self {
                Self::from_service(e)
            }
        }
    };
}

pub(crate) use error_from_service;

macro_rules! blank_error_service {
    ($type: ty, $status: path) => {
        impl<'r, C, B> crate::service::Service<crate::WebContext<'r, C, B>> for $type {
            type Response = crate::http::WebResponse;
            type Error = ::core::convert::Infallible;

            async fn call(&self, ctx: crate::WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                let mut res = ctx.into_response(crate::body::ResponseBody::empty());
                *res.status_mut() = $status;
                Ok(res)
            }
        }
    };
}

pub(crate) use blank_error_service;

macro_rules! forward_blank_internal {
    ($type: ty) => {
        impl<'r, C, B> crate::service::Service<crate::WebContext<'r, C, B>> for $type {
            type Response = crate::http::WebResponse;
            type Error = ::core::convert::Infallible;

            async fn call(&self, ctx: crate::WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                crate::http::StatusCode::INTERNAL_SERVER_ERROR.call(ctx).await
            }
        }
    };
}

pub(crate) use forward_blank_internal;

macro_rules! forward_blank_bad_request {
    ($type: ty) => {
        impl<'r, C, B> crate::service::Service<crate::WebContext<'r, C, B>> for $type {
            type Response = crate::http::WebResponse;
            type Error = ::core::convert::Infallible;

            async fn call(&self, ctx: crate::WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                crate::http::StatusCode::BAD_REQUEST.call(ctx).await
            }
        }
    };
}

pub(crate) use forward_blank_bad_request;

impl From<Infallible> for Error {
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

error_from_service!(io::Error);
forward_blank_internal!(io::Error);

type StdErr = Box<dyn error::Error + Send + Sync>;

impl From<StdErr> for Error {
    fn from(e: StdErr) -> Self {
        // this is a hack for middleware::Limit where it wraps around request stream body
        // and produce BodyOverFlow error and return it as BodyError. In the mean time
        // BodyError is another type alias share the same real type of StdErr and both share
        // the same conversion path when converting into Error.
        //
        // currently the downcast and clone is to restore BodyOverFlow's original Service impl
        // where it will produce 400 bad request http response while StdErr will be producing
        // 500 internal server error http response. As well as restoring downstream Error
        // consumer's chance to downcast BodyOverFlow type.
        //
        // TODO: BodyError type should be replaced with Error in streaming interface.
        if let Some(e) = e.downcast_ref::<BodyOverFlow>() {
            return Self::from(e.clone());
        }

        Self(Box::new(StdError(e)))
    }
}

forward_blank_internal!(StdErr);

/*
    new type for `Box<dyn std::error::Error + Send + Sync>`. produce minimal
    "500 InternalServerError" response and forward formatting, error handling
    to inner type.
    In other words it's an error type keep it's original formatting and error
    handling methods without a specific `Service` impl for generating custom
    http response.
*/
struct StdError(StdErr);

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
        let any = self.0.lock().unwrap();

        // only try to catch typical panic message. currently the cases covered are
        // format string and string reference generated by panic! macro.
        any.downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| any.downcast_ref::<&str>().copied())
            .map(|msg| write!(f, "error joining thread: {msg}"))
            // arbitrary panic message type has to be handled by user manually.
            .unwrap_or_else(|| f.write_str("error joining thread: unknown. please consider downcast ThreadJoinError.0"))
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

impl<F, S> From<PipelineE<F, S>> for Error
where
    F: Into<Error>,
    S: Into<Error>,
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

    /// helper trait for constraint error object to multiple bounds
    pub trait ErrorService<Req>:
        ServiceObject<Req, Response = WebResponse, Error = Infallible> + DynError + Send + Sync
    {
    }

    impl<S, Req> ErrorService<Req> for S where
        S: ServiceObject<Req, Response = WebResponse, Error = Infallible> + DynError + Send + Sync
    {
    }

    /// helper trait for enabling error trait upcast without depending on nightly rust feature
    /// (written when project MSRV is 1.79)
    pub trait DynError: error::Error {
        fn dyn_err(&self) -> &(dyn error::Error + 'static);
    }

    impl<E> DynError for E
    where
        E: error::Error + 'static,
    {
        fn dyn_err(&self) -> &(dyn error::Error + 'static) {
            self
        }
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{body::ResponseBody, http::StatusCode};

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
                Ok(WebResponse::new(ResponseBody::none()))
            }
        }

        let foo = Error::from_service(Foo);

        println!("{foo:?}");
        println!("{foo}");

        let mut ctx = WebContext::new_test(());
        let res = Service::call(&foo, ctx.as_web_ctx()).now_or_panic().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let err = Error::from(Box::new(Foo) as Box<dyn std::error::Error + Send + Sync>);

        println!("{err:?}");
        println!("{err}");

        assert!(err.upcast().downcast_ref::<Foo>().is_some());

        #[derive(Debug)]
        struct Bar;

        impl fmt::Display for Bar {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("Foo")
            }
        }

        impl error::Error for Bar {}

        impl<'r> Service<WebContext<'r, Request<'r>>> for Bar {
            type Response = WebResponse;
            type Error = Infallible;

            async fn call(&self, ctx: WebContext<'r, Request<'r>>) -> Result<Self::Response, Self::Error> {
                let status = ctx.state().request_ref::<StatusCode>().unwrap();
                Ok(WebResponse::builder().status(*status).body(Default::default()).unwrap())
            }
        }

        let bar = Error::from_service(Bar);

        let res = bar
            .call(WebContext::new_test(StatusCode::IM_USED).as_web_ctx())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status(), StatusCode::IM_USED);
    }
}
