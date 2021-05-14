use http::Request;

use super::body::RequestBody;

/// Type alias for [Request](http::Request) type where generic body type
/// is default to [Body](super::body::Body)
pub type HttpRequest<B = RequestBody> = Request<B>;
