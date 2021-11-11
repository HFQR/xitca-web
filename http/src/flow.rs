pub(crate) struct HttpFlow<S, X> {
    pub(crate) service: S,
    pub(crate) expect: X,
}

impl<S, X> HttpFlow<S, X> {
    pub fn new(service: S, expect: X) -> Self {
        Self { service, expect }
    }
}
