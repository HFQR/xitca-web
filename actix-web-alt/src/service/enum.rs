use std::future::Future;
use std::task::{Context, Poll};

use actix_service_alt::{Service, ServiceFactory};

use crate::request::WebRequest;

macro_rules! enum_service ({ $($param:ident),+ } => {
    pub enum EnumService<$($param),+> {
        $($param($param)), +
    }

    #[rustfmt::skip]
    impl<Res, Err, $($param),+> Service for EnumService<$($param),+>
    where
        Res: 'static,
        Err: 'static,
        $($param: for<'r> Service<Request<'r> = WebRequest<'r, ()>, Response = Res, Error = Err> + 'static), +
    {
        type Request<'r> = WebRequest<'r, ()>;
        type Response = Res;
        type Error = Err;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

        fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            match *self {
                $(Self::$param(ref s) => s.poll_ready(cx)), +
            }
        }

        fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
            async move {
                match *self {
                    $(Self::$param(ref s) => s.call(req).await), +
                }
            }
        }
    }
});

enum_service! { A, B, C, D, E, F, G, H, I, J, K, L }
