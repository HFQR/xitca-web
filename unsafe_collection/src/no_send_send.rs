use core::ops::{Deref, DerefMut};

use std::thread::{self, ThreadId};

/// thread id guarded wrapper for `!Send` B type to make it `Send` bound.
/// It is safe to transfer NoSendSend between threads and panic when actual access to B happens on
/// threads other than the one it's constructed from.
pub struct NoSendSend<B> {
    id: ThreadId,
    inner: B,
}

// SAFETY:
// safe to impl Send bound as B type never get accessed from other threads it constructed from.
unsafe impl<B> Send for NoSendSend<B> {}

impl<B> NoSendSend<B> {
    /// Construct a new Send type from B. preferably B is bound to `!Send`.
    ///
    /// # Examples:
    /// ```rust
    /// # use xitca_unsafe_collection::no_send_send::NoSendSend;
    ///
    /// // make a "Send" Rc.
    /// let inner = std::rc::Rc::new(123);
    /// let send = NoSendSend::new(inner);
    ///
    /// // move between threads.
    /// let send = std::thread::spawn(move || {
    ///     let send = send;
    ///     send
    /// })
    /// .join()
    /// .unwrap();
    ///
    /// // panic when referencing from other thread.
    /// std::thread::spawn(move || {
    ///     let send = send;
    ///     let _r = &**send;
    /// })
    /// .join()
    /// .unwrap_err();
    /// ```
    pub fn new(inner: B) -> Self {
        Self {
            id: thread::current().id(),
            inner,
        }
    }

    /// Obtain ownership of B.
    ///
    /// # Panics:
    /// - When called from a thread not where B is originally constructed.
    pub fn into_inner(self) -> B {
        self.assert_thread_id();
        self.inner
    }

    fn assert_thread_id(&self) {
        assert_eq!(thread::current().id(), self.id);
    }
}

impl<B> Deref for NoSendSend<B> {
    type Target = B;

    fn deref(&self) -> &Self::Target {
        self.assert_thread_id();
        &self.inner
    }
}

impl<B> DerefMut for NoSendSend<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.assert_thread_id();
        &mut self.inner
    }
}
