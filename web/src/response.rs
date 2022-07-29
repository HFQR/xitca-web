pub use xitca_http::{http::response::Builder as WebResponseBuilder, ResponseBody};

use crate::http::Response;

// TODO: add app state to response type.
pub type WebResponse<B = ResponseBody> = Response<B>;
