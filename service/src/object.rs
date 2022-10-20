use core::{future::Future, marker::PhantomData};

use alloc::boxed::Box;

use super::service::Service;

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
pub type Object<Arg, S, E> = Wrapper<Box<dyn ServiceObject<Arg, Response = S, Error = E>>>;

/// The most trivial [ObjectConstructor] for [Service] types.
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
    Req: 'static,
    T: Service<Arg, Error = BErr> + 'static,
    T::Response: Service<Req, Response = Res, Error = Err> + 'static,
{
    type Object = DefaultFactoryObject<Arg, Req, Res, Err, BErr>;

    fn into_object(inner: T) -> Self::Object {
        struct DefaultObjBuilder<T, Req>(T, PhantomData<Req>);

        impl<T, Req, Arg, BErr, Res, Err> Service<Arg> for DefaultObjBuilder<T, Req>
        where
            T: Service<Arg, Error = BErr> + 'static,
            T::Response: Service<Req, Response = Res, Error = Err> + 'static,
        {
            type Response = Wrapper<Box<dyn ServiceObject<Req, Response = Res, Error = Err>>>;
            type Error = BErr;
            type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

            fn call(&self, req: Arg) -> Self::Future<'_> {
                async move {
                    let service = self.0.call(req).await?;
                    Ok(Wrapper(Box::new(service) as _))
                }
            }
        }

        Wrapper(Box::new(DefaultObjBuilder(inner, PhantomData)))
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

        fn call<'s>(&'s self, req: Req) -> BoxFuture<'s, Self::Response, Self::Error>
        where
            Req: 's;
    }

    impl<S, Req> ServiceObject<Req> for S
    where
        S: Service<Req>,
    {
        type Response = S::Response;
        type Error = S::Error;

        #[inline]
        fn call<'s>(&'s self, req: Req) -> BoxFuture<'s, Self::Response, Self::Error>
        where
            Req: 's,
        {
            Box::pin(Service::call(self, req))
        }
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
}
