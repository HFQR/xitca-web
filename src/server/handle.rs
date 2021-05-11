use std::future::Future;

use tokio::sync::{mpsc::UnboundedSender, oneshot};

use super::Command;

#[derive(Clone)]
pub struct ServerHandle {
    pub(super) tx: UnboundedSender<Command>,
}

impl ServerHandle {
    /// Stop actix-server with graceful flag.
    ///
    /// Stop process starts as soon as method is called.
    /// `await` or `Poll` the return future is for wait for the stop outcome.
    /// (Can be ignored if the shutdown outcome is not needed.)
    pub fn stop(&self, graceful: bool) -> impl Future<Output = ()> {
        let rx = if graceful {
            let (tx, rx) = oneshot::channel();
            let _ = self.tx.send(Command::GracefulStop(tx));

            Some(rx)
        } else {
            let _ = self.tx.send(Command::ForceStop);

            None
        };

        async {
            if let Some(rx) = rx {
                let _ = rx.await;
            }
        }
    }
}
