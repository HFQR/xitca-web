use actix_http_alt::HttpResponse;

use super::request::WebRequest;

pub type WebResponse = HttpResponse;

pub trait Responder<D>: Sized {
    fn respond_to(self, req: &WebRequest<'_, D>) -> WebResponse;
}

impl<D> Responder<D> for WebResponse {
    fn respond_to(self, _: &WebRequest<'_, D>) -> WebResponse {
        self
    }
}
