//! type extractor for value from [Extensions] and itself.

use core::{fmt, ops::Deref};

use std::{convert::Infallible, error};

use crate::{
    body::BodyStream,
    context::WebContext,
    dev::service::Service,
    error::Error,
    handler::{error::blank_bad_request, FromRequest},
    http::{Extensions, WebResponse},
};

/// Extract immutable reference of element stored inside [Extensions]
#[derive(Debug)]
pub struct ExtensionRef<'a, T>(pub &'a T);

impl<T> Deref for ExtensionRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for ExtensionRef<'a, T>
where
    T: Send + Sync + 'static,
    B: BodyStream,
{
    type Type<'b> = ExtensionRef<'b, T>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        ctx.req()
            .extensions()
            .get::<T>()
            .map(ExtensionRef)
            .ok_or_else(|| ExtensionNotFound.into())
    }
}

/// Extract owned type stored inside [Extensions]
pub struct ExtensionOwn<T>(pub T);

impl<T: fmt::Debug> fmt::Debug for ExtensionOwn<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ExtensionOwn({:?})", self.0)
    }
}

impl<T> Deref for ExtensionOwn<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for ExtensionOwn<T>
where
    T: Send + Sync + Clone + 'static,
    B: BodyStream,
{
    type Type<'b> = ExtensionOwn<T>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        ctx.req()
            .extensions()
            .get::<T>()
            .map(|ext| ExtensionOwn(ext.clone()))
            .ok_or_else(|| ExtensionNotFound.into())
    }
}

/// Extract immutable reference of the [Extensions].
pub struct ExtensionsRef<'a>(pub &'a Extensions);

impl Deref for ExtensionsRef<'_> {
    type Target = Extensions;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for ExtensionsRef<'a>
where
    B: BodyStream,
{
    type Type<'b> = ExtensionsRef<'b>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ExtensionsRef(ctx.req().extensions()))
    }
}

/// Absent type of request's (Extensions)[crate::http::Extensions] type map.
#[derive(Debug)]
pub struct ExtensionNotFound;

impl fmt::Display for ExtensionNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Extension data type can not be found")
    }
}

impl error::Error for ExtensionNotFound {}

impl<'r, C, B> Service<WebContext<'r, C, B>> for ExtensionNotFound {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Ok(blank_bad_request(ctx))
    }
}
