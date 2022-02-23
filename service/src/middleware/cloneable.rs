use core::future::Future;

use alloc::rc::Rc;

use crate::{service::Service, ServiceFactory};

#[derive(Clone, Copy)]
pub struct Cloneable;

impl<S, Req> ServiceFactory<Req, S> for Cloneable
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Service = Rc<S>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, service: S) -> Self::Future {
        async { Ok(Rc::new(service)) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::{fn_service, ServiceFactory, ServiceFactoryExt};

    #[tokio::test]
    async fn cloneable() {
        let service = fn_service(|_: &'static str| async { Ok::<_, ()>("996") })
            .enclosed(Cloneable)
            .new_service(())
            .await
            .unwrap();

        let res = service.clone().call("req").await.unwrap();

        assert_eq!(res, "996");
    }
}
