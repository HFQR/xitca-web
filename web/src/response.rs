//! web response types.

pub use xitca_http::http::response::Builder as WebResponseBuilder;

use xitca_http::http::Response;

use super::body::ResponseBody;

pub type WebResponse<B = ResponseBody> = Response<B>;
