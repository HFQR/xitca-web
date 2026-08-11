use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use xitca_service::shutdown::ShutdownToken;

use super::Command;

#[derive(Clone)]
pub struct ServerHandle {
    pub(super) tx: UnboundedSender<Command>,
    pub(super) shutdown_token: Arc<ShutdownToken>,
}

impl ServerHandle {
    /// Stop xitca-server with graceful flag.
    pub fn stop(&self, graceful: bool) {
        let cmd = if graceful {
            Command::GracefulStop
        } else {
            Command::ForceStop
        };

        let _ = self.tx.send(cmd);
        self.shutdown_token.shutdown();
    }
}
