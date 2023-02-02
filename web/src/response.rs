pub use xitca_http::http::response::Builder as WebResponseBuilder;

use xitca_http::response::Response;

use super::body::ResponseBody;

pub type WebResponse<B = ResponseBody> = Response<B>;
