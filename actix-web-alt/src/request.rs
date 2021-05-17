use actix_http_alt::HttpRequest;
use std::marker::PhantomData;

#[derive(Copy, Clone)]
pub struct WebRequest<'a, D> {
    pub http: &'a HttpRequest,
    pub data: &'a D,
}
