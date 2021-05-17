use std::{
    ops::Deref,
    rc::Rc,
    task::{Context, Poll},
};

use actix_service_alt::Service;

/// A smart pointer holds one service type. Used for Http/2 and Http/3
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

impl<S: Service> HttpFlowSimple<S> {
    pub fn new(service: S) -> Self {
        let inner = HttpFlowSimpleInner { service };

        Self(Rc::new(inner))
    }

    pub fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.service.poll_ready(cx)
    }
}

/// A smart pointer holds 3 services. Used for Http/1
pub(crate) struct HttpFlow<S, X, U>(Rc<HttpFlowInner<S, X, U>>);

impl<S, X, U> Deref for HttpFlow<S, X, U> {
    type Target = HttpFlowInner<S, X, U>;

    fn deref(&self) -> &Self::Target {
        &*self.0
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

        Self(Rc::new(inner))
    }
}
