use std::{
    future::Future,
    io, mem,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::ready;

use super::{handle::ServerHandle, Command, Server};

#[must_use = "ServerFuture must be .await or spawn as task."]
pub enum ServerFuture {
    Init { server: Server, enable_signal: bool },
    Running(ServerFutureInner),
    Error(io::Error),
    Finished,
}

impl ServerFuture {
    /// A handle for mutate Server state.
    ///
    /// # Examples:
    ///
    /// ```rust
    /// # use xitca_io::net::TcpStream;
    /// # use xitca_server::Builder;
    /// # use xitca_service::fn_service;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut server = Builder::new()
    ///     .bind("test", "127.0.0.1:0", || fn_service(|_io: TcpStream| async { Ok::<_, ()>(())}))
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
            Self::Init { ref server, .. } => Ok(ServerHandle {
                tx: server.tx_cmd.clone(),
            }),
            Self::Running(ref inner) => Ok(ServerHandle {
                tx: inner.server.tx_cmd.clone(),
            }),
            Self::Error(_) => match mem::take(self) {
                Self::Error(e) => Err(e),
                _ => unreachable!(),
            },
            Self::Finished => panic!("ServerFuture used after finished"),
        }
    }

    #[cfg(feature = "signal")]
    pub fn wait(self) -> io::Result<()> {
        match self {
            Self::Init {
                mut server,
                enable_signal,
            } => match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle.block_on(Self::Running(ServerFutureInner::new(server, enable_signal))),
                Err(_) => {
                    let rt = server.rt.take().unwrap();
                    let (mut inner, cmd) = rt.block_on(async {
                        let mut inner = ServerFutureInner::new(server, enable_signal);
                        let cmd = xitca_unsafe_collection::futures::poll_fn(|cx| inner.poll_cmd(cx)).await;
                        (inner, cmd)
                    });
                    inner.server.rt = Some(rt);
                    inner.handle_cmd(cmd);
                    Ok(())
                }
            },
            Self::Running(..) => panic!("ServerFuture is already polled."),
            Self::Error(e) => Err(e),
            Self::Finished => unreachable!(),
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
    #[inline(never)]
    fn new(server: Server, _enable_signal: bool) -> Self {
        Self {
            server,
            #[cfg(feature = "signal")]
            signals: _enable_signal.then(crate::signals::Signals::start),
        }
    }

    #[inline(never)]
    fn poll_cmd(&mut self, cx: &mut Context<'_>) -> Poll<Command> {
        #[cfg(feature = "signal")]
        {
            let p = self.poll_signal(cx);
            if p.is_ready() {
                return p;
            }
        }

        match ready!(Pin::new(&mut self.server.rx_cmd).poll_recv(cx)) {
            Some(cmd) => Poll::Ready(cmd),
            None => Poll::Pending,
        }
    }

    #[inline(never)]
    #[cfg(feature = "signal")]
    fn poll_signal(&mut self, cx: &mut Context<'_>) -> Poll<Command> {
        use crate::signals::Signal;
        if let Some(signals) = self.signals.as_mut() {
            if let Poll::Ready(sig) = Pin::new(signals).poll(cx) {
                tracing::info!("Signal {:?} received.", sig);
                let cmd = match sig {
                    Signal::Int | Signal::Quit => Command::ForceStop,
                    Signal::Term => Command::GracefulStop,
                    // Remove signal listening and keep Server running when
                    // terminal closed which xitca-server process belong.
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

    #[inline(never)]
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
}

impl Future for ServerFuture {
    type Output = io::Result<()>;

    #[inline(never)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        match *this {
            Self::Init { .. } => match mem::take(this) {
                Self::Init { server, enable_signal } => {
                    self.set(Self::Running(ServerFutureInner::new(server, enable_signal)));
                    self.poll(cx)
                }
                _ => unreachable!(),
            },
            Self::Running(ref mut inner) => {
                let cmd = ready!(inner.poll_cmd(cx));
                inner.handle_cmd(cmd);
                self.set(Self::Finished);
                Poll::Ready(Ok(()))
            }
            Self::Error(_) => match mem::take(this) {
                Self::Error(e) => Poll::Ready(Err(e)),
                _ => unreachable!(""),
            },
            Self::Finished => unreachable!("ServerFuture polled after finish"),
        }
    }
}
