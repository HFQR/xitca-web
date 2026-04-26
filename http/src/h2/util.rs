/// Shared slab-backed storage for frames across all streams, avoiding
/// per-stream heap allocation. Mirrors `h2::proto::streams::buffer::Buffer`.
pub(super) struct FrameBuffer<T> {
    entries: Vec<Entry<T>>,
    /// Head of the free list (indices of vacant slots), or `u32::MAX` if none.
    next_free: u32,
    len: usize,
}

/// A slab entry. `Vacant` carries the next free-list index; `Occupied` carries
/// the value together with the next index in its owning [`Deque`]. Unifying
/// the free-list link with the deque link means vacant slots pay nothing for
/// the absent value and occupied slots pay nothing for a dead free pointer.
enum Entry<T> {
    Vacant(u32),
    Occupied { value: T, next: u32 },
}

/// A per-stream linked-list deque backed by a shared [`FrameBuffer`].
/// Stores only head/tail indices — zero per-stream allocation.
pub(super) struct Deque {
    head: u32,
    tail: u32,
}

const NONE: u32 = u32::MAX;

impl<T> FrameBuffer<T> {
    pub(super) const fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_free: NONE,
            len: 0,
        }
    }

    /// Insert a value into the slab, returning its index.
    fn insert(&mut self, value: T) -> u32 {
        self.len += 1;
        if self.next_free != NONE {
            let idx = self.next_free;
            match &self.entries[idx as usize] {
                Entry::Vacant(next) => self.next_free = *next,
                Entry::Occupied { .. } => unreachable!("free-list slot is occupied"),
            }
            self.entries[idx as usize] = Entry::Occupied { value, next: NONE };
            idx
        } else {
            debug_assert!(self.entries.len() < NONE as usize, "FrameBuffer index space exhausted");
            let idx = self.entries.len() as u32;
            self.entries.push(Entry::Occupied { value, next: NONE });
            idx
        }
    }

    /// Remove a value by index, returning it and placing the slot on the free list.
    fn remove(&mut self, idx: u32) -> T {
        self.len -= 1;
        let entry = core::mem::replace(&mut self.entries[idx as usize], Entry::Vacant(self.next_free));
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
            match &mut buf.entries[self.tail as usize] {
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
        let next = match &buf.entries[idx as usize] {
            Entry::Occupied { next, .. } => *next,
            Entry::Vacant(_) => unreachable!("deque head points at vacant slot"),
        };
        if next == NONE {
            self.tail = NONE;
        }
        self.head = next;
        Some(buf.remove(idx))
    }
}
