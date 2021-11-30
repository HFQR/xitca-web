use core::future::{ready, Ready};

use alloc::rc::Rc;

use crate::{service::Service, transform::Transform};

#[derive(Clone, Copy)]
pub struct Cloneable;

impl<S, Req> Transform<S, Req> for Cloneable
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Transform = Rc<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(Rc::new(service)))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::{fn_service, ServiceFactory, ServiceFactoryExt};

    #[tokio::test]
    async fn cloneable() {
        let service = fn_service(|_: &'static str| ready(Ok::<_, ()>("996")))
            .transform(Cloneable)
            .new_service(())
            .await
            .unwrap();

        let res = service.clone().call("req").await.unwrap();

        assert_eq!(res, "996");
    }
}
