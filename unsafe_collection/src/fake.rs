#[cfg(feature = "std")]
mod fake_send;

#[cfg(feature = "std")]
pub use fake_send::FakeSend;

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
