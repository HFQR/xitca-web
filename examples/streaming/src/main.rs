//! example showcasing http response body streaming with xitca-web.
//!
//! example must be compiled with nightly Rust. nightly is not required for streaming but it can enhance
//! the experience with native async generator and other async nightly features (for await loop .etc)

// async generator blocks nightly feature for high level streaming interface.
#![feature(gen_blocks)]

use std::{
    convert::Infallible,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use xitca_web::{
    body::{AsyncBody, BoxBody, RequestBody, ResponseBody},
    handler::handler_service,
    http::{
        const_header_value::TEXT_UTF8,
        header::{HeaderName, HeaderValue, CONTENT_TYPE},
    },
    route::{get, post},
    service::Service,
    App, WebContext,
};

fn main() -> io::Result<()> {
    App::new()
        // route handler utilize http response streaming body.
        .at("/nightly", get(handler_service(nightly)))
        .at("/macro", get(handler_service(r#macro)))
        .at("/", get(handler_service(stable)))
        // route handler utilize http request streaming body.
        .at("/body", post(handler_service(body)))
        // router middleware utilize http request streaming body.
        .enclosed_fn(nightly_middleware)
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

// streaming with nightly Rust's async generator.
async fn nightly() -> ((HeaderName, HeaderValue), ResponseBody) {
    (
        (CONTENT_TYPE, TEXT_UTF8),
        ResponseBody::box_stream(AsyncBody::from(async gen {
            yield Ok("hello,");
            yield Ok::<_, Infallible>("world!");
        })),
    )
}

// streaming with stable Rust and async-stream macro crate.
// it achieves the same effect as async generator with extra dependency.
async fn r#macro() -> ((HeaderName, HeaderValue), ResponseBody) {
    (
        (CONTENT_TYPE, TEXT_UTF8),
        ResponseBody::box_stream(async_stream::stream! {
            yield Ok("hello,");
            yield Ok::<_, Infallible>("world!");
        }),
    )
}

// streaming with stable Rust and futures::Stream trait implement.
// low level stream type and trait implement requires no extra dep.
async fn stable() -> ((HeaderName, HeaderValue), ResponseBody) {
    ((CONTENT_TYPE, TEXT_UTF8), ResponseBody::box_stream(Hello::Step1))
}

// typed stream and trait implement.
enum Hello {
    Step1,
    Step2,
    Step3,
}

impl futures::stream::Stream for Hello {
    type Item = Result<&'static str, Infallible>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match *this {
            Self::Step1 => {
                *this = Hello::Step2;
                Poll::Ready(Some(Ok("hello,")))
            }
            Self::Step2 => {
                *this = Hello::Step3;
                Poll::Ready(Some(Ok("world!")))
            }
            Self::Step3 => Poll::Ready(None),
        }
    }
}

// route handler echo post request with body as string. used for active below nightly_middleware for
// counting http request body.
async fn body(body: String) -> String {
    body
}

// request body byte counting middleware with nightly Rust's async generator.
async fn nightly_middleware<S, C, Res, Err>(next: &S, mut ctx: WebContext<'_, C>) -> Result<Res, Err>
where
    S: for<'r> Service<WebContext<'r, C>, Response = Res, Error = Err>,
{
    let body = ctx.body_get_mut();

    // take request body and move it into generator for async iterating.
    let mut body_own = std::mem::take(body);

    let async_body = AsyncBody::from(async gen move {
        // a counter type with drop guard
        struct Counter(usize);

        // downstream async body consumer is free to cancel body streaming at any
        // time so print the counter on drop to coop with possible cancelation.
        impl Drop for Counter {
            fn drop(&mut self) {
                println!("request body size: {}", self.0);
            }
        }

        let mut counter = Counter(0);

        // request body does not implement AsyncIterator and futures crate must be used for async yielding.
        use futures::stream::StreamExt;
        while let Some(res) = body_own.next().await {
            // record every successful bytes and yield body item.
            yield res.map(|bytes| {
                counter.0 += bytes.len();
                bytes
            });
        }
    });

    // assign newly generate request streaming body to context.
    *body = RequestBody::from(BoxBody::new(async_body));

    // call next application service.
    next.call(ctx).await
}

// stable and macro based middlewares are skipped. The difference is similar to between route handlers where instead of
// async generator the macro or type implementing Stream trait would be passed to BoxBody::new

// a personal suggestion from the maintainer:
// use nightly async gen block when you have spare time that can afford cutting edge compiler features.
// use async-stream macro when you want a easy to setup streaming api that can transit to async gen block in the future.
// use Stream trait with low leve poll_next implement when you want the most efficient and stable streaming api.
