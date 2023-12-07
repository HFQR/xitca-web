//! type extractor for value from [Extensions] and itself.

use core::{fmt, ops::Deref};

use crate::{
    body::BodyStream,
    context::WebContext,
    handler::{error::ExtractError, FromRequest},
    http::Extensions,
};

/// Extract immutable reference of element stored inside [Extensions]
pub struct ExtensionRef<'a, T>(pub &'a T);

impl<T: fmt::Debug> fmt::Debug for ExtensionRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ExtensionRef({:?})", self.0)
    }
}

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
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let ext = req
            .req()
            .extensions()
            .get::<T>()
            .ok_or(ExtractError::ExtensionNotFound)?;
        Ok(ExtensionRef(ext))
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
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let ext = req
            .req()
            .extensions()
            .get::<T>()
            .ok_or(ExtractError::ExtensionNotFound)?
            .clone();
        Ok(ExtensionOwn(ext))
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
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ExtensionsRef(req.req().extensions()))
    }
}
