pub mod middleware;

#[cfg(feature = "util-service")]
pub mod service;

pub use self::keep_alive::KeepAlive;

pub(crate) mod futures;
#[cfg(feature = "http1")]
pub(crate) mod hint;
pub(crate) mod keep_alive;

pub mod cursed {
    use std::{boxed::Box, marker::PhantomData};

    use xitca_service::{
        fn_build,
        object::{
            helpers::{ServiceObject, Wrapper},
            ObjectConstructor,
        },
        BuildService, BuildServiceExt, Service,
    };

    /// A trait that serves a type-level function to convert a type from one lifetime to another.
    ///
    /// For example: `for<'a> <&'static str as Cursed>::Type<'a> = &'a str`
    pub trait Cursed {
        /// A projection that takes `Self` as type and a given lifetime `'a`
        type Type<'a>: 'a;
    }

    /// Same as [Cursed] but works in value-level, rather than type-level.
    ///
    /// Cannot be merged with [Cursed], because of _rust issue number_.
    pub trait CursedMap: Cursed {
        /// A map that takes `Self` as a value and given lifetime, `'a`,
        /// and outputs a value of type `Self::Type<'a>`.
        fn map<'a>(self) -> Self::Type<'a>
        where
            Self: 'a;
    }

    impl<B: 'static> Cursed for http::Request<B> {
        type Type<'a> = Self;
    }
    impl<B: 'static> CursedMap for http::Request<B> {
        fn map<'a>(self) -> Self::Type<'a> {
            self
        }
    }

    impl<B: 'static> Cursed for crate::request::Request<B> {
        type Type<'a> = Self;
    }
    impl<B: 'static> CursedMap for crate::request::Request<B> {
        fn map<'a>(self) -> Self::Type<'a> {
            self
        }
    }

    /// An [object constructor](ObjectConstructor) for service with [cursed](Cursed)
    /// request types.
    pub struct CursedObjectConstructor<Req: Cursed>(PhantomData<Req>);

    pub type CursedFactoryObject<Req: Cursed, BErr, Res, Err> =
        impl BuildService<Error = BErr, Service = CursedServiceObject<Req, Res, Err>>;

    pub type CursedServiceObject<Req: Cursed, Res, Err> =
        impl for<'r> Service<Req::Type<'r>, Response = Res, Error = Err>;

    impl<I, Svc, BErr, Req, Res, Err> ObjectConstructor<I> for CursedObjectConstructor<Req>
    where
        I: BuildService<Service = Svc, Error = BErr> + 'static,
        Svc: for<'r> Service<Req::Type<'r>, Response = Res, Error = Err> + 'static,

        Req: Cursed,

        // This bound is necessary to guide type inference to infer `Req`.
        // Because we can't  provide a static guarantee to exclude
        // such rogue implementations:
        // ```
        // impl Cursed for &'_ str {
        //      type Type<'a> = &'a u8;
        // }
        // ```
        Svc: Service<Req>,
    {
        type Object = CursedFactoryObject<Req, BErr, Res, Err>;

        fn into_object(inner: I) -> Self::Object {
            let factory = fn_build(move |_arg: ()| {
                let fut = inner.build(());
                async move {
                    let boxed_service = Box::new(Wrapper(fut.await?))
                        as Box<dyn for<'r> ServiceObject<Req::Type<'r>, Response = _, Error = _>>;
                    Ok(Wrapper(boxed_service))
                }
            })
            .boxed_future();

            Box::new(factory) as Box<dyn BuildService<Service = _, Error = _, Future = _>>
        }
    }
}
