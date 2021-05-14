use http::Response;

use super::body::ResponseBody;

/// Type alias for [Request](http::Response) type where generic body type
/// is default to [Body](super::body::ResponseBody)
pub type HttpResponse<B = ResponseBody> = Response<B>;
