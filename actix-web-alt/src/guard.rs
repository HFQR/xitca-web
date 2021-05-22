//! Route match guards.

#![allow(non_snake_case)]
use std::{convert::TryFrom, ops::Deref, rc::Rc};

use actix_http_alt::http::{self, header, request::Parts, uri::Uri};

/// Trait defines resource guards. Guards are used for route selection.
///
/// Guards can not modify the request object. But it is possible
/// to store extra attributes on a request by using the `Extensions` container.
/// Extensions containers are available via the `RequestHead::extensions()` method.
pub trait Guard {
    /// Check if request matches predicate
    fn check(&self, req: &Parts) -> bool;
}

impl<G> Guard for Rc<G>
where
    G: Guard + ?Sized,
{
    fn check(&self, req: &Parts) -> bool {
        (**self).check(req)
    }
}

/// Create guard object for supplied function.
pub fn fn_guard<F>(f: F) -> impl Guard
where
    F: Fn(&Parts) -> bool,
{
    FnGuard(f)
}

struct FnGuard<F: Fn(&Parts) -> bool>(F);

impl<F> Guard for FnGuard<F>
where
    F: Fn(&Parts) -> bool,
{
    fn check(&self, req: &Parts) -> bool {
        (self.0)(req)
    }
}

impl<F> Guard for F
where
    F: Fn(&Parts) -> bool,
{
    fn check(&self, req: &Parts) -> bool {
        (self)(req)
    }
}

/// Return guard that matches if any of supplied guards.
pub fn Any<F: Guard + 'static>(guard: F) -> AnyGuard {
    AnyGuard(vec![Box::new(guard)])
}

/// Matches any of supplied guards.
pub struct AnyGuard(Vec<Box<dyn Guard>>);

impl AnyGuard {
    /// Add guard to a list of guards to check
    pub fn or<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.0.push(Box::new(guard));
        self
    }
}

impl Guard for AnyGuard {
    fn check(&self, req: &Parts) -> bool {
        for p in &self.0 {
            if p.check(req) {
                return true;
            }
        }
        false
    }
}

/// Return guard that matches if all of the supplied guards.
pub fn All<F: Guard + 'static>(guard: F) -> AllGuard {
    AllGuard(vec![Box::new(guard)])
}

/// Matches if all of supplied guards.
pub struct AllGuard(Vec<Box<dyn Guard>>);

impl AllGuard {
    /// Add new guard to the list of guards to check
    pub fn and<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.0.push(Box::new(guard));
        self
    }
}

impl Guard for AllGuard {
    fn check(&self, req: &Parts) -> bool {
        for p in &self.0 {
            if !p.check(req) {
                return false;
            }
        }
        true
    }
}

/// Return guard that matches if supplied guard does not match.
pub fn Not<F: Guard + 'static>(guard: F) -> NotGuard {
    NotGuard(Box::new(guard))
}

#[doc(hidden)]
pub struct NotGuard(Box<dyn Guard>);

impl Guard for NotGuard {
    fn check(&self, req: &Parts) -> bool {
        !self.0.check(req)
    }
}

/// HTTP method guard.
#[doc(hidden)]
pub struct MethodGuard(http::Method);

impl Guard for MethodGuard {
    fn check(&self, req: &Parts) -> bool {
        req.method == self.0
    }
}

/// Guard to match *GET* HTTP method.
pub fn Get() -> MethodGuard {
    MethodGuard(http::Method::GET)
}

/// Predicate to match *POST* HTTP method.
pub fn Post() -> MethodGuard {
    MethodGuard(http::Method::POST)
}

/// Predicate to match *PUT* HTTP method.
pub fn Put() -> MethodGuard {
    MethodGuard(http::Method::PUT)
}

/// Predicate to match *DELETE* HTTP method.
pub fn Delete() -> MethodGuard {
    MethodGuard(http::Method::DELETE)
}

/// Predicate to match *HEAD* HTTP method.
pub fn Head() -> MethodGuard {
    MethodGuard(http::Method::HEAD)
}

/// Predicate to match *OPTIONS* HTTP method.
pub fn Options() -> MethodGuard {
    MethodGuard(http::Method::OPTIONS)
}

/// Predicate to match *CONNECT* HTTP method.
pub fn Connect() -> MethodGuard {
    MethodGuard(http::Method::CONNECT)
}

/// Predicate to match *PATCH* HTTP method.
pub fn Patch() -> MethodGuard {
    MethodGuard(http::Method::PATCH)
}

/// Predicate to match *TRACE* HTTP method.
pub fn Trace() -> MethodGuard {
    MethodGuard(http::Method::TRACE)
}

/// Predicate to match specified HTTP method.
pub fn Method(method: http::Method) -> MethodGuard {
    MethodGuard(method)
}

/// Return predicate that matches if request contains specified header and
/// value.
pub fn Header(name: &'static str, value: &'static str) -> HeaderGuard {
    HeaderGuard(
        header::HeaderName::from_static(name),
        header::HeaderValue::from_static(value),
    )
}

#[doc(hidden)]
pub struct HeaderGuard(header::HeaderName, header::HeaderValue);

impl Guard for HeaderGuard {
    fn check(&self, req: &Parts) -> bool {
        if let Some(val) = req.headers.get(&self.0) {
            return val == self.1;
        }
        false
    }
}

pub fn Host<H: AsRef<str>>(host: H) -> HostGuard {
    HostGuard(host.as_ref().to_string(), None)
}

fn get_host_uri(req: &Parts) -> Option<Uri> {
    use core::str::FromStr;
    req.headers
        .get(header::HOST)
        .and_then(|host_value| host_value.to_str().ok())
        .or_else(|| req.uri.host())
        .map(|host: &str| Uri::from_str(host).ok())
        .and_then(|host_success| host_success)
}

#[doc(hidden)]
pub struct HostGuard(String, Option<String>);

impl HostGuard {
    /// Set request scheme to match
    pub fn scheme<H: AsRef<str>>(mut self, scheme: H) -> HostGuard {
        self.1 = Some(scheme.as_ref().to_string());
        self
    }
}

impl Guard for HostGuard {
    fn check(&self, req: &Parts) -> bool {
        let req_host_uri = if let Some(uri) = get_host_uri(req) {
            uri
        } else {
            return false;
        };

        if let Some(uri_host) = req_host_uri.host() {
            if self.0 != uri_host {
                return false;
            }
        } else {
            return false;
        }

        if let Some(ref scheme) = self.1 {
            if let Some(ref req_host_uri_scheme) = req_host_uri.scheme_str() {
                return scheme == req_host_uri_scheme;
            }
        }

        true
    }
}
