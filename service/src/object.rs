use alloc::boxed::Box;
use core::marker::PhantomData;

use crate::{fn_factory, Service, ServiceFactory};

use self::helpers::{ServiceFactoryObject, ServiceObject, Wrapper};

/// An object constructor represents a one of possibly many ways to create a trait object from `I`.
///
/// A [Service] type, for example, may be type-erased into `Box<dyn Service<&'static str>>`,
/// `Box<dyn for<'a> Service<&'a str>>`, `Box<dyn Service<&'static str> + Service<u8>>`, etc.
/// Each would be a separate impl for [ObjectConstructor].
pub trait ObjectConstructor<I> {
    /// The type-erased form of `I`.
    type Object;

    /// Constructs `Self::Object` from `I`.
    fn into_object(inner: I) -> Self::Object;
}

/// The most trivial [ObjectConstructor] for [ServiceFactory] types.
///
/// Its main limitation is that the trait object is not polymorphic over `Req`.
/// So if the original service type is `impl for<'r> Service<&'r str>`,
/// the resulting object type would only be `impl Service<&'r str>`
/// for some specific lifetime `'r`.
pub struct DefaultObjectConstructor<Req, Arg>(PhantomData<(Req, Arg)>);

/// [ServiceFactory] object created by the [DefaultObjectConstructor]
pub type DefaultFactoryObject<Req, Arg, Res, Err> =
    impl ServiceFactory<Req, Arg, Response = Res, Error = Err, Service = DefaultServiceObject<Req, Res, Err>>;

/// [Service] object created by the [DefaultObjectConstructor]
pub type DefaultServiceObject<Req, Res, Err> = impl Service<Req, Response = Res, Error = Err>;

impl<T, Req, Arg, Res, Err> ObjectConstructor<T> for DefaultObjectConstructor<Req, Arg>
where
    T: ServiceFactory<Req, Arg, Response = Res, Error = Err> + 'static,
    T::Service: 'static,
    T::Future: 'static,
{
    type Object = DefaultFactoryObject<Req, Arg, Res, Err>;

    fn into_object(inner: T) -> Self::Object {
        let factory = fn_factory(move |arg: Arg| {
            let fut = inner.new_service(arg);
            async move {
                let svc = Box::new(Wrapper(fut.await?)) as Box<dyn ServiceObject<Req, Response = _, Error = _>>;
                Ok(Wrapper(svc))
            }
        });

        Wrapper(Box::new(Wrapper(factory)) as Box<dyn ServiceFactoryObject<Req, Arg, Service = _>>)
    }
}

pub mod helpers {
    //! Useful types and traits for implementing a custom
    //! [ObjectConstructor](super::ObjectConstructor).

    use alloc::boxed::Box;
    use core::future::Future;

    use crate::{BoxFuture, Service, ServiceFactory};

    /// Object-safe counterpart to [ServiceFactory].
    pub trait ServiceFactoryObject<Req, Arg = ()> {
        type Service: Service<Req>;

        fn new_service(&self, arg: Arg) -> BoxFuture<'static, Self::Service, <Self::Service as Service<Req>>::Error>;
    }

    /// Object-safe counterpart of [Service].
    pub trait ServiceObject<Req> {
        type Response;
        type Error;

        fn call<'s, 'f>(&'s self, req: Req) -> BoxFuture<'f, Self::Response, Self::Error>
        where
            Req: 'f,
            's: 'f;
    }

    /// Converts between object-safe non-object-safe Service and ServiceFactory. See impls.
    pub struct Wrapper<I>(pub I);

    impl<Inner, Req, Arg> ServiceFactory<Req, Arg> for Wrapper<Box<Inner>>
    where
        Inner: ServiceFactoryObject<Req, Arg> + ?Sized,
    {
        type Response = <Inner::Service as Service<Req>>::Response;
        type Error = <Inner::Service as Service<Req>>::Error;
        type Service = Inner::Service;
        type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

        fn new_service(&self, arg: Arg) -> Self::Future {
            ServiceFactoryObject::new_service(&*self.0, arg)
        }
    }

    impl<Inner, Req, Arg, Err> ServiceFactoryObject<Req, Arg> for Wrapper<Inner>
    where
        Inner: ServiceFactory<Req, Arg, Error = Err>,
        Inner::Future: 'static,
    {
        type Service = Inner::Service;

        fn new_service(&self, arg: Arg) -> BoxFuture<'static, Self::Service, Err> {
            let fut = ServiceFactory::new_service(&self.0, arg);
            Box::pin(fut)
        }
    }

    impl<Inner, Req> Service<Req> for Wrapper<Box<Inner>>
    where
        Inner: ServiceObject<Req> + ?Sized,
    {
        type Response = Inner::Response;
        type Error = Inner::Error;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

        #[inline]
        fn call(&self, req: Req) -> Self::Future<'_> {
            async move { ServiceObject::call(&*self.0, req).await }
        }
    }

    impl<Inner, Req> ServiceObject<Req> for Wrapper<Inner>
    where
        Inner: Service<Req>,
    {
        type Response = Inner::Response;
        type Error = Inner::Error;

        #[inline]
        fn call<'s, 'f>(&'s self, req: Req) -> BoxFuture<'f, Inner::Response, Inner::Error>
        where
            Req: 'f,
            's: 'f,
        {
            Box::pin(async move { Service::call(&self.0, req).await })
        }
    }
}
