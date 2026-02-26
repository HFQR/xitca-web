use std::{
    future::Future,
    io, mem,
    pin::Pin,
    task::{Context, Poll, ready},
};

use crate::signals::{self, Signal, SignalFuture};

use super::{Command, Server, handle::ServerHandle};

#[must_use = "ServerFuture must be .await/ spawn as task / consumed with ServerFuture::wait."]
#[derive(Default)]
pub enum ServerFuture {
    Init {
        server: Server,
        enable_signal: bool,
    },
    Running(ServerFutureInner),
    Error(io::Error),
    #[default]
    Finished,
}

impl ServerFuture {
    /// A handle for mutate Server state.
    ///
    /// # Examples:
    ///
    /// ```rust
    /// # use xitca_io::net::{TcpStream};
    /// # use xitca_server::Builder;
    /// # use xitca_service::fn_service;
    /// # use tokio_util::sync::CancellationToken;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut server = Builder::new()
    ///     .bind("test", "127.0.0.1:0", fn_service(|(_io, _token): (TcpStream, CancellationToken)| async { Ok::<_, ()>(())}))
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
                cancellation_token: server.cancellation_token.clone(),
            }),
            Self::Running(ref inner) => Ok(ServerHandle {
                tx: inner.server.tx_cmd.clone(),
                cancellation_token: inner.server.cancellation_token.clone(),
            }),
            Self::Error(_) => match mem::take(self) {
                Self::Error(e) => Err(e),
                _ => unreachable!(),
            },
            Self::Finished => panic!("ServerFuture used after finished"),
        }
    }

    /// Consume ServerFuture and block current thread waitting for server stop.
    ///
    /// Server can be stopped through OS signal or [ServerHandle::stop]. If none is active this call
    /// would block forever.
    pub fn wait(self) -> io::Result<()> {
        match self {
            Self::Init {
                mut server,
                enable_signal,
            } => {
                let rt = server.rt.take().unwrap();

                let func = move || {
                    let (mut server_fut, cmd) = rt.block_on(async {
                        let mut server_fut = ServerFutureInner::new(server, enable_signal);
                        let cmd = std::future::poll_fn(|cx| server_fut.poll_cmd(cx)).await;
                        (server_fut, cmd)
                    });
                    server_fut.server.rt = Some(rt);
                    (server_fut, cmd)
                };

                let (mut server_fut, cmd) = match tokio::runtime::Handle::try_current() {
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

                server_fut.handle_cmd(cmd);
                Ok(())
            }
            Self::Running(..) => panic!("ServerFuture is already polled."),
            Self::Error(e) => Err(e),
            Self::Finished => unreachable!(),
        }
    }
}

pub struct ServerFutureInner {
    pub(crate) server: Server,
    pub(crate) signals: Option<SignalFuture>,
}

impl ServerFutureInner {
    #[inline(never)]
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
