use std::{
    future::Future,
    io, mem,
    pin::Pin,
    task::{Context, Poll, ready},
};

use crate::signals::{self, Signal, SignalFuture};

use super::{Command, Server, handle::ServerHandle};

/// Future that waits for a shutdown request and then performs the server shutdown.
///
/// Awaiting this future composes [`ServerFuture::run_to_shutdown`] with the returned
/// [`ServerShutdownFuture`]. Use [`ServerFuture::run_to_shutdown`] directly when cleanup must
/// run between receiving the shutdown request and stopping the server.
#[must_use = "ServerFuture must be .await/ spawn as task / consumed with ServerFuture::wait or ServerFuture::run_to_shutdown."]
pub struct ServerFuture {
    state: State,
}

enum State {
    Run(ServerRunFuture),
    ShutDown(ServerShutdownFuture),
    Finished,
}

impl Default for ServerFuture {
    fn default() -> Self {
        Self { state: State::Finished }
    }
}

impl Future for ServerFuture {
    type Output = io::Result<()>;

    #[inline(never)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();

        loop {
            match &mut this.state {
                State::Run(wait) => {
                    let shutdown = match ready!(Pin::new(wait).poll(cx)) {
                        Ok(shutdown) => shutdown,
                        Err(error) => {
                            this.state = State::Finished;
                            return Poll::Ready(Err(error));
                        }
                    };

                    this.state = State::ShutDown(shutdown);
                }
                State::ShutDown(shutdown) => {
                    let result = ready!(Pin::new(shutdown).poll(cx));
                    this.state = State::Finished;
                    return Poll::Ready(result);
                }
                State::Finished => unreachable!("ServerFuture polled after finish"),
            }
        }
    }
}

impl ServerFuture {
    pub(crate) fn new(server: Server, enable_signal: bool) -> Self {
        Self {
            state: State::Run(ServerRunFuture::new(server, enable_signal)),
        }
    }

    pub(crate) fn error(error: io::Error) -> Self {
        Self {
            state: State::Run(ServerRunFuture {
                state: ServerRunFutureState::Error(error),
            }),
        }
    }

    /// Wait for a shutdown request without shutting down the server.
    ///
    /// The returned future owns the server and resolves to a [`ServerShutdownFuture`] when an
    /// OS signal or [`ServerHandle::stop`] request is received. The server remains running until
    /// that shutdown future is awaited, which makes it possible to run cleanup logic in between.
    ///
    /// The shutdown mode is determined by the request that wakes this future. `SIGTERM` and
    /// [`ServerHandle::stop(true)`] request a graceful shutdown. `SIGINT`, `SIGQUIT`, and
    /// [`ServerHandle::stop(false)`] request a forceful shutdown.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use xitca_io::net::TcpStream;
    /// # use xitca_server::Builder;
    /// # use xitca_service::fn_service;
    /// # #[tokio::main]
    /// # async fn main() -> std::io::Result<()> {
    /// let server = Builder::new()
    ///     .bind("test", "127.0.0.1:0", fn_service(|_io: TcpStream| async {
    ///         Ok::<_, ()>(())
    ///     }))?
    ///     .build();
    ///
    /// let shutdown = server.run_to_shutdown().await?;
    /// // Run application cleanup while the server is still running.
    /// shutdown.await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn run_to_shutdown(self) -> ServerRunFuture {
        match self.state {
            State::Run(wait) => wait,
            State::ShutDown(_) => panic!("ServerFuture is already shutting down"),
            State::Finished => panic!("ServerFuture used after finished"),
        }
    }

    /// A handle for mutate Server state.
    ///
    /// # Examples:
    ///
    /// ```rust
    /// # use xitca_io::net::{TcpStream};
    /// # use xitca_server::Builder;
    /// # use xitca_service::fn_service;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut server = Builder::new()
    ///     .bind("test", "127.0.0.1:0", fn_service(|_io: TcpStream| async { Ok::<_, ()>(())}))
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
        match &mut self.state {
            State::Run(run) => {
                let result = run.handle();
                if result.is_err() {
                    self.state = State::Finished;
                }
                result
            }
            State::ShutDown(_) => panic!("ServerFuture is already shutting down"),
            State::Finished => panic!("ServerFuture used after finished"),
        }
    }

    /// Consume ServerFuture and block current thread waitting for server stop.
    ///
    /// Server can be stopped through OS signal or [ServerHandle::stop]. If none is active this call
    /// would block forever.
    pub fn wait(self) -> io::Result<()> {
        match self.state {
            State::Run(run) => run.wait()?.shutdown(),
            State::ShutDown(_) => panic!("ServerFuture is already shutting down"),
            State::Finished => unreachable!(),
        };
        Ok(())
    }
}

/// Future that waits for an OS signal or a [`ServerHandle::stop`] request.
///
/// Awaiting this future returns a [`ServerShutdownFuture`]. The server remains running between
/// the two futures.
#[must_use = "ServerWaitFuture must be awaited to receive a shutdown request."]
pub struct ServerRunFuture {
    state: ServerRunFutureState,
}

