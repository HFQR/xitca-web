use std::{convert::Infallible, future::Future};

use xitca_service::BuildService;

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

impl<F, Arg> BuildService<Arg> for H3ServiceBuilder<F>
where
    F: BuildService<Arg>,
{
    type Service = H3Service<F::Service>;
    type Error = BuildError<Infallible, F::Error>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let service = self.factory.build(arg);
        async {
            let service = service.await.map_err(BuildError::Second)?;
            Ok(H3Service::new(service))
        }
    }
}
