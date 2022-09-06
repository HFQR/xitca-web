use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Different types of process signals
#[allow(dead_code)]
#[derive(Eq, PartialEq, Clone, Copy, Debug)]
pub(crate) enum Signal {
    /// SIGHUP
    Hup,
    /// SIGINT
    Int,
    /// SIGTERM
    Term,
    /// SIGQUIT
    Quit,
}

pub(crate) struct Signals {
    #[cfg(unix)]
    signals: Vec<(Signal, tokio::signal::unix::Signal)>,
    #[cfg(not(unix))]
    signals: Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>,
}

impl Signals {
    #[cfg(unix)]
    pub(crate) fn start() -> Self {
        use tokio::signal::unix;

        let sig_map = [
            (unix::SignalKind::interrupt(), Signal::Int),
            (unix::SignalKind::hangup(), Signal::Hup),
            (unix::SignalKind::terminate(), Signal::Term),
            (unix::SignalKind::quit(), Signal::Quit),
        ];

        let signals = sig_map
            .iter()
            .filter_map(|(kind, sig)| {
                unix::signal(*kind)
                    .map(|tokio_sig| (*sig, tokio_sig))
                    .map_err(|e| tracing::error!("Can not initialize stream handler for {:?} err: {}", sig, e))
                    .ok()
            })
            .collect::<Vec<_>>();

        Self { signals }
    }

    #[cfg(not(any(unix, target_family = "wasm")))]
    pub(crate) fn start() -> Self {
        Self {
            signals: Box::pin(tokio::signal::ctrl_c()),
        }
    }

    #[cfg(target_family = "wasm")]
    pub(crate) fn start() -> Self {
        Self {
            signals: Box::pin(std::future::pending()),
        }
    }
}

impl Future for Signals {
    type Output = Signal;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(not(unix))]
        {
            self.signals.as_mut().poll(cx).map(|_| Signal::Int)
        }
        #[cfg(unix)]
        {
            for (sig, fut) in self.signals.iter_mut() {
                if Pin::new(fut).poll_recv(cx).is_ready() {
                    return Poll::Ready(*sig);
                }
            }
            Poll::Pending
        }
    }
}