#[derive(Default)]
enum ServerRunFutureState {
    Init {
        server: Server,
        enable_signal: bool,
    },
    Running(ServerSignalFuture),
    Error(io::Error),
    #[default]
    Finished,
}

impl ServerRunFuture {
    fn new(server: Server, enable_signal: bool) -> Self {
        Self {
            state: ServerRunFutureState::Init { server, enable_signal },
        }
    }

    fn handle(&mut self) -> io::Result<ServerHandle> {
        match &mut self.state {
            ServerRunFutureState::Init { server, .. } => Ok(ServerHandle {
                tx: server.tx_cmd.clone(),
            }),
            ServerRunFutureState::Running(inner) => Ok(ServerHandle {
                tx: inner.server.tx_cmd.clone(),
            }),
            ServerRunFutureState::Error(_) => match mem::take(&mut self.state) {
                ServerRunFutureState::Error(error) => Err(error),
                _ => unreachable!(),
            },
            ServerRunFutureState::Finished => panic!("ServerWaitFuture used after finished"),
        }
    }

    fn wait(self) -> io::Result<ServerShutdownFuture> {
        match self.state {
            ServerRunFutureState::Init {
                mut server,
                enable_signal,
            } => {
                let rt = server.rt.take().unwrap();

                let func = move || {
                    let (mut server_fut, cmd) = rt.block_on(async {
                        let mut server_fut = ServerSignalFuture::new(server, enable_signal);
                        let cmd = std::future::poll_fn(|cx| server_fut.poll_cmd(cx)).await;
                        (server_fut, cmd)
                    });
                    server_fut.server.rt = Some(rt);
                    (server_fut, cmd)
                };

                let (server_fut, cmd) = match tokio::runtime::Handle::try_current() {
                    Ok(_) => {
                        tracing::warn!(
                            "ServerFuture::wait is called from within tokio context. It would block current thread from handling async tasks."
                        );
                        std::thread::Builder::new()
                            .name(String::from("xitca-server-wait-scoped"))
                            .spawn(func)?
                            .join()
                            .expect("ServerFutureInner unexpected panicing")
                    }
                    Err(_) => func(),
                };

                let ServerSignalFuture { server, .. } = server_fut;
                Ok(ServerShutdownFuture::new(server, cmd))
            }
            ServerRunFutureState::Running(..) => panic!("ServerWaitFuture is already polled."),
            ServerRunFutureState::Error(error) => Err(error),
            ServerRunFutureState::Finished => unreachable!(),
        }
    }
}

impl Future for ServerRunFuture {
    type Output = io::Result<ServerShutdownFuture>;

    #[inline(never)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();

        match &mut this.state {
            ServerRunFutureState::Init { .. } => {
                let state = mem::take(&mut this.state);
                let ServerRunFutureState::Init { server, enable_signal } = state else {
                    unreachable!();
                };

                this.state = ServerRunFutureState::Running(ServerSignalFuture::new(server, enable_signal));
                self.poll(cx)
            }
            ServerRunFutureState::Running(inner) => {
                let cmd = ready!(inner.poll_cmd(cx));
                let state = mem::replace(&mut this.state, ServerRunFutureState::Finished);
                let ServerRunFutureState::Running(inner) = state else {
                    unreachable!();
                };
                let ServerSignalFuture { server, .. } = inner;

                Poll::Ready(Ok(ServerShutdownFuture::new(server, cmd)))
            }
            ServerRunFutureState::Error(_) => match mem::take(&mut this.state) {
                ServerRunFutureState::Error(error) => Poll::Ready(Err(error)),
                _ => unreachable!(),
            },
            ServerRunFutureState::Finished => unreachable!("ServerWaitFuture polled after finish"),
        }
    }
}

/// Future that performs the server shutdown after a shutdown request has been received.
///
/// This future owns the server after [`ServerWaitFuture`] resolves. The server remains running
/// until this future is awaited.
#[must_use = "ServerShutdownFuture must be awaited to shut down the server."]
pub struct ServerShutdownFuture {
    server: Option<Server>,
    graceful: bool,
}

impl ServerShutdownFuture {
    fn new(server: Server, cmd: Command) -> Self {
        Self {
            server: Some(server),
            graceful: cmd.is_graceful(),
        }
    }

    fn shutdown(&mut self) {
        self.server
            .take()
            .expect("ServerShutdownFuture polled after finish")
            .stop(self.graceful);
    }
}

impl Future for ServerShutdownFuture {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().shutdown();
        Poll::Ready(Ok(()))
    }
}

pub struct ServerSignalFuture {
    pub(crate) server: Server,
    pub(crate) signals: Option<SignalFuture>,
}

impl ServerSignalFuture {
    fn new(server: Server, enable_signal: bool) -> Self {
        Self {
            server,
            signals: enable_signal.then(signals::start),
        }
    }

    #[inline(never)]
    fn poll_cmd(&mut self, cx: &mut Context<'_>) -> Poll<Command> {
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

        match ready!(Pin::new(&mut self.server.rx_cmd).poll_recv(cx)) {
            Some(cmd) => Poll::Ready(cmd),
            None => Poll::Pending,
        }
    }
}

impl Command {
    fn is_graceful(&self) -> bool {
        matches!(self, Self::GracefulStop)
    }
}
