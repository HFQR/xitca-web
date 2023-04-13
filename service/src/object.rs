use core::{future::Future, marker::PhantomData};

pub use helpers::{BoxedServiceObject, ServiceObject, StaticObject};

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

impl<T, Req, Arg, BErr, Res, Err> ObjectConstructor<T> for DefaultObjectConstructor<Req, Arg>
where
    Req: 'static,
    T: Service<Arg, Error = BErr> + 'static,
    T::Response: Service<Req, Response = Res, Error = Err> + 'static,
{
    type Object = StaticObject<Arg, StaticObject<Req, Res, Err>, BErr>;

    fn into_object(inner: T) -> Self::Object {
        struct DefaultObjBuilder<T, Req>(T, PhantomData<Req>);

        impl<T, Req, Arg, BErr, Res, Err> Service<Arg> for DefaultObjBuilder<T, Req>
        where
            T: Service<Arg, Error = BErr> + 'static,
            T::Response: Service<Req, Response = Res, Error = Err> + 'static,
        {
            type Response = StaticObject<Req, Res, Err>;
            type Error = BErr;
            type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

            fn call<'s>(&'s self, req: Arg) -> Self::Future<'s>
            where
                Arg: 's,
            {
                async {
                    let service = self.0.call(req).await?;
                    Ok(StaticObject::from_service(service))
                }
            }
        }

        StaticObject::from_service(DefaultObjBuilder(inner, PhantomData))
    }
}

mod helpers {
    //! Useful types and traits for implementing a custom
    //! [ObjectConstructor](super::ObjectConstructor).

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

    /// new type for `Box<dyn ServiceObject>`. used for implementing [Service] trait.
    ///
    /// For any given type impl `Service<Req>` where Req is 'static lifetime it's recommended
    /// to use [StaticObject] instead.
    ///
    /// [BoxedServiceObject] on the other hand is more useful for Req type where it has non static
    /// lifetime params like `Foo<'_>` for example. In order to express it's lifetime with HRTB
    /// currently it's (currently) necessary to use pattern as `dyn for<'i> ServiceObject<Foo<'i>>`.
    pub struct BoxedServiceObject<I: ?Sized>(pub Box<I>);

    impl<I, Req> Service<Req> for BoxedServiceObject<I>
    where
        I: ServiceObject<Req> + ?Sized,
    {
        type Response = I::Response;
        type Error = I::Error;
        type Future<'f> = BoxFuture<'f, Self::Response, Self::Error> where Self: 'f, Req: 'f;

        #[inline]
        fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
        where
            Req: 's,
        {
            ServiceObject::call(&*self.0, req)
        }
    }

    /// An often used type alias for boxed service object. used when Arg type is not bound to any
    /// lifetime.
    pub type StaticObject<Req, Res, Err> = BoxedServiceObject<dyn ServiceObject<Req, Response = Res, Error = Err>>;

    impl<Req, Res, Err> StaticObject<Req, Res, Err> {
        pub fn from_service<I>(service: I) -> Self
        where
            I: Service<Req, Response = Res, Error = Err> + 'static,
        {
            BoxedServiceObject(Box::new(service))
        }
    }
}
