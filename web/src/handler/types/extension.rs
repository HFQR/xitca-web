use std::{fmt, ops::Deref};

use crate::{
    body::BodyStream,
    handler::{error::ExtractError, FromRequest},
    http::Extensions,
    request::WebRequest,
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

impl<'a, 'r, C, B, T> FromRequest<'a, WebRequest<'r, C, B>> for ExtensionRef<'a, T>
where
    T: Send + Sync + 'static,
    B: BodyStream,
{
    type Type<'b> = ExtensionRef<'b, T>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebRequest<'r, C, B>) -> Result<Self, Self::Error> {
        let ext = req
            .req()
            .extensions()
            .get::<T>()
            .ok_or(ExtractError::ExtensionNotFound)?;
        Ok(ExtensionRef(ext))
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

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for ExtensionsRef<'a>
where
    B: BodyStream,
{
    type Type<'b> = ExtensionsRef<'b>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebRequest<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ExtensionsRef(req.req().extensions()))
    }
}
