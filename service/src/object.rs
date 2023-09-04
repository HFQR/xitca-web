use core::{future::Future, marker::PhantomData};

use alloc::boxed::Box;

use super::{service::Service, BoxFuture};

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

impl<I, Req> Service<Req> for Box<I>
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
        ServiceObject::call(&**self, req)
    }
}

/// An often used type alias for boxed service object. used when Req type is not bound to any
/// lifetime.
pub type BoxedServiceObject<Req, Res, Err> = Box<dyn ServiceObject<Req, Response = Res, Error = Err>>;

/// An object constructor represents a one of possibly many ways to create a trait object from `I`.
///
/// A [Service] type, for example, may be type-erased into `Box<dyn Service<&'static str>>`,
/// `Box<dyn for<'a> Service<&'a str>>`, `Box<dyn Service<&'static str> + Service<u8>>`, etc.
/// Each would be a separate impl for [IntoObject].
pub trait IntoObject<I, Arg, Req>
where
    I: Service<Arg>,
    I::Response: Service<Req>,
{
    /// The type-erased form of `I`.
    type Object;

    /// Constructs `Self::Object` from `I`.
    fn into_object(inner: I) -> Self::Object;
}

/// The most trivial [IntoObject] for [Service] types.
///
/// Its main limitation is that the trait object is not polymorphic over `Req`.
/// So if the original service type is `impl for<'r> Service<&'r str>`,
/// the resulting object type would only be `impl Service<&'r str>`
/// for some specific lifetime `'r`.
pub struct DefaultObjectConstructor;

impl<T, Req, Arg, BErr, Res, Err> IntoObject<T, Arg, Req> for DefaultObjectConstructor
where
    Req: 'static,
    T: Service<Arg, Error = BErr> + 'static,
    T::Response: Service<Req, Response = Res, Error = Err> + 'static,
{
    type Object = BoxedServiceObject<Arg, BoxedServiceObject<Req, Res, Err>, BErr>;

    fn into_object(inner: T) -> Self::Object {
        struct Builder<T, Req>(T, PhantomData<Req>);

        impl<T, Req, Arg, BErr, Res, Err> Service<Arg> for Builder<T, Req>
        where
            T: Service<Arg, Error = BErr> + 'static,
            T::Response: Service<Req, Response = Res, Error = Err> + 'static,
        {
            type Response = BoxedServiceObject<Req, Res, Err>;
            type Error = BErr;
            type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

            fn call<'s>(&'s self, req: Arg) -> Self::Future<'s>
            where
                Arg: 's,
            {
                async { self.0.call(req).await.map(|s| Box::new(s) as _) }
            }
        }

        Box::new(Builder(inner, PhantomData))
    }
}
