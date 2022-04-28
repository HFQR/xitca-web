use alloc::boxed::Box;

use crate::BoxFuture;

use super::BuildService;

pub struct Boxed<SF> {
    factory: SF,
}

impl<SF> Boxed<SF> {
    pub(super) fn new(factory: SF) -> Self {
        Self { factory }
    }
}

impl<SF, Arg> BuildService<Arg> for Boxed<SF>
where
    SF: BuildService<Arg>,
    SF::Future: 'static,
{
    type Service = SF::Service;
    type Error = SF::Error;
    type Future = BoxFuture<'static, Self::Service, Self::Error>;

    fn build(&self, arg: Arg) -> Self::Future {
        Box::pin(self.factory.build(arg))
    }
}
