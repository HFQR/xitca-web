mod and_then;
mod enclosed_fn;
mod function;
mod map;
mod map_err;

use core::{future::Future, ops::Deref, pin::Pin};

/// Extend trait for [Service](crate::Service).
///
/// Can be used to cehck the ready state of a service before calling it.
///
/// # Examples:
/// ```rust
/// # use std::{cell::Cell, rc::Rc, future::Future};
/// # use xitca_service::{Service, ready::ReadyService};
///
/// // a service with conditional availability based on state of Permit.
/// struct Foo(Permit);
///
/// // a permit reset the inner boolean to true on drop.
/// #[derive(Clone)]
/// struct Permit(Rc<Cell<bool>>);
///
/// impl Drop for Permit {
///     fn drop(&mut self) {
///         self.0.set(true);
///     }
/// }
///
/// impl Service<()> for Foo {
///     type Response = ();
///     type Error = ();
///
///     async fn call(&self, _req: ()) -> Result<Self::Response, Self::Error> {
///         Ok(())
///     }
/// }
///
/// impl ReadyService for Foo {
///     type Ready = Result<Permit, ()>;
///
///     async fn ready(&self) -> Self::Ready {
///         if self.0.0.get() {
///             // set permit to false and return with Ok<Permit>
///             self.0.0.set(false);
///             Ok(self.0.clone())
///         } else {
///             // return error is to simply the example.
///             // In real world this branch should be an async waiting for Permit reset to true.
///             Err(())
///         }                
///     }
/// }
///
/// async fn workflow(service: &Foo) {
///     let permit = service.ready().await.unwrap(); // check service ready state.
///
///     service.call(()).await.unwrap(); // run Service::call when permit is held in scope.
///
///     drop(permit); // drop permit after Service::call is finished.
/// }
///
/// async fn throttle(service: &Foo) {
///     let permit = service.ready().await.unwrap();
///     assert!(service.ready().await.is_err());  // service is throttled because permit is still held in scope.
/// }
/// ```
pub trait ReadyService {
    type Ready;

    fn ready(&self) -> impl Future<Output = Self::Ready>;
}

#[cfg(feature = "alloc")]
mod alloc_impl {
    use super::ReadyService;

    use alloc::{boxed::Box, rc::Rc, sync::Arc};

    macro_rules! impl_alloc {
        ($alloc: ident) => {
            impl<S> ReadyService for $alloc<S>
            where
                S: ReadyService + ?Sized,
            {
                type Ready = S::Ready;

                #[inline]
                async fn ready(&self) -> Self::Ready {
                    (**self).ready().await
                }
            }
        };
    }

    impl_alloc!(Box);
    impl_alloc!(Rc);
    impl_alloc!(Arc);
}

impl<S> ReadyService for Pin<S>
where
    S: Deref,
    S::Target: ReadyService,
{
    type Ready = <S::Target as ReadyService>::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.as_ref().get_ref().ready().await
    }
}
