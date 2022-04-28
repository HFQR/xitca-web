use core::{convert::Infallible, future::Future};

use alloc::rc::Rc;

use crate::build::BuildService;

#[derive(Clone, Copy)]
pub struct Cloneable;

impl<S> BuildService<S> for Cloneable {
    type Service = Rc<S>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, service: S) -> Self::Future {
        async { Ok(Rc::new(service)) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::{fn_service, BuildService, BuildServiceExt, Service};

    #[tokio::test]
    async fn cloneable() {
        let factory = fn_service(|_: &'static str| async { Ok::<_, ()>("996") }).enclosed(Cloneable);

        let service = BuildService::build(&factory, ()).await.unwrap();

        let res = service.clone().call("req").await.unwrap();

        assert_eq!(res, "996");
    }
}
