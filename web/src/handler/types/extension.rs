//! type extractor for value from [Extensions] and itself.

use core::{fmt, ops::Deref};

use crate::{
    context::WebContext,
    error::{Error, ExtensionNotFound},
    handler::FromRequest,
    http::Extensions,
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
{
    type Type<'b> = ExtensionRef<'b, T>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        ctx.req()
            .extensions()
            .get::<T>()
            .map(ExtensionRef)
            .ok_or_else(|| Error::from_service(ExtensionNotFound::from_type::<T>()))
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
{
    type Type<'b> = ExtensionOwn<T>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        ctx.req()
            .extensions()
            .get::<T>()
            .map(|ext| ExtensionOwn(ext.clone()))
            .ok_or_else(|| Error::from_service(ExtensionNotFound::from_type::<T>()))
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

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for ExtensionsRef<'a> {
    type Type<'b> = ExtensionsRef<'b>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ExtensionsRef(ctx.req().extensions()))
    }
}
