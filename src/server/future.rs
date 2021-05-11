use std::{
    future::Future,
    io, mem,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::{future::BoxFuture, ready};

use super::handle::ServerHandle;
use super::{Command, Server};

#[must_use = "ServerFuture must be .await or spawn as task."]
pub enum ServerFuture {
    Server(ServerFutureInner),
    Error(io::Error),
    Shutdown(BoxFuture<'static, io::Result<()>>),
    Finished,
}

impl ServerFuture {
    /// A handle for mutate Server state.
    ///
    /// # Examples:
    ///
    /// ```rust
    /// # use actix_server_alt::Builder;
    /// # use actix_service::fn_service;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut server = Builder::new()
    ///     .bind("test", "127.0.0.1:0", || fn_service(|_| async { Ok::<_, ()>(())}))
    ///     .unwrap()
    ///     .build();
    ///
    /// // obtain a handle. if server fail to start a std::io::Error would return.
    /// let handle = server.handle().unwrap();
    ///
    /// // spawn server future.
    /// tokio::spawn(server);
    ///
    /// // do a graceful shutdown of server.
    /// handle.stop(true).await
    /// # }
    /// ```
    pub fn handle(&mut self) -> io::Result<ServerHandle> {
        match *self {
            Self::Server(ref inner) => Ok(ServerHandle {
                tx: inner.server.tx_cmd.clone(),
            }),
            Self::Error(_) => match mem::take(self) {
                Self::Error(e) => Err(e),
                _ => unreachable!(),
            },
            Self::Shutdown(_) => panic!("ServerFuture used during shutdown"),
            Self::Finished => panic!("ServerFuture used after finished"),
        }
    }
}

pub struct ServerFutureInner {
    pub(crate) server: Server,
    #[cfg(feature = "signal")]
    pub(crate) signals: Option<crate::signals::Signals>,
}

impl Default for ServerFuture {
    fn default() -> Self {
        Self::Finished
    }
}

impl ServerFutureInner {
    #[cfg(feature = "signal")]
    fn poll_signal(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if let Some(signals) = self.signals.as_mut() {
            if let Poll::Ready(sig) = Pin::new(signals).poll(cx) {
                log::info!("Signal {:?} received.", sig);
                return Poll::Ready(());
            }
        }

        Poll::Pending
    }

    fn poll_cmd(&mut self, cx: &mut Context<'_>) -> Poll<BoxFuture<'static, io::Result<()>>> {
        match ready!(Pin::new(&mut self.server.rx_cmd).poll_recv(cx)) {
            Some(cmd) => match cmd {
                Command::ForceStop => Poll::Ready(Box::pin(async { Ok(()) })),
                Command::GracefulStop(tx) => Poll::Ready(Box::pin(async {
                    let _ = tx.send(());
                    Ok(())
                })),
            },
            None => Poll::Pending,
        }
    }
}

impl Future for ServerFuture {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        match *this {
            Self::Error(_) => match mem::take(this) {
                Self::Error(e) => Poll::Ready(Err(e)),
                _ => unreachable!("Can not happen"),
            },
            Self::Server(ref mut inner) => {
                #[cfg(feature = "signal")]
                {
                    if inner.poll_signal(cx).is_ready() {
                        return Poll::Ready(Ok(()));
                    }
                }

                let shutdown = ready!(inner.poll_cmd(cx));
                self.set(Self::Shutdown(shutdown));
                self.poll(cx)
            }
            Self::Shutdown(ref mut fut) => fut.as_mut().poll(cx),
            Self::Finished => unreachable!("ServerFuture polled after finish"),
        }
    }
}
