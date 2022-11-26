use std::{future::Future, pin::Pin};

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

pub(crate) type SignalFuture = Pin<Box<dyn Future<Output = Signal> + Send>>;

pub(crate) fn start() -> SignalFuture {
    #[cfg(unix)]
    {
        use std::{future::poll_fn, task::Poll};

        use tokio::signal::unix;

        let mut signals = [
            (unix::SignalKind::interrupt(), Signal::Int),
            (unix::SignalKind::hangup(), Signal::Hup),
            (unix::SignalKind::terminate(), Signal::Term),
            (unix::SignalKind::quit(), Signal::Quit),
        ]
        .iter()
        .filter_map(|(kind, sig)| {
            unix::signal(*kind)
                .map(|tokio_sig| (*sig, tokio_sig))
                .map_err(|e| tracing::error!("Can not initialize stream handler for {:?} err: {}", sig, e))
                .ok()
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();

        Box::pin(poll_fn(move |cx| {
            for (sig, fut) in signals.iter_mut() {
                if Pin::new(fut).poll_recv(cx).is_ready() {
                    return Poll::Ready(*sig);
                }
            }
            Poll::Pending
        }))
    }

    #[cfg(not(any(unix, target_family = "wasm")))]
    {
        Box::pin(async {
            let _ = tokio::signal::ctrl_c().await;
            Signal::Int
        })
    }

    #[cfg(target_family = "wasm")]
    {
        Box::pin(std::future::pending())
    }
}
