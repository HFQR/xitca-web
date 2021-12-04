mod body;
mod state;

pub use self::state::State;

use std::ops::{Deref, DerefMut};

pub struct Extract<T>(T);

impl<T> Extract<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Extract<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Extract<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
