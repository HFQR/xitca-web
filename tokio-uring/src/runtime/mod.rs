use std::future::Future;
use std::io;
use tokio::io::unix::AsyncFd;
use tokio::task::LocalSet;

mod context;
pub(crate) mod driver;

pub(crate) use context::RuntimeContext;

thread_local! {
    pub(crate) static CONTEXT: RuntimeContext = RuntimeContext::new();
}

#[cfg(not(tokio_unstable))]
type TokioRt = tokio::runtime::Runtime;

#[cfg(tokio_unstable)]
type TokioRt = tokio::runtime::LocalRuntime;

/// The Runtime Executor
///
/// This is the Runtime for `tokio-uring`.
/// It wraps the default [`Runtime`] using the platform-specific Driver.
///
/// This executes futures and tasks within the current-thread only.
///
/// [`Runtime`]: tokio::runtime::Runtime
pub struct Runtime {
    // field drop order matters. LocalSet must be dropped before TokioRT
    #[cfg(not(tokio_unstable))]
    /// LocalSet for !Send tasks
    local: LocalSet,

    /// Tokio runtime, always current-thread
    tokio_rt: TokioRt,

    /// Strong reference to the driver.
    driver: driver::Handle,
}

/// Spawns a new asynchronous task, returning a [`JoinHandle`] for it.
///
/// Spawning a task enables the task to execute concurrently to other tasks.
/// There is no guarantee that a spawned task will execute to completion. When a
/// runtime is shutdown, all outstanding tasks are dropped, regardless of the
/// lifecycle of that task.
///
/// This function must be called from the context of a `tokio-uring` runtime.
///
/// [`JoinHandle`]: tokio::task::JoinHandle
///
/// # Examples
///
/// In this example, a server is started and `spawn` is used to start a new task
/// that processes each received connection.
///
/// ```no_run
/// tokio_uring_xitca::start(async {
///     let handle = tokio_uring_xitca::spawn(async {
///         println!("hello from a background task");
///     });
///
///     // Let the task complete
///     handle.await.unwrap();
/// });
/// ```
pub fn spawn<T: Future + 'static>(task: T) -> tokio::task::JoinHandle<T::Output> {
    tokio::task::spawn_local(task)
}

impl Runtime {
    /// Creates a new tokio_uring runtime on the current thread.
    ///
    /// This takes the tokio-uring [`Builder`](crate::Builder) as a parameter.
    pub fn new(b: &crate::Builder) -> io::Result<Runtime> {
        let mut builder = tokio::runtime::Builder::new_current_thread();
        builder
            .on_thread_park(|| {
                CONTEXT.with(|x| {
                    let _ = x
                        .handle()
                        .expect("Internal error, driver context not present when invoking hooks")
                        .flush();
                });
            })
            .enable_all();

        #[cfg(tokio_unstable)]
        let tokio_rt = builder.build_local(Default::default())?;

        #[cfg(not(tokio_unstable))]
        let tokio_rt = builder.build()?;

        let _local = LocalSet::new();

        let driver = driver::Handle::new(b)?;

        start_uring_wakes_task(&tokio_rt, &_local, driver.clone());

        Ok(Runtime {
            #[cfg(not(tokio_unstable))]
            local: _local,
            tokio_rt,
            driver,
        })
    }

    /// Runs a future to completion on the tokio-uring runtime. This is the
    /// runtime's entry point.
    ///
    /// This runs the given future on the current thread, blocking until it is
    /// complete, and yielding its resolved result. Any tasks, futures, or timers
    /// which the future spawns internally will be executed on this runtime.
    ///
    /// Any spawned tasks will be suspended after `block_on` returns. Calling
    /// `block_on` again will resume previously spawned tasks.
    ///
    /// # Panics
    ///
    /// This function panics if the provided future panics, or if called within an
    /// asynchronous execution context.
    /// Runs a future to completion on the current runtime.
    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        struct ContextGuard;

        impl Drop for ContextGuard {
            fn drop(&mut self) {
                CONTEXT.with(|cx| cx.unset_driver());
            }
        }

        CONTEXT.with(|cx| cx.set_handle(self.driver.clone()));

        let _guard = ContextGuard;

        tokio::pin!(future);

        let task = std::future::poll_fn(|cx| {
            // assert!(drive.as_mut().poll(cx).is_pending());
            future.as_mut().poll(cx)
        });

        #[cfg(not(tokio_unstable))]
        let task = self.local.run_until(task);

        self.tokio_rt.block_on(task)
    }
}

fn start_uring_wakes_task(tokio_rt: &TokioRt, _local: &LocalSet, driver: driver::Handle) {
    let _guard = tokio_rt.enter();
    let async_driver_handle = AsyncFd::new(driver).unwrap();

    let task = drive_uring_wakes(async_driver_handle);

    #[cfg(not(tokio_unstable))]
    _local.spawn_local(task);

    #[cfg(tokio_unstable)]
    tokio::task::spawn_local(task);
}

async fn drive_uring_wakes(driver: AsyncFd<driver::Handle>) {
    loop {
        // Wait for read-readiness
        let mut guard = driver.readable().await.unwrap();

        guard.get_inner().dispatch_completions();

        guard.clear_ready();
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use crate::builder;

    #[test]
    fn block_on() {
        let rt = Runtime::new(&builder()).unwrap();
        rt.block_on(async {});
    }

    #[test]
    fn block_on_twice() {
        let rt = Runtime::new(&builder()).unwrap();
        rt.block_on(async {});
        rt.block_on(async {});
    }
}
