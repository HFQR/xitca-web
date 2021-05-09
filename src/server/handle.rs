use std::future::Future;

use tokio::sync::{mpsc::UnboundedSender, oneshot};

use super::Command;

pub struct ServerHandle {
    pub(super) tx: UnboundedSender<Command>,
}

impl ServerHandle {
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
