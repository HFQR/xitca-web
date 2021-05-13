use std::{ops::Deref, rc::Rc};

pub(crate) struct HttpFlow<S>(Rc<HttpFlowInner<S>>);

impl<S> Clone for HttpFlow<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S> Deref for HttpFlow<S> {
    type Target = HttpFlowInner<S>;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

pub(crate) struct HttpFlowInner<S> {
    pub(crate) service: S,
}

impl<S> HttpFlow<S> {
    pub fn new(service: S) -> Self {
        let inner = HttpFlowInner { service };

        Self(Rc::new(inner))
    }
}
