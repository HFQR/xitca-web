use core::ops::{Deref, DerefMut};

use std::{
    mem::{self, ManuallyDrop},
    thread::{self, ThreadId},
};

/// thread id guarded wrapper for `!Send` B type to make it `Send` bound.
/// It is safe to transfer NoSendSend between threads and panic when actual access to B happens on
/// threads other than the one it's constructed from.
pub struct NoSendSend<B> {
    id: ThreadId,
    inner: ManuallyDrop<B>,
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
            inner: ManuallyDrop::new(inner),
        }
    }

    /// Obtain ownership of B.
    ///
    /// # Panics:
    /// - When called from a thread not where B is originally constructed.
    pub fn into_inner(self) -> B {
        self.assert_thread_id();

        // forget self so Drop does not run.
        let mut this = ManuallyDrop::new(self);

        // SAFETY:
        // function take Self by value leave no other owner/borrower.
        unsafe { ManuallyDrop::take(&mut this.inner) }
    }

    fn assert_thread_id(&self) {
        assert_eq!(thread::current().id(), self.id);
    }
}

impl<B> Deref for NoSendSend<B> {
    type Target = B;

    fn deref(&self) -> &Self::Target {
        self.assert_thread_id();
        self.inner.deref()
    }
}

impl<B> DerefMut for NoSendSend<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.assert_thread_id();
        self.inner.deref_mut()
    }
}

impl<B> Drop for NoSendSend<B> {
    fn drop(&mut self) {
        // drop B when possible. leak the memory if not.
        if mem::needs_drop::<B>() && thread::current().id() == self.id {
            // SAFETY:
            // B needs drop and is not moved to other threads.
            unsafe {
                ManuallyDrop::drop(&mut self.inner);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    #[should_panic]
    fn test_move() {
        let send = NoSendSend::new(std::rc::Rc::new(123));

        let _ = std::thread::spawn(move || {
            &*send;
        })
        .join()
        .unwrap();
    }
}
