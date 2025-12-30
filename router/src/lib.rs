//! A fork of [matchit](https://github.com/ibraheemdev/matchit) using small string type for params lifetime elision.
//!
//!```rust
//!use xitca_router::Router;
//!
//!fn main() -> Result<(), Box<dyn core::error::Error>> {
//!    let mut router = Router::new();
//!    router.insert("/home", "Welcome!")?;
//!    router.insert("/users/{id}", "A User")?;
//!
//!    let matched = router.at("/users/978")?;
//!    assert_eq!(matched.params.get("id"), Some("978"));
//!    assert_eq!(*matched.value, "A User");
//!
//!    Ok(())
//!}
//!```
//!
//!# Parameters
//!
//!The router supports dynamic route segments. These can either be named or catch-all parameters.
//!
//!Named parameters like `/{id}` match anything until the next static segment or the end of the path.
//!
//!```rust
//!# use xitca_router::Router;
//!# fn main() -> Result<(), Box<dyn core::error::Error>> {
//!let mut router = Router::new();
//!router.insert("/users/{id}", 42)?;
//!
//!let matched = router.at("/users/1")?;
//!assert_eq!(matched.params.get("id"), Some("1"));
//!
//!let matched = router.at("/users/23")?;
//!assert_eq!(matched.params.get("id"), Some("23"));
//!
//!assert!(router.at("/users").is_err());
//!# Ok(())
//!# }
//!```
//!
//!Prefixes and suffixes within a segment are also supported. However, there may only be a single named parameter per route segment.
//!```rust
//!# use xitca_router::Router;
//!# fn main() -> Result<(), Box<dyn core::error::Error>> {
//!let mut router = Router::new();
//!router.insert("/images/img-{id}.png", true)?;
//!
//!let matched = router.at("/images/img-1.png")?;
//!assert_eq!(matched.params.get("id"), Some("1"));
//!
//!assert!(router.at("/images/img-1.jpg").is_err());
//!# Ok(())
//!# }
//!```
//!
//!Catch-all parameters start with a `*` and match anything until the end of the path. They must always be at the *end* of the route.
//!
//!```rust
//!# use xitca_router::Router;
//!# fn main() -> Result<(), Box<dyn core::error::Error>> {
//!let mut router = Router::new();
//!router.insert("/{*rest}", true)?;
//!
//!let matched = router.at("/foo.html")?;
//!assert_eq!(matched.params.get("rest"), Some("foo.html"));
//!
//!let matched = router.at("/static/bar.css")?;
//!assert_eq!(matched.params.get("rest"), Some("static/bar.css"));
//!
//!// Note that this would lead to an empty parameter value.
//!assert!(router.at("/").is_err());
//!# Ok(())
//!# }
//!```
//!
//!Relaxed Catch-all Parameters
//!Relaxed Catch-all parameters with a single `*` and match everything after the `/`(Including `/` itself). They must always be at the **end** of the route:
//!
//! ```rust
//!# use xitca_router::Router;
//!# fn main() -> Result<(), Box<dyn core::error::Error>> {
//! let mut m = Router::new();
//! m.insert("/{*}", true)?;
//!
//! assert!(m.at("/")?.value);
//! assert!(m.at("/foo")?.value);
//!
//! # Ok(())
//! # }
//! ```
//!
//!The literal characters `{` and `}` may be included in a static route by escaping them with the same character. For example, the `{` character is escaped with `{{`, and the `}` character is escaped with `}}`.
//!
//!```rust
//!# use xitca_router::Router;
//!# fn main() -> Result<(), Box<dyn core::error::Error>> {
//!let mut router = Router::new();
//!router.insert("/{{hello}}", true)?;
//!router.insert("/{hello}", true)?;
//!
//!// Match the static route.
//!let matched = router.at("/{hello}")?;
//!assert!(matched.params.is_empty());
//!
//!// Match the dynamic route.
//!let matched = router.at("/hello")?;
//!assert_eq!(matched.params.get("hello"), Some("hello"));
//!# Ok(())
//!# }
//!```
//!
//!# Conflict Rules
//!
//!Static and dynamic route segments are allowed to overlap. If they do, static segments will be given higher priority:
//!
//!```rust
//!# use xitca_router::Router;
//!# fn main() -> Result<(), Box<dyn core::error::Error>> {
//!let mut router = Router::new();
//!router.insert("/", "Welcome!").unwrap();       // Priority: 1
//!router.insert("/about", "About Me").unwrap();  // Priority: 1
//!router.insert("/{*filepath}", "...").unwrap();  // Priority: 2
//!# Ok(())
//!# }
//!```
//!
//!Formally, a route consists of a list of segments separated by `/`, with an optional leading and trailing slash: `(/)<segment_1>/.../<segment_n>(/)`.
//!
//!Given set of routes, their overlapping segments may include, in order of priority:
//!
//!- Any number of static segments (`/a`, `/b`, ...).
//!- *One* of the following:
//!  - Any number of route parameters with a suffix (`/{x}a`, `/{x}b`, ...), prioritizing the longest suffix.
//!  - Any number of route parameters with a prefix (`/a{x}`, `/b{x}`, ...), prioritizing the longest prefix.
//!  - A single route parameter with both a prefix and a suffix (`/a{x}b`).
//!- *One* of the following;
//!  - A single standalone parameter (`/{x}`).
//!  - A single standalone catch-all parameter (`/{*rest}`). Note this only applies to the final route segment.
//!
//!Any other combination of route segments is considered ambiguous, and attempting to insert such a route will result in an error.
//!
//!The one exception to the above set of rules is that catch-all parameters are always considered to conflict with suffixed route parameters, i.e. that `/{*rest}`
//!and `/{x}suffix` are overlapping. This is due to an implementation detail of the routing tree that may be relaxed in the future.

#![forbid(unsafe_code)]
#![no_std]

mod error;
mod escape;
mod router;
mod tree;

pub mod params;

pub use error::{InsertError, MatchError};
pub use router::{Match, Router};

extern crate alloc;

// TODO: consider no alloc alternative of these types so alloc can become an optional feature
use alloc::{collections::VecDeque, string::String, vec::Vec};
use xitca_unsafe_collection::small_str::SmallBoxedStr as SmallStr;
