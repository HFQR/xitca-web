pub use xitca_http::{http::response::Builder as WebResponseBuilder, ResponseBody};

use xitca_http::http::Response;

use super::request::WebRequest;

// TODO: add app state to response type.
pub type WebResponse = Response<ResponseBody>;

pub trait Responder<D>: Sized {
    fn respond_to(self, req: &mut WebRequest<'_, D>) -> WebResponse;
}

impl<D> Responder<D> for WebResponse {
    fn respond_to(self, _: &mut WebRequest<'_, D>) -> WebResponse {
        self
    }
}
