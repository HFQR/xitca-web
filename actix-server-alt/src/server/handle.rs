use tokio::sync::mpsc::UnboundedSender;

use super::Command;

#[derive(Clone)]
pub struct ServerHandle {
    pub(super) tx: UnboundedSender<Command>,
}

impl ServerHandle {
    /// Stop actix-server with graceful flag.
    pub fn stop(&self, graceful: bool) {
        let cmd = if graceful {
            Command::GracefulStop
        } else {
            Command::ForceStop
        };

        let _ = self.tx.send(cmd);
    }
}
