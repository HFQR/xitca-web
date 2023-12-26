//! lazy type extractor.

use core::marker::PhantomData;

use std::borrow::Cow;

/// lazy deserialize type that wrap around other extractor type like `Json`, `Form`, `Params` 
/// and `Query`. It lowers the deserialization to handler function where zero copy deserialize 
/// can happen.
///
/// # Example
/// ```rust(no_run)
/// use serde::Deserialize;
/// use xitca_web::{
///   error::Error,
///   http::StatusCode,
///   handler::{handler_service, json::Json, lazy::Lazy},
///   App, WebContext
/// };
///
/// // a json object with zero copy deserialization.
/// #[derive(Deserialize)]
/// struct Post<'a> {
///     title: &'a str,
///     content: &'a str    
/// }
///
/// // handler function utilize Lazy type to lower the Json type into handler function.
/// async fn handler(lazy: Lazy<'static, Json<Post<'_>>>) -> Result<String, Error> {
///     // actual deserialize happens here.
///     let Post { title, content } = lazy.deserialize()?;
///     // the Post type and it's &str referencing lazy would live until the handler function
///     // returns.
///     Ok(format!("Post {{ title: {title}, content: {content} }}"))
/// }
///
/// App::new()
///     .at("/post", handler_service(handler))
///     .at("/", handler_service(|_: &WebContext<'_>| async { "used for infer type" }));
/// ```
pub struct Lazy<'a, T> {
    inner: Cow<'a, [u8]>,
    _type: PhantomData<T>,
}

impl<T> Lazy<'_, T> {
    pub(super) fn as_slice(&self) -> &[u8] {
        match self.inner {
            Cow::Owned(ref vec) => vec.as_slice(),
            Cow::Borrowed(slice) => slice,
        }
    }
}

impl<T> From<Vec<u8>> for Lazy<'static, T> {
    fn from(vec: Vec<u8>) -> Self {
        Self {
            inner: Cow::Owned(vec),
            _type: PhantomData,
        }
    }
}

impl<'a, T> From<&'a [u8]> for Lazy<'a, T> {
    fn from(slice: &'a [u8]) -> Self {
        Self {
            inner: Cow::Borrowed(slice),
            _type: PhantomData,
        }
    }
}
