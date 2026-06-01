//! a [`tokio::sync::watch`] backed implementation of the runtime agnostic shutdown
//! traits from [`xitca_service::shutdown`].

use tokio::sync::watch;
use xitca_service::shutdown::{ShutdownEmitter, ShutdownListener};

/// A cloneable shutdown listener. Pass one (or a clone) to
/// [`HttpServiceConfig::shutdown`](crate::config::HttpServiceConfig::shutdown) for every
/// service that should observe the same shutdown signal.
#[derive(Clone)]
pub struct ShutdownToken {
    receiver: watch::Receiver<bool>,
}

impl ShutdownToken {
    /// Create a new `ShutdownToken` together with the [`ShutdownHandle`] that can trigger
    /// it. Clone the returned token to hand it to multiple services.
    pub fn new() -> (Self, ShutdownHandle) {
        let (sender, receiver) = watch::channel(false);
        (Self { receiver }, ShutdownHandle { sender })
    }
}

impl ShutdownListener for ShutdownToken {
    async fn wait(self) {
        let mut receiver = self.receiver;
        // resolve as soon as the sender flips the value or is dropped. the concrete value
        // is irrelevant: any change means shutdown was signalled.
        let _ = receiver.changed().await;
    }
}

/// Triggers shutdown across every [`ShutdownToken`] cloned from the same source.
///
/// Dropping this handle does **not** trigger shutdown; call
/// [`shutdown`](ShutdownEmitter::shutdown) explicitly.
pub struct ShutdownHandle {
    sender: watch::Sender<bool>,
}

impl ShutdownEmitter for ShutdownHandle {
    fn shutdown(&self) {
        // Ignore the error: it only occurs when all receivers have been dropped,
        // which means there is nothing left to notify.
        let _ = self.sender.send(true);
    }
}
