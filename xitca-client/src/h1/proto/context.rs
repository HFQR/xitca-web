use xitca_http::http::HeaderMap;

pub(crate) struct Context {
    headers: HeaderMap,
}

impl Context {
    pub(crate) fn new(headers: HeaderMap) -> Self {
        Self { headers }
    }

    pub(crate) fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub(crate) fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }
}
