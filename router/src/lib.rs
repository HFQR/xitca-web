//! A fork of [matchit](https://github.com/ibraheemdev/matchit) using small string type for params lifetime elision.
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut router = xitca_router::Router::new();
//! router.insert("/home", "Welcome!")?;
//! router.insert("/users/:id", "A User")?;
//!
//! let matched = router.at("/users/978")?;
//! assert_eq!(*matched.value, "A User");
//!
//! // params is owned value that can be sent between threads.
//! let params = matched.params;
//! std::thread::spawn(move || {
//!     assert_eq!(params.get("id"), Some("978"));
//! })
//! .join()
//! .unwrap();
//! # Ok(())
//! # }
//! ```
//!
//! ## Parameters
//!
//! Along with static routes, the router also supports dynamic route segments. These can either be named or catch-all parameters:
//!
//! ### Named Parameters
//!
//! Named parameters like `/:id` match anything until the next `/` or the end of the path:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut m = xitca_router::Router::new();
//! m.insert("/users/:id", true)?;
//!
//! assert_eq!(m.at("/users/1")?.params.get("id"), Some("1"));
//! assert_eq!(m.at("/users/23")?.params.get("id"), Some("23"));
//! assert!(m.at("/users").is_err());
//!
//! # Ok(())
//! # }
//! ```
//!
//! ### Catch-all Parameters
//!
//! Catch-all parameters start with `*` and match everything after the `/`.
//! They must always be at the **end** of the route:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut m = xitca_router::Router::new();
//! m.insert("/*p", true)?;
//!
//! assert_eq!(m.at("/foo.js")?.params.get("p"), Some("foo.js"));
//! assert_eq!(m.at("/c/bar.css")?.params.get("p"), Some("c/bar.css"));
//!
//! # Ok(())
//! # }
//! ```
//!
//! ### Relaxed Catch-all Parameters
//!
//! Relaxed Catch-all parameters with a single `*` and match everything after the `/`(Including `/` itself).
//! Since there is no identifier for Params key associated they are left empty.
//! They must always be at the **end** of the route:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut m = xitca_router::Router::new();
//! m.insert("/*", true)?;
//!
//! assert!(m.at("/")?.value);
//! assert!(m.at("/foo")?.value);
//!
//! # Ok(())
//! # }
//! ```
//!
//! ## Routing Priority
//!
//! Static and dynamic route segments are allowed to overlap. If they do, static segments will be given higher priority:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut m = xitca_router::Router::new();
//! m.insert("/", "Welcome!").unwrap()    ;  // priority: 1
//! m.insert("/about", "About Me").unwrap(); // priority: 1
//! m.insert("/*filepath", "...").unwrap();  // priority: 2
//!
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

extern crate alloc;

mod error;
mod router;
mod tree;

pub mod params;

pub use error::{InsertError, MatchError};
pub use router::{Match, Router};
