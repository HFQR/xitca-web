//! example for using trait object as application state.

use std::sync::Arc;

use xitca_web::{
    handler::{handler_service, state::StateRef},
    route::get,
    App,
};

fn main() -> std::io::Result<()> {
    App::new()
        // use trait object as application state
        .with_state(object_state())
        // extract trait object in handler
        .at("/", get(handler_service(handler)))
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

// thread safe trait object state constructor.
fn object_state() -> Arc<dyn DI + Send + Sync> {
    // only this function knows the exact type implementing DI trait.
    // everywhere else in the application it's only known as dyn DI trait object.
    Arc::new(Foo)
}

// StateRef extractor is able extract &dyn DI from application state.
async fn handler(StateRef(obj): StateRef<'_, dyn DI + Send + Sync>) -> String {
    // get request to "/" path should return "hello foo" string response.
    format!("hello {}", obj.name())
}

// a simple trait for injecting dependent type and logic to application.
trait DI {
    fn name(&self) -> &'static str;
}

// a dummy type implementing DI trait.
struct Foo;

impl DI for Foo {
    fn name(&self) -> &'static str {
        "foo"
    }
}
