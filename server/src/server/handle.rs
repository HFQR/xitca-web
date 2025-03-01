use super::Command;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ServerHandle {
    pub(super) tx: UnboundedSender<Command>,
    pub(super) cancellation_token: CancellationToken,
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
        self.cancellation_token.cancel();
    }
}
