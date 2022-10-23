use core::{future::Future, marker::PhantomData};

pub use helpers::{Object, ServiceObject, Wrapper};

use super::service::Service;

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

/// The most trivial [ObjectConstructor] for [Service] types.
///
/// Its main limitation is that the trait object is not polymorphic over `Req`.
/// So if the original service type is `impl for<'r> Service<&'r str>`,
/// the resulting object type would only be `impl Service<&'r str>`
/// for some specific lifetime `'r`.
pub struct DefaultObjectConstructor<Req, Arg>(PhantomData<(Req, Arg)>);

/// [ServiceObject] type created by the [DefaultObjectConstructor]
pub type DefaultObject<Arg, Req, Res, Err, BErr> = Object<Arg, ServiceAlias<Req, Res, Err>, BErr>;

/// [Service] imp trait alias created by the [DefaultObjectConstructor]
pub type ServiceAlias<Req, Res, Err> = impl Service<Req, Response = Res, Error = Err>;

impl<T, Req, Arg, BErr, Res, Err> ObjectConstructor<T> for DefaultObjectConstructor<Req, Arg>
where
    Req: 'static,
    T: Service<Arg, Error = BErr> + 'static,
    T::Response: Service<Req, Response = Res, Error = Err> + 'static,
{
    type Object = DefaultObject<Arg, Req, Res, Err, BErr>;

    fn into_object(inner: T) -> Self::Object {
        struct DefaultObjBuilder<T, Req>(T, PhantomData<Req>);

        impl<T, Req, Arg, BErr, Res, Err> Service<Arg> for DefaultObjBuilder<T, Req>
        where
            T: Service<Arg, Error = BErr> + 'static,
            T::Response: Service<Req, Response = Res, Error = Err> + 'static,
        {
            type Response = Object<Req, Res, Err>;
            type Error = BErr;
            type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

            fn call<'s, 'f>(&'s self, req: Arg) -> Self::Future<'f>
            where
                's: 'f,
                Arg: 'f,
            {
                async {
                    let service = self.0.call(req).await?;
                    Ok(Object::from_service(service))
                }
            }
        }

        Object::from_service(DefaultObjBuilder(inner, PhantomData))
    }
}

mod helpers {
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

    /// Converts between object-safe non-object-safe Service. See impls.
    pub struct Wrapper<I>(pub I);

    impl<Inner, Req> Service<Req> for Wrapper<Box<Inner>>
    where
        Inner: ServiceObject<Req> + ?Sized,
    {
        type Response = Inner::Response;
        type Error = Inner::Error;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

        #[inline]
        fn call<'s, 'f>(&'s self, req: Req) -> Self::Future<'f>
        where
            's: 'f,
            Req: 'f,
        {
            async { ServiceObject::call(&*self.0, req).await }
        }
    }

    /// An often used type alias for [super::ObjectConstructor::Object] type.
    pub type Object<Arg, S, E> = Wrapper<Box<dyn ServiceObject<Arg, Response = S, Error = E>>>;

    impl<Arg, Res, Err> Object<Arg, Res, Err> {
        pub fn from_service<I>(service: I) -> Self
        where
            I: Service<Arg, Response = Res, Error = Err> + 'static,
        {
            Wrapper(Box::new(service))
        }
    }
}
