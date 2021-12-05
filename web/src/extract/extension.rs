use std::{convert::Infallible, future::Future, ops::Deref};

use crate::http::Extensions;
use xitca_http::util::service::FromRequest;

use crate::request::WebRequest;

/// Extract immutable reference of element stored inside [Extensions]
pub struct ExtensionRef<'a, T>(pub &'a T);

impl<T> Deref for ExtensionRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, 's, S, T> FromRequest<'a, &'r mut WebRequest<'s, S>> for ExtensionRef<'a, T>
where
    T: Send + Sync + 'static,
{
    type Type<'b> = ExtensionRef<'b, T>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(ExtensionRef(req.req().extensions().get::<T>().unwrap())) }
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

impl<'a, 'r, 's, S> FromRequest<'a, &'r mut WebRequest<'s, S>> for ExtensionsRef<'a> {
    type Type<'b> = ExtensionsRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(ExtensionsRef(req.req().extensions())) }
    }
}
