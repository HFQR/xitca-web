use std::{ops::Deref, rc::Rc};

/// A smart pointer holds one service type. Used for Http/2 and Http/3
///
/// Rc is used for multiplexing connections in Http/2 and Http/3.
pub(crate) struct HttpFlowSimple<S>(Rc<HttpFlowSimpleInner<S>>);

impl<S> Clone for HttpFlowSimple<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S> Deref for HttpFlowSimple<S> {
    type Target = HttpFlowSimpleInner<S>;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

pub(crate) struct HttpFlowSimpleInner<S> {
    pub(crate) service: S,
}

impl<S> HttpFlowSimple<S> {
    pub fn new(service: S) -> Self {
        let inner = HttpFlowSimpleInner { service };

        Self(Rc::new(inner))
    }
}

/// A struct holds 3 services. Used for Http/1
pub(crate) struct HttpFlow<S, X, U>(HttpFlowInner<S, X, U>);

impl<S, X, U> Deref for HttpFlow<S, X, U> {
    type Target = HttpFlowInner<S, X, U>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(crate) struct HttpFlowInner<S, X, U> {
    pub(crate) service: S,
    pub(crate) expect: X,
    pub(crate) upgrade: U,
}

impl<S, X, U> HttpFlow<S, X, U> {
    pub fn new(service: S, expect: X, upgrade: U) -> Self {
        let inner = HttpFlowInner {
            service,
            expect,
            upgrade,
        };

        Self(inner)
    }
}
