/// Shared slab-backed storage for frames across all streams, avoiding
/// per-stream heap allocation. Mirrors `h2::proto::streams::buffer::Buffer`.
pub(super) struct FrameBuffer<T> {
    entries: Vec<Entry<T>>,
    /// Head of the free list (indices of vacant slots), or `usize::MAX` if none.
    next_free: usize,
    len: usize,
}

/// A slab entry. `Vacant` carries the next free-list index; `Occupied` carries
/// the value together with the next index in its owning [`Deque`]. Unifying
/// the free-list link with the deque link means vacant slots pay nothing for
/// the absent value and occupied slots pay nothing for a dead free pointer.
enum Entry<T> {
    Vacant(usize),
    Occupied { value: T, next: usize },
}

/// A per-stream linked-list deque backed by a shared [`FrameBuffer`].
/// Stores only head/tail indices — zero per-stream allocation.
pub(super) struct Deque {
    head: usize,
    tail: usize,
}

const NONE: usize = usize::MAX;

impl<T> FrameBuffer<T> {
    pub(super) const fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_free: NONE,
            len: 0,
        }
    }

    /// Insert a value into the slab, returning its index.
    fn insert(&mut self, value: T) -> usize {
        self.len += 1;
        if self.next_free != NONE {
            let idx = self.next_free;
            match &self.entries[idx] {
                Entry::Vacant(next) => self.next_free = *next,
                Entry::Occupied { .. } => unreachable!("free-list slot is occupied"),
            }
            self.entries[idx] = Entry::Occupied { value, next: NONE };
            idx
        } else {
            let idx = self.entries.len();
            self.entries.push(Entry::Occupied { value, next: NONE });
            idx
        }
    }

    /// Remove a value by index, returning it and placing the slot on the free list.
    fn remove(&mut self, idx: usize) -> T {
        self.len -= 1;
        let entry = core::mem::replace(&mut self.entries[idx], Entry::Vacant(self.next_free));
        self.next_free = idx;
        match entry {
            Entry::Occupied { value, .. } => value,
            Entry::Vacant(_) => unreachable!("removed slot was already vacant"),
        }
    }
}

impl Deque {
    pub(super) fn new() -> Self {
        Self { head: NONE, tail: NONE }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.head == NONE
    }

    pub(super) fn push_back<T>(&mut self, buf: &mut FrameBuffer<T>, value: T) {
        let key = buf.insert(value);
        if self.tail != NONE {
            match &mut buf.entries[self.tail] {
                Entry::Occupied { next, .. } => *next = key,
                Entry::Vacant(_) => unreachable!("deque tail points at vacant slot"),
            }
            self.tail = key;
        } else {
            self.head = key;
            self.tail = key;
        }
    }

    pub(super) fn pop_front<T>(&mut self, buf: &mut FrameBuffer<T>) -> Option<T> {
        if self.head == NONE {
            return None;
        }
        let idx = self.head;
        let next = match &buf.entries[idx] {
            Entry::Occupied { next, .. } => *next,
            Entry::Vacant(_) => unreachable!("deque head points at vacant slot"),
        };
        if next == NONE {
            self.tail = NONE;
        }
        self.head = next;
        Some(buf.remove(idx))
    }

    pub(super) fn clear<T>(&mut self, buf: &mut FrameBuffer<T>) {
        while self.pop_front(buf).is_some() {}
    }
}
