mod and_then;
mod enclosed;
mod ext;
mod function;
mod map;
mod map_err;
mod opt;

pub use self::{
    ext::ServiceExt,
    function::{FnService, fn_build, fn_service},
};

use core::{future::Future, ops::Deref, pin::Pin};

/// Trait for simulate `Fn<(&Self, Arg)> -> impl Future<Output = Result<T, E>> + '_`.
/// The function call come from stateful type that can be referenced within returned opaque future.
pub trait Service<Req = ()> {
    /// The Ok part of output future.
    type Response;

    /// The Err part of output future.
    type Error;

    fn call(&self, req: Req) -> impl Future<Output = Result<Self::Response, Self::Error>>;
}

impl<S, Req> Service<Req> for Pin<S>
where
    S: Deref,
    S::Target: Service<Req>,
{
    type Response = <S::Target as Service<Req>>::Response;
    type Error = <S::Target as Service<Req>>::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.as_ref().get_ref().call(req).await
    }
}

#[cfg(feature = "alloc")]
impl<S, Req> Service<Req> for alloc::sync::Arc<S>
where
    S: Service<Req> + ?Sized,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        Service::call(&**self, req).await
    }
}

#[cfg(feature = "alloc")]
#[cfg(test)]
mod test {
    use super::*;

    use alloc::string::String;

    use xitca_unsafe_collection::futures::NowOrPanic;

    struct Layer1<S> {
        name: String,
        service: S,
    }

    struct Layer2<S> {
        name: String,
        service: S,
    }

    impl<'r, S, Res, Err> Service<&'r str> for Layer1<S>
    where
        S: for<'r2> Service<(&'r2 str, &'r2 str), Response = Res, Error = Err> + 'static,
    {
        type Response = Res;
        type Error = Err;

        async fn call(&self, req: &'r str) -> Result<Self::Response, Self::Error> {
            let req = (req, self.name.as_str());
            self.service.call(req).await
        }
    }

    impl<'r, S, Res, Err> Service<(&'r str, &'r str)> for Layer2<S>
    where
        S: for<'r2> Service<(&'r2 str, &'r2 str, &'r2 str), Response = Res, Error = Err> + 'static,
    {
        type Response = Res;
        type Error = Err;

        async fn call(&self, req: (&'r str, &'r str)) -> Result<Self::Response, Self::Error> {
            let req = (req.0, req.1, self.name.as_str());
            self.service.call(req).await
        }
    }

    struct DummyService;

    impl<'r> Service<(&'r str, &'r str, &'r str)> for DummyService {
        type Response = String;
        type Error = ();

        async fn call(&self, req: (&'r str, &'r str, &'r str)) -> Result<Self::Response, Self::Error> {
            let mut res = String::new();
            res.push_str(req.0);
            res.push_str(req.1);
            res.push_str(req.2);

            Ok(res)
        }
    }

    #[test]
    fn nest_service() {
        let service = Layer2 {
            name: String::from("Layer2"),
            service: DummyService,
        };

        let service = Layer1 {
            name: String::from("Layer1"),
            service,
        };

        let req = "Request";

        let res = service.call(req).now_or_panic().unwrap();

        assert_eq!(res, String::from("RequestLayer1Layer2"));
    }
}
