use std::{ops::Deref, rc::Rc};

pub(crate) struct HttpFlow<S, X, U>(Rc<HttpFlowInner<S, X, U>>);

impl<S, X, U> Clone for HttpFlow<S, X, U> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S, X, U> Deref for HttpFlow<S, X, U> {
    type Target = HttpFlowInner<S, X, U>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(crate) struct HttpFlowInner<S, X, U> {
    pub(crate) service: S,
    pub(crate) expect: X,
    pub(crate) upgrade: Option<U>,
}

impl<S, X, U> HttpFlow<S, X, U> {
    pub fn new(service: S, expect: X, upgrade: Option<U>) -> Self {
        let inner = HttpFlowInner {
            service,
            expect,
            upgrade,
        };

        Self(Rc::new(inner))
    }
}
