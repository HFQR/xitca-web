use std::{convert::Infallible, future::Future};

use xitca_service::Service;

use crate::error::BuildError;

use super::service::H3Service;

/// Http/3 Builder type.
/// Take in generic types of ServiceFactory for `quinn`.
pub struct H3ServiceBuilder<F> {
    factory: F,
}

impl<F> H3ServiceBuilder<F> {
    /// Construct a new Service Builder with given service factory.
    pub fn new(factory: F) -> Self {
        Self { factory }
    }
}

impl<F, Arg> Service<Arg> for H3ServiceBuilder<F>
where
    F: Service<Arg>,
{
    type Response = H3Service<F::Response>;
    type Error = BuildError<Infallible, F::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s, 'f>(&'s self, arg: Arg) -> Self::Future<'f>
    where
        's: 'f,
        Arg: 'f,
    {
        async {
            let service = self.factory.call(arg).await.map_err(BuildError::Second)?;
            Ok(H3Service::new(service))
        }
    }
}
