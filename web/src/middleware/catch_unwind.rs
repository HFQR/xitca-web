//! panic catcher middleware.

use xitca_http::util::middleware::catch_unwind::{self, CatchUnwindError};

use crate::{
    WebContext,
    error::{Error, ThreadJoinError},
    service::{Service, ready::ReadyService},
};

/// middleware for catching panic inside [`Service::call`] and return a 500 error response.
///
/// # Examples:
/// ```rust
/// # use xitca_web::{handler::handler_service, middleware::CatchUnwind, service::ServiceExt, App, WebContext};
/// // handler function that cause panic.
/// async fn handler(_: &WebContext<'_>) -> &'static str {
///     panic!("");
/// }
///
/// App::new()
///     // request to "/" would always panic due to handler function.
///     .at("/", handler_service(handler))
///     // enclosed application with CatchUnwind middleware.
///     // panic in handler function would be caught and converted to 500 internal server error response to client.
///     .enclosed(CatchUnwind);
///
/// // CatchUnwind can also be used on individual route service for scoped panic catching:
/// App::new()
///     .at("/", handler_service(handler))
///     // only catch panic on "/scope" path.
///     .at("/scope", handler_service(handler).enclosed(CatchUnwind));
/// ```
#[derive(Clone)]
pub struct CatchUnwind;

impl<Arg> Service<Arg> for CatchUnwind
where
    catch_unwind::CatchUnwind: Service<Arg>,
{
    type Response = CatchUnwindService<<catch_unwind::CatchUnwind as Service<Arg>>::Response>;
    type Error = <catch_unwind::CatchUnwind as Service<Arg>>::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        catch_unwind::CatchUnwind.call(arg).await.map(CatchUnwindService)
    }
}

pub struct CatchUnwindService<S>(S);

impl<'r, C, B, S> Service<WebContext<'r, C, B>> for CatchUnwindService<S>
where
    S: Service<WebContext<'r, C, B>>,
    S::Error: Into<Error>,
{
    type Response = S::Response;
    type Error = Error;

    #[inline]
    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.0.call(ctx).await.map_err(Into::into)
    }
}

impl<E> From<CatchUnwindError<E>> for Error
where
    E: Into<Error>,
{
    fn from(e: CatchUnwindError<E>) -> Self {
        match e {
            CatchUnwindError::First(e) => Error::from(ThreadJoinError::new(e)),
            CatchUnwindError::Second(e) => e.into(),
        }
    }
}

impl<S> ReadyService for CatchUnwindService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.0.ready().await
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        App,
        handler::handler_service,
        http::{Request, StatusCode},
    };

    use super::*;

    #[test]
    fn catch_panic() {
        async fn handler() -> &'static str {
            panic!("");
        }

        let res = App::new()
            .with_state("996")
            .at("/", handler_service(handler))
            .enclosed(CatchUnwind)
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
