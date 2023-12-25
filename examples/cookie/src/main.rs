//! example of utilizing cookies in xitca-web.
//!
//! Follow these steps to test:
//!
//! authentication:
//! curl -v -X POST http://localhost:8080/auth
//!
//! copy the set_cookie: header value from curl response. for example:
//! user=WEEjNTDcymCrmTpjEq77Jp7Zh6krDs41aJbkvuB3ZQ%3D%3D
//!
//! visit auth required route with the cookie just acquired:
//! curl -v -b "<set_cookie_header_value>" http://localhost:8080/

use xitca_web::{
    handler::{
        cookie::{CookieJar, Private, StateKey},
        handler_service,
        redirect::Redirect,
    },
    http::StatusCode,
    route::{get, post},
    App,
};

fn main() -> std::io::Result<()> {
    // a random generated private key that supposed to be added into application state.
    let key = StateKey::generate();
    App::with_state(key)
        .at("/", get(handler_service(index)))
        .at("/auth", post(handler_service(auth)))
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

// type alias of cookie jar.
// The Private type indicate the cookie jar is private and encrypted with a key which is
// the StateKey type we setup earlier.
type PrivateJar = CookieJar<Private<StateKey>>;

// a dummy authenticate route.
async fn auth(key: StateKey) -> (Redirect, PrivateJar) {
    // extract state key and generate a new cookie jar.
    let mut jar = CookieJar::private(key);
    // insert cookie key=value pair to the jar.
    jar.add(("user", "foo"));
    // send cookie jar to client and redirect him to "/" route.
    (Redirect::see_other("/"), jar)
}

// extract cookie and generate http response.
async fn index(jar: PrivateJar) -> Result<String, StatusCode> {
    match jar.get("user") {
        // authenticated user provided cookie key=value pair acquired from "/auth" route
        Some(user) => Ok(format!("hello: {user}")),
        // otherwise it's an unauthorized request.
        None => Err(StatusCode::UNAUTHORIZED),
    }
}
