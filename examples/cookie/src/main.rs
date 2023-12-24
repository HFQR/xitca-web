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
    error::Error,
    handler::{
        cookie::{CookieJar, Key, Private},
        handler_service,
        redirect::Redirect,
        FromRequest,
    },
    http::StatusCode,
    route::{get, post},
    App, WebContext,
};

fn main() -> std::io::Result<()> {
    // a random generated private key.
    let key = StateKey(Key::generate());
    App::with_state(key)
        .at("/", get(handler_service(index)))
        .at("/auth", post(handler_service(auth)))
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

// storage location of key can be customized:
// normally it's either in application state or extension middleware and can be extended
// to any key storage location(database, remote peer, cloud, etc) as long as the key
// retrieving logic can be implemented inside FromRequest::from_request trait method.

// a cookie jar key stored in application state.
#[derive(Clone)]
struct StateKey(Key);

// necessary boilerplate for CookieJar to convert our key type to the key
// it knows.
impl From<StateKey> for Key {
    fn from(key: StateKey) -> Key {
        key.0
    }
}

// logic of retrieving state key.
impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for StateKey
where
    // please reference xitca_web::App::with_state API document for explanation.
    C: std::borrow::Borrow<Self>,
{
    type Type<'b> = Self;
    type Error = Error<C>;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.state().borrow().clone())
    }
}

// type alias of cookie jar.
// The Private type indicate the cookie jar is private and encrypted with a key which is
// the StateKey type we setup earlier.
type PrivateJar = CookieJar<Private<StateKey>>;

// extract extension key and construct a new private cookie jar.
async fn auth(key: StateKey) -> (Redirect, PrivateJar) {
    let mut jar = CookieJar::private(key);
    jar.add(("user", "foo"));
    // send cookie to client and redirect him to /
    (Redirect::see_other("/"), jar)
}

// extract cookie and generate http response.
async fn index<C>(jar: PrivateJar) -> Result<String, Error<C>> {
    match jar.get("user") {
        Some(user) => Ok(format!("hello: {user}")),
        None => Err(StatusCode::UNAUTHORIZED.into()),
    }
}
