use core::convert::Infallible;

use xitca_service::{ServiceExt, fn_service, ready::ReadyService};
use xitca_unsafe_collection::futures::NowOrPanic;

use crate::{
    http::{RequestExt, Response},
    util::service::route::get,
};

use super::*;

async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
where
    S: Service<Req>,
{
    service.call(req).await
}

async fn func(_: Request<RequestExt<()>>) -> Result<Response<()>, Infallible> {
    Ok(Response::new(()))
}

#[test]
fn router_sync() {
    fn bound_check<T: Send + Sync>(_: T) {}

    bound_check(Router::new().insert("/", fn_service(func)))
}

#[test]
fn router_accept_request() {
    Router::new()
        .insert("/", fn_service(func))
        .call(())
        .now_or_panic()
        .unwrap()
        .call(Request::default())
        .now_or_panic()
        .unwrap();
}

#[test]
fn router_enclosed_fn() {
    Router::new()
        .insert("/", fn_service(func))
        .enclosed_fn(enclosed)
        .call(())
        .now_or_panic()
        .unwrap()
        .call(Request::default())
        .now_or_panic()
        .unwrap();
}

#[test]
fn router_add_params_http() {
    let req = Request::builder().uri("/users/1").body(Default::default()).unwrap();

    Router::new()
        .insert(
            "/users/{id}",
            fn_service(|req: Request<RequestExt<()>>| async move {
                let params = req.body().params();
                assert_eq!(params.get("id").unwrap(), "1");
                Ok::<_, Infallible>(Response::new(()))
            }),
        )
        .enclosed_fn(enclosed)
        .call(())
        .now_or_panic()
        .unwrap()
        .call(req)
        .now_or_panic()
        .unwrap();
}

#[test]
fn router_nest() {
    let handler = || get(fn_service(func)).enclosed_fn(enclosed);

    let router = || Router::new().insert("/nest", handler()).enclosed_fn(enclosed);

    let req = || {
        Request::builder()
            .uri("http://foo.bar/scope/nest")
            .body(Default::default())
            .unwrap()
    };

    Router::new()
        .insert("/raw", fn_service(func))
        .insert("/root", handler())
        .insert("/scope", router())
        .call(())
        .now_or_panic()
        .unwrap()
        .call(req())
        .now_or_panic()
        .unwrap();

    Router::new()
        .insert("/root", handler())
        .insert("/scope/", router())
        .call(())
        .now_or_panic()
        .unwrap()
        .call(req())
        .now_or_panic()
        .unwrap();

    Router::new()
        .insert(
            "/1111/",
            Router::new().insert(
                "/222222/3",
                Router::new().insert("/3333333", Router::new().insert("/4444", router())),
            ),
        )
        .call(())
        .now_or_panic()
        .unwrap()
        .call(
            Request::builder()
                .uri("http://foo.bar/1111/222222/3/3333333/4444/nest")
                .body(Default::default())
                .unwrap(),
        )
        .now_or_panic()
        .unwrap();

    Router::new()
        .insert("/api", Router::new().insert("/v2", router()))
        .call(())
        .now_or_panic()
        .unwrap()
        .call(
            Request::builder()
                .uri("http://foo.bar/api/v2/nest")
                .body(Default::default())
                .unwrap(),
        )
        .now_or_panic()
        .unwrap();
}

#[test]
fn router_service_call_size() {
    let service = Router::new()
        .insert("/", fn_service(func))
        .call(())
        .now_or_panic()
        .unwrap();

    println!(
        "router service ready call size: {:?}",
        core::mem::size_of_val(&service.ready())
    );

    println!(
        "router service call size: {:?}",
        core::mem::size_of_val(&service.call(Request::default()))
    );
}

#[test]
fn router_typed() {
    type Req = Request<RequestExt<()>>;
    type Route = BoxedServiceObject<Req, Response<()>, RouterError<Infallible>>;
    type Object = object::RouteObject<(), Route, Infallible>;

    struct Index;

    impl TypedRoute for Index {
        type Route = Object;

        fn path() -> &'static str {
            "/"
        }

        fn route() -> Self::Route {
            Req::into_object(RouterMapErr(xitca_service::fn_service(func)))
        }
    }

    struct V2;

    impl TypedRoute for V2 {
        type Route = Object;

        fn path() -> &'static str {
            "/v2"
        }

        fn route() -> Self::Route {
            Req::into_object(RouterMapErr(xitca_service::fn_service(func)))
        }
    }

    Router::new()
        .insert_typed(Index)
        .insert_typed(V2)
        .call(())
        .now_or_panic()
        .unwrap()
        .call(Request::default())
        .now_or_panic()
        .unwrap();
}
