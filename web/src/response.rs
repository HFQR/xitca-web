pub use xitca_http::{
    body::{ResponseBody, StreamBody},
    http::response::Builder as WebResponseBuilder,
};

use xitca_http::response::Response;

pub type WebResponse<B = ResponseBody> = Response<B>;
