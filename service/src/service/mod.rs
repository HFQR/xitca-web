pub(crate) mod pipeline;

mod and_then;
mod enclosed_fn;
mod map;
mod map_err;
mod object;

pub use object::ServiceObject;

use core::{future::Future, ops::Deref, pin::Pin};

use alloc::{boxed::Box, rc::Rc, sync::Arc};

pub trait Service<Req> {
    type Response;

    type Error;

    type Future<'f>: Future<Output = Result<Self::Response, Self::Error>>
    where
        Self: 'f;

    fn call(&self, req: Req) -> Self::Future<'_>;
}

macro_rules! impl_alloc {
    ($alloc: ident) => {
        impl<S, Req> Service<Req> for $alloc<S>
        where
            S: Service<Req> + ?Sized,
        {
            type Response = S::Response;
            type Error = S::Error;
            type Future<'f> = S::Future<'f> where Self: 'f;

            #[inline]
            fn call(&self, req: Req) -> Self::Future<'_> {
                (**self).call(req)
            }
        }
    };
}

impl_alloc!(Box);
impl_alloc!(Rc);
impl_alloc!(Arc);

impl<S, Req> Service<Req> for Pin<S>
where
    S: Deref,
    S::Target: Service<Req>,
{
    type Response = <S::Target as Service<Req>>::Response;
    type Error = <S::Target as Service<Req>>::Error;
    type Future<'f> = <S::Target as Service<Req>>::Future<'f> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        self.as_ref().get_ref().call(req)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use alloc::string::String;

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
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

        fn call(&self, req: &'r str) -> Self::Future<'_> {
            async move {
                let req = (req, self.name.as_str());
                self.service.call(req).await
            }
        }
    }

    impl<'r, S, Res, Err> Service<(&'r str, &'r str)> for Layer2<S>
    where
        S: for<'r2> Service<(&'r2 str, &'r2 str, &'r2 str), Response = Res, Error = Err> + 'static,
    {
        type Response = Res;
        type Error = Err;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

        fn call(&self, req: (&'r str, &'r str)) -> Self::Future<'_> {
            async move {
                let req = (req.0, req.1, self.name.as_str());
                self.service.call(req).await
            }
        }
    }

    struct DummyService;

    impl<'r> Service<(&'r str, &'r str, &'r str)> for DummyService {
        type Response = String;
        type Error = ();
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

        fn call(&self, req: (&'r str, &'r str, &'r str)) -> Self::Future<'_> {
            async move {
                let mut res = String::new();
                res.push_str(req.0);
                res.push_str(req.1);
                res.push_str(req.2);

                Ok(res)
            }
        }
    }

    #[tokio::test]
    async fn nest_service() {
        let service = Layer2 {
            name: String::from("Layer2"),
            service: DummyService,
        };

        let service = Layer1 {
            name: String::from("Layer1"),
            service,
        };

        let req = "Request";

        let res = service.call(req).await.unwrap();

        assert_eq!(res, String::from("RequestLayer1Layer2"));
    }
}
