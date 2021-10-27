pub(crate) use xitca_http::body::{ResponseBody, ResponseBodySize, StreamBody};

/// When used by client [ResponseBody] is used as Request body.
pub type RequestBody<B = StreamBody> = ResponseBody<B>;

/// When used by client [ResponseBodySize] is used as Request body size.
pub type RequestBodySize = ResponseBodySize;
