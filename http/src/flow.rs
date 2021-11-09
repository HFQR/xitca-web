use std::{ops::Deref, rc::Rc};

pub(crate) struct HttpFlow<S, X>(Rc<HttpFlowInner<S, X>>);

impl<S, X> Clone for HttpFlow<S, X> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S, X> Deref for HttpFlow<S, X> {
    type Target = HttpFlowInner<S, X>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(crate) struct HttpFlowInner<S, X> {
    pub(crate) service: S,
    pub(crate) expect: X,
}

impl<S, X> HttpFlow<S, X> {
    pub fn new(service: S, expect: X) -> Self {
        let inner = HttpFlowInner { service, expect };

        Self(Rc::new(inner))
    }
}
