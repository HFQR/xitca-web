use std::{cell::Cell, rc::Rc};

#[derive(Clone)]
pub(crate) struct Counter(Rc<Cell<usize>>);

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

impl Counter {
    pub(crate) fn new() -> Self {
        Self(Rc::new(Cell::new(0)))
    }

    pub(crate) fn guard(&self) -> CounterGuard {
        self.0.set(self.0.get() + 1);
        CounterGuard(self.0.clone())
    }

    pub(super) fn get(&self) -> usize {
        self.0.get()
    }
}

pub(crate) struct CounterGuard(Rc<Cell<usize>>);

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn counter() {
        let counter = Counter::new();

        let guard = counter.guard();
        let guard2 = counter.guard();

        assert_eq!(counter.get(), 2);

        drop(guard2);
        assert_eq!(counter.get(), 1);

        drop(guard);
        assert_eq!(counter.get(), 0);
    }
}
