pub use xitca_http::{
    body::{ResponseBody, StreamBody},
    http::response::Builder as WebResponseBuilder,
};

use crate::http::Response;

pub type WebResponse<B = StreamBody> = Response<ResponseBody<B>>;
