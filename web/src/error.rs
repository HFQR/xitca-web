//! web error types.

use core::{
    convert::Infallible,
    fmt,
    ops::{Deref, DerefMut},
};

use std::{error, io};

pub use xitca_http::{
    error::BodyError,
    util::service::{
        route::MethodNotAllowed,
        router::{MatchError, RouterError},
    },
};

use crate::{
    bytes::Bytes,
    context::WebContext,
    http::{header::ALLOW, StatusCode, WebResponse},
    service::Service,
};

use self::service_impl::ErrorService;

type BoxErrService<C> = Box<dyn for<'r> ErrorService<WebContext<'r, C>>>;

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
///     // *. trait upcast is a feature stabled in Rust 1.76
///     // let e = &**e as &dyn error::Error;
///     // assert!(e.downcast_ref::<Foo>().is_some());
/// }
/// ```
pub struct Error<C>(BoxErrService<C>);

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

impl<C> Deref for Error<C> {
    type Target = BoxErrService<C>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<C> DerefMut for Error<C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<C> From<Infallible> for Error<C> {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

impl<C> From<io::Error> for Error<C> {
    fn from(e: io::Error) -> Self {
        Self::from_service(e)
    }
}

impl<C> From<BodyError> for Error<C> {
    fn from(e: BodyError) -> Self {
        Self::from_service(e)
    }
}

impl<'r, C> Service<WebContext<'r, C>> for Error<C> {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C>) -> Result<Self::Response, Self::Error> {
        crate::service::object::ServiceObject::call(&***self, ctx).await
    }
}

#[derive(Debug)]
pub struct Internal;

impl fmt::Display for Internal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Internal error")
    }
}

impl error::Error for Internal {}

impl<'r, C, B> Service<WebContext<'r, C, B>> for Internal {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(Bytes::new());
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        Ok(res)
    }
}

#[derive(Debug)]
pub struct BadRequest;

impl fmt::Display for BadRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Bad request")
    }
}

impl error::Error for BadRequest {}

impl<'r, C, B> Service<WebContext<'r, C, B>> for BadRequest {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(Bytes::new());
        *res.status_mut() = StatusCode::BAD_REQUEST;
        Ok(res)
    }
}

macro_rules! blank_error_service {
    ($type: ty, $status: path) => {
        impl<'r, C, B> Service<WebContext<'r, C, B>> for $type {
            type Response = WebResponse;
            type Error = Infallible;

            async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                let mut res = ctx.into_response(Bytes::new());
                *res.status_mut() = $status;
                Ok(res)
            }
        }
    };
}

blank_error_service!(MatchError, StatusCode::NOT_FOUND);
blank_error_service!(io::Error, StatusCode::INTERNAL_SERVER_ERROR);
blank_error_service!(BodyError, StatusCode::BAD_REQUEST);
blank_error_service!(Box<dyn error::Error>, StatusCode::INTERNAL_SERVER_ERROR);
blank_error_service!(Box<dyn error::Error + Send>, StatusCode::INTERNAL_SERVER_ERROR);
blank_error_service!(Box<dyn error::Error + Send + Sync>, StatusCode::INTERNAL_SERVER_ERROR);

impl<'r, C, B, E> Service<WebContext<'r, C, B>> for RouterError<E>
where
    E: for<'r2> Service<WebContext<'r2, C, B>, Response = WebResponse, Error = Infallible>,
{
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        match *self {
            Self::Match(ref e) => e.call(ctx).await,
            Self::NotAllowed(ref e) => e.call(ctx).await,
            Self::Service(ref e) => e.call(ctx).await,
        }
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for MethodNotAllowed {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(Bytes::new());

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
