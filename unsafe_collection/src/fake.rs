use core::{
    mem::{self, ManuallyDrop},
    ops::{Deref, DerefMut},
};

use std::thread::{self, ThreadId};

/// thread id guarded wrapper for `!Send` type to make it `Send` bound.
///
/// It is safe to transfer FakeSend between threads and panic when actual access happens on threads
/// other than the one it's constructed from.
///
/// Drop `FakeSend<B>` on a foreign thread will not trigger de structure of B(if it has one). The
/// memory of B allocation would be leaked.
pub struct FakeSend<B> {
    id: ThreadId,
    inner: ManuallyDrop<B>,
}

// SAFETY:
// safe to impl Send bound as B type never get accessed from other threads it constructed from.
unsafe impl<B> Send for FakeSend<B> {}

impl<B> FakeSend<B> {
    /// Construct a new Send type from B. preferably B is bound to `!Send`.
    ///
    /// # Examples:
    /// ```no_run
    /// # use xitca_unsafe_collection::fake::FakeSend;
    ///
    /// // make a "Send" Rc.
    /// let inner = std::rc::Rc::new(123);
    /// let send = FakeSend::new(inner);
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

    /// Obtain ownership of inner value.
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

impl<B> Deref for FakeSend<B> {
    type Target = B;

    fn deref(&self) -> &Self::Target {
        self.assert_thread_id();
        self.inner.deref()
    }
}

impl<B> DerefMut for FakeSend<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.assert_thread_id();
        self.inner.deref_mut()
    }
}

impl<B> Drop for FakeSend<B> {
    fn drop(&mut self) {
        // drop B when possible. leak B if not.
        if mem::needs_drop::<B>() && thread::current().id() == self.id {
            // SAFETY:
            // B needs drop and is not moved to other threads.
            unsafe {
                ManuallyDrop::drop(&mut self.inner);
            }
        }
    }
}

// TODO: remove when ::core::sync::Exclusive stabled.
pub struct FakeSync<B>(B);

// SAFETY:
// safe to impl Sync bound as FakeSync can not be used to dereference to &B nor &mut B.
unsafe impl<B> Sync for FakeSync<B> {}

impl<B> FakeSync<B> {
    /// Construct a new Sync type from B. preferably B is bound to `!Sync`.
    #[inline]
    pub const fn new(inner: B) -> Self {
        Self(inner)
    }

    /// Obtain ownership of inner value.
    #[inline]
    pub fn into_inner(self) -> B {
        self.0
    }
}

pub struct FakeClone<B>(B);

impl<B> Clone for FakeClone<B> {
    fn clone(&self) -> Self {
        panic!("Fake Clone can not be cloned")
    }
}

impl<B> FakeClone<B> {
    /// Construct a new Clone type from B. preferably B is bound to `!Clone`.
    #[inline]
    pub const fn new(inner: B) -> Self {
        Self(inner)
    }

    /// Obtain ownership of inner value.
    #[inline]
    pub fn into_inner(self) -> B {
        self.0
    }
}
