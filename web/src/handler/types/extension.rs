use std::{convert::Infallible, fmt, future::Future, ops::Deref};

use crate::{handler::FromRequest, http::Extensions, request::WebRequest};

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

impl<'a, 'r, 's, S: 's, T> FromRequest<'a, &'r mut WebRequest<'s, S>> for ExtensionRef<'a, T>
where
    T: Send + Sync + 'static,
{
    type Type<'b> = ExtensionRef<'b, T>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

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

impl<'a, 'r, 's, S: 's> FromRequest<'a, &'r mut WebRequest<'s, S>> for ExtensionsRef<'a> {
    type Type<'b> = ExtensionsRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(ExtensionsRef(req.req().extensions())) }
    }
}
