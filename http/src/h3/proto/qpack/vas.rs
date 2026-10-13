/*
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-absolute-indexing
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-relative-indexing
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-post-base-indexing
 */

/*
 *  # Virtually infinite address space mapper.
 *
 *  It can be described as an infinitive growable list, with a visibility
 *  window that can only move in the direction of insertion.
 *
 *  Origin          Visible window
 *  /\         /===========^===========\
 *  ++++-------+ - + - + - + - + - + - +
 *  ||||       |   |   |   |   |   |   |  ==> Grow direction
 *  ++++-------+ - + - + - + - + - + - +
 *  \================v==================/
 *           Full Virtual Space
 *
 *
 *  QPACK indexing is 1-based for absolute index, and 0-based for relative's.
 *  Container (ex: list) indexing is 0-based.
 *
 *
 *  # Basics
 *
 *  inserted: number of insertion
 *  dropped : number of drop
 *  delta   : count of available elements
 *
 *  abs: absolute index
 *  rel: relative index
 *  pos: real index in memory container
 *  pst: post-base relative index (only with base index)
 *
 *    first      oldest              latest
 *    element    insertion           insertion
 *    (not       available           available
 *    available) |                   |
 *    |          |                   |
 *    v          v                   v
 *  + - +------+ - + - + - + - + - + - +  inserted: 21
 *  | a |      | p | q | r | s | t | u |  dropped: 15
 *  + - +------+ - + - + - + - + - + - +  delta: 21 - 15: 6
 *    ^          ^                   ^
 *    |          |                   |
 * abs:-      abs:16              abs:21
 * rel:-      rel:5               rel:0
 * pos:-      pos:0               pos:6
 *
 *
 * # Base index
 * A base index can arbitrary shift the relative index.
 * The base index itself is an absolute index.
 *
 *                       base index: 17
 *                       |
 *                       v
 *  + - +------+ - + - + - + - + - + - +  inserted: 21
 *  | a |      | p | q | r | s | t | u |  dropped: 15
 *  + - +------+ - + - + - + - + - + - +  delta: 21 - 15: 6
 *    ^          ^       ^           ^
 *    |          |       |           |
 * abs:-      abs:16  abs:18      abs:21
 * rel:-      rel:2   rel:0       rel:-
 * pst:-      pst:-   pst:-       pst:2
 * pos:-      pos:0   pos:2       pos:6
 */

pub type RelativeIndex = usize;
pub type AbsoluteIndex = usize;

#[derive(Debug, PartialEq)]
pub enum Error {
    RelativeIndex(usize),
    PostbaseIndex(usize),
    Index(usize),
}

#[derive(Debug, Default)]
pub struct VirtualAddressSpace {
    inserted: usize,
    dropped: usize,
    delta: usize,
}

impl VirtualAddressSpace {
    pub fn add(&mut self) -> AbsoluteIndex {
        self.inserted += 1;
        self.delta += 1;
        self.inserted
    }

    pub fn drop(&mut self) {
        self.dropped += 1;
        self.delta -= 1;
    }

    pub fn relative(&self, index: RelativeIndex) -> Result<usize, Error> {
        if self.inserted < index || self.delta == 0 || self.inserted - index <= self.dropped {
            Err(Error::RelativeIndex(index))
        } else {
            Ok(self.inserted - self.dropped - index - 1)
        }
    }

    pub fn evicted(&self, index: AbsoluteIndex) -> bool {
        index != 0 && index <= self.dropped
    }

    pub fn relative_base(&self, base: usize, index: RelativeIndex) -> Result<usize, Error> {
        if self.delta == 0 || index > base || base - index <= self.dropped {
            Err(Error::RelativeIndex(index))
        } else {
            Ok(base - self.dropped - index - 1)
        }
    }

    pub fn post_base(&self, base: usize, index: RelativeIndex) -> Result<usize, Error> {
        if self.delta == 0 || base + index >= self.inserted || base + index < self.dropped {
            Err(Error::PostbaseIndex(index))
        } else {
            Ok(base + index - self.dropped)
        }
    }

    pub fn index(&self, index: usize) -> Result<usize, Error> {
        if index >= self.delta {
            Err(Error::Index(index))
        } else {
            Ok(index + self.dropped + 1)
        }
    }

    pub fn largest_ref(&self) -> usize {
        self.inserted - self.dropped
    }

    pub fn total_inserted(&self) -> usize {
        self.inserted
    }
}

#[cfg(test)]
mod tests;
