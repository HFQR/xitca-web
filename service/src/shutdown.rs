//! traits decoupling application shutdown signalling from any async runtime.
//!
//! a [`ShutdownEmitter`] triggers shutdown and every [`ShutdownListener`] handed out
//! beforehand resolves its future in response. neither trait is bound to a specific
//! runtime: an implementor only has to produce a [`Future`] that resolves once shutdown
//! is signalled.

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, sync::Arc};
use core::future::Future;
#[cfg(feature = "alloc")]
use core::pin::Pin;

/// emitter side of a shutdown signal.
///
/// calling [`shutdown`](ShutdownEmitter::shutdown) resolves the future of every
/// [`ShutdownListener`] derived from the same source.
pub trait ShutdownEmitter {
    /// signal shutdown. every associated [`ShutdownListener`] is resolved as a result.
    fn shutdown(&self);
}

/// listener side of a shutdown signal.
///
/// a listener is [`Clone`] and [`Send`] so the same shutdown source can be observed by
/// services running on different threads. [`wait`](ShutdownListener::wait) consumes the
/// listener and returns a future resolving once the associated [`ShutdownEmitter`]
/// signals shutdown. the resolved value carries no information: observing the resolution
/// is the signal.
pub trait ShutdownListener: Clone + Send {
    /// consume the listener and wait for shutdown to be signalled.
    fn wait(self) -> impl Future<Output = ()> + Send;
}

/// a type erased, cheaply cloneable [`ShutdownListener`].
///
/// [`ShutdownListener`] is not object safe (it is generic over the future it returns and
/// requires [`Clone`]), so it can not be stored as a `dyn` trait object directly. wrap any
/// listener in a `BoxShutdownListener` to store it in a type that must stay free of generic
/// parameters (e.g. a configuration struct). the concrete listener is erased behind a
/// reference counted trait object so cloning only bumps the reference count.
#[cfg(feature = "alloc")]
#[derive(Clone)]
pub struct BoxShutdownListener {
    inner: Arc<dyn DynShutdownListener>,
}

#[cfg(feature = "alloc")]
impl BoxShutdownListener {
    /// erase `listener` behind a [`BoxShutdownListener`].
    pub fn new(listener: impl ShutdownListener + Sync + 'static) -> Self {
        Self { inner: Arc::new(listener) }
    }
}

#[cfg(feature = "alloc")]
impl ShutdownListener for BoxShutdownListener {
    fn wait(self) -> impl Future<Output = ()> + Send {
        self.inner.wait()
    }
}

/// object safe counterpart of [`ShutdownListener`] used to erase the listener type behind
/// [`BoxShutdownListener`]. `wait` takes `&self` and clones internally so the trait object
/// stays shareable, and the future is boxed to escape the associated type.
#[cfg(feature = "alloc")]
trait DynShutdownListener: Send + Sync {
    fn wait(&self) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

#[cfg(feature = "alloc")]
impl<L> DynShutdownListener for L
where
    L: ShutdownListener + Sync + 'static,
{
    fn wait(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(self.clone().wait())
    }
}
