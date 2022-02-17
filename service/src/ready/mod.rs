mod and_then;
mod function;
mod map;
mod map_err;
mod pipeline;
mod transform_fn;

use core::{future::Future, ops::Deref, pin::Pin};

use alloc::{boxed::Box, rc::Rc, sync::Arc};

use super::service::Service;

pub trait ReadyService<Req>: Service<Req> {
    type Ready;

    type ReadyFuture<'f>: Future<Output = Result<Self::Ready, Self::Error>>
    where
        Self: 'f;

    fn ready(&self) -> Self::ReadyFuture<'_>;
}

macro_rules! impl_alloc {
    ($alloc: ident) => {
        impl<S, Req> ReadyService<Req> for $alloc<S>
        where
            S: ReadyService<Req> + ?Sized,
        {
            type Ready = S::Ready;
            type ReadyFuture<'f>
            where
                Self: 'f,
            = S::ReadyFuture<'f>;

            #[inline]
            fn ready(&self) -> Self::ReadyFuture<'_> {
                (**self).ready()
            }
        }
    };
}

impl_alloc!(Box);
impl_alloc!(Rc);
impl_alloc!(Arc);

impl<S, Req> ReadyService<Req> for Pin<S>
where
    S: Deref,
    S::Target: ReadyService<Req>,
{
    type Ready = <S::Target as ReadyService<Req>>::Ready;
    type ReadyFuture<'f>
    where
        Self: 'f,
    = <S::Target as ReadyService<Req>>::ReadyFuture<'f>;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.as_ref().get_ref().ready()
    }
}
