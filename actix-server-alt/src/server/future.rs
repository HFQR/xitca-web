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
    /// # use actix_service_alt::fn_factory;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut server = Builder::new()
    ///     .bind("test", "127.0.0.1:0", || fn_factory(|_| async { Ok::<_, ()>(())}))
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
    /// handle.stop(true);
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
    fn poll_signal(&mut self, cx: &mut Context<'_>) -> Poll<Command> {
        use crate::signals::Signal;
        if let Some(signals) = self.signals.as_mut() {
            if let Poll::Ready(sig) = Pin::new(signals).poll(cx) {
                log::info!("Signal {:?} received.", sig);
                let cmd = match sig {
                    Signal::Int | Signal::Quit => Command::ForceStop,
                    Signal::Term => Command::GracefulStop,
                    // Remove signal listening and keep Server running when
                    // terminal closed which actix-server-alt process belong.
                    Signal::Hup => {
                        self.signals = None;
                        return Poll::Pending;
                    }
                };
                return Poll::Ready(cmd);
            }
        }

        Poll::Pending
    }

    fn poll_cmd(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        match ready!(Pin::new(&mut self.server.rx_cmd).poll_recv(cx)) {
            Some(cmd) => {
                self.handle_cmd(cmd);
                Poll::Ready(())
            }
            None => Poll::Pending,
        }
    }

    fn handle_cmd(&mut self, cmd: Command) {
        match cmd {
            Command::ForceStop => {
                self.server.stop(false);
            }
            Command::GracefulStop => {
                self.server.stop(true);
            }
        }
    }

    fn on_stop(&mut self) -> BoxFuture<'static, io::Result<()>> {
        Box::pin(async { Ok(()) })
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
                    if let Poll::Ready(cmd) = inner.poll_signal(cx) {
                        inner.handle_cmd(cmd);
                        let task = inner.on_stop();
                        self.set(Self::Shutdown(task));
                        return self.poll(cx);
                    }
                }

                ready!(inner.poll_cmd(cx));
                let task = inner.on_stop();
                self.set(Self::Shutdown(task));
                self.poll(cx)
            }
            Self::Shutdown(ref mut fut) => fut.as_mut().poll(cx),
            Self::Finished => unreachable!("ServerFuture polled after finish"),
        }
    }
}
