use std::task::Poll;

use super::poll_fn::poll_fn;

/// An async function that never resolve to the output.
pub(crate) async fn never<T>() -> T {
    poll_fn(|_| Poll::Pending).await
}
