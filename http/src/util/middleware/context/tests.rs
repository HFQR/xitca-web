use std::convert::Infallible;

use xitca_service::{ServiceExt, fn_service};
use xitca_unsafe_collection::futures::NowOrPanic;

use crate::http::{Request, Response};

use super::*;

struct Context2<'a, ST> {
    req: Request<()>,
    state: &'a ST,
}

async fn into_context(req: Context<'_, Request<()>, String>) -> Result<Context2<'_, String>, Infallible> {
    let (req, state) = req.into_parts();
    assert_eq!(state, "string_state");
    Ok(Context2 { req, state })
}

async fn ctx_handler(ctx: Context2<'_, String>) -> Result<Response<()>, Infallible> {
    assert_eq!(ctx.state, "string_state");
    assert_eq!(ctx.req.method().as_str(), "GET");
    Ok(Response::new(()))
}

#[test]
fn test_state_and_then() {
    let res = fn_service(into_context)
        .and_then(fn_service(ctx_handler))
        .enclosed(ContextBuilder::new(|| async {
            Ok::<_, Infallible>(String::from("string_state"))
        }))
        .call(())
        .now_or_panic()
        .ok()
        .unwrap()
        .call(Request::default())
        .now_or_panic()
        .unwrap();

    assert_eq!(res.status().as_u16(), 200);
}

#[cfg(feature = "router")]
#[test]
fn test_state_in_router() {
    use crate::{
        http::RequestExt,
        util::service::{Router, route::get},
    };

    async fn enclosed<S, Req, C, Res, Err>(service: &S, req: Context<'_, Req, C>) -> Result<Res, Err>
    where
        S: for<'c> Service<Context<'c, Req, C>, Response = Res, Error = Err>,
    {
        service.call(req).await
    }

    let router = || {
        Router::new().insert(
            "/",
            get(fn_service(async |req: Context<'_, Request<RequestExt<()>>, String>| {
                let (_, state) = req.into_parts();
                assert_eq!(state, "string_state");
                Ok::<_, Infallible>(Response::new(()))
            })),
        )
    };

    let router_with_ctx = || {
        router().enclosed_fn(enclosed).enclosed(ContextBuilder::new(|| async {
            Ok::<_, Infallible>(String::from("string_state"))
        }))
    };

    fn bound_check<T: Send + Sync>(_: T) {}

    bound_check(router_with_ctx());

    let res = router_with_ctx()
        .call(())
        .now_or_panic()
        .ok()
        .unwrap()
        .call(Request::default())
        .now_or_panic()
        .unwrap();

    assert_eq!(res.status().as_u16(), 200);

    let res = router()
        .insert("/nest", router())
        .enclosed(ContextBuilder::new(|| async {
            Ok::<_, Infallible>(String::from("string_state"))
        }))
        .call(())
        .now_or_panic()
        .ok()
        .unwrap()
        .call(Request::default())
        .now_or_panic()
        .unwrap();

    assert_eq!(res.status().as_u16(), 200);
}
