//! a new type lock share the same api between `RefCell` and `Mutex`.

#[cfg(feature = "single-thread")]
mod types {
    pub type Lock<T> = std::cell::RefCell<T>;
    pub type LockGuard<'a, T> = std::cell::RefMut<'a, T>;
}

#[cfg(not(feature = "single-thread"))]
mod types {
    pub type Lock<T> = std::sync::Mutex<T>;
    pub type LockGuard<'a, T> = std::sync::MutexGuard<'a, T>;
}

pub struct Lock<T>(types::Lock<T>);

impl<T> Lock<T> {
    #[inline]
    pub fn new(val: T) -> Self {
        Self(types::Lock::new(val))
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        #[cfg(not(feature = "single-thread"))]
        {
            self.0.get_mut().unwrap()
        }
        #[cfg(feature = "single-thread")]
        {
            self.0.get_mut()
        }
    }

    #[inline]
    pub fn lock(&self) -> types::LockGuard<'_, T> {
        #[cfg(not(feature = "single-thread"))]
        {
            self.0.lock().unwrap()
        }
        #[cfg(feature = "single-thread")]
        {
            self.0.borrow_mut()
        }
    }
}
