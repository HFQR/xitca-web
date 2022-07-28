use core::marker::PhantomData;

use alloc::boxed::Box;

use super::{
    build::{fn_build, BuildService, BuildServiceExt},
    service::Service,
    BoxFuture,
};

use self::helpers::{ServiceObject, Wrapper};

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

/// An often used type alias for [ObjectConstructor::Object] type.
pub type Object<Arg, S, E> = Box<dyn BuildService<Arg, Service = S, Error = E, Future = BoxFuture<'static, S, E>>>;

/// The most trivial [ObjectConstructor] for [BuildService] types.
///
/// Its main limitation is that the trait object is not polymorphic over `Req`.
/// So if the original service type is `impl for<'r> Service<&'r str>`,
/// the resulting object type would only be `impl Service<&'r str>`
/// for some specific lifetime `'r`.
pub struct DefaultObjectConstructor<Req, Arg>(PhantomData<(Req, Arg)>);

/// [ServiceFactory] object created by the [DefaultObjectConstructor]
pub type DefaultFactoryObject<Arg, Req, Res, Err, BErr> = Object<Arg, ServiceAlias<Req, Res, Err>, BErr>;

/// [Service] imp trait alias created by the [DefaultObjectConstructor]
pub type ServiceAlias<Req, Res, Err> = impl Service<Req, Response = Res, Error = Err>;

impl<T, Req, Arg, BErr, Res, Err> ObjectConstructor<T> for DefaultObjectConstructor<Req, Arg>
where
    T: BuildService<Arg, Error = BErr> + 'static,
    T::Service: Service<Req, Response = Res, Error = Err> + 'static,
    T::Future: 'static,
{
    type Object = DefaultFactoryObject<Arg, Req, Res, Err, BErr>;

    fn into_object(inner: T) -> Self::Object {
        let factory = fn_build(move |arg: Arg| {
            let fut = inner.build(arg);
            async move {
                let svc = Box::new(Wrapper(fut.await?)) as Box<dyn ServiceObject<Req, Response = _, Error = _>>;
                Ok::<_, BErr>(Wrapper(svc))
            }
        })
        .boxed_future();

        Box::new(factory)
    }
}

pub mod helpers {
    //! Useful types and traits for implementing a custom
    //! [ObjectConstructor](super::ObjectConstructor).

    use core::future::Future;

    use alloc::boxed::Box;

    use crate::{service::Service, BoxFuture};

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
            Box::pin(Service::call(&self.0, req))
        }
    }
}
