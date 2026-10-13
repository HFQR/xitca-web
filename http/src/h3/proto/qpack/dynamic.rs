use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, VecDeque, btree_map::Entry as BTEntry, hash_map::Entry},
};

use super::{
    field::HeaderField,
    static_::StaticTable,
    vas::{self, VirtualAddressSpace},
};

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#maximum-dynamic-table-capacity
 */
const SETTINGS_MAX_TABLE_CAPACITY_MAX: usize = 1_073_741_823; // 2^30 -1
const SETTINGS_MAX_BLOCKED_STREAMS_MAX: usize = 65_535; // 2^16 - 1

#[derive(Debug, PartialEq)]
pub enum Error {
    BadRelativeIndex(usize),
    BadPostbaseIndex(usize),
    BadIndex(usize),
    MaxTableSizeReached,
    MaximumTableSizeTooLarge,
    MaxBlockedStreamsTooLarge,
    UnknownStreamId(u64),
    NoTrackingData,
    InvalidTrackingCount,
}

pub struct DynamicTableDecoder<'a> {
    table: &'a DynamicTable,
    base: usize,
}

impl<'a> DynamicTableDecoder<'a> {
    pub(super) fn get_relative(&self, index: usize) -> Result<&HeaderField, Error> {
        let real_index = self.table.vas.relative_base(self.base, index)?;
        self.table.fields.get(real_index).ok_or(Error::BadIndex(real_index))
    }

    pub(super) fn get_postbase(&self, index: usize) -> Result<&HeaderField, Error> {
        let real_index = self.table.vas.post_base(self.base, index)?;
        self.table.fields.get(real_index).ok_or(Error::BadIndex(real_index))
    }
}

pub struct DynamicTableEncoder<'a> {
    table: &'a mut DynamicTable,
    base: usize,
    commited: bool,
    stream_id: u64,
    block_refs: HashMap<usize, usize>,
}

impl<'a> Drop for DynamicTableEncoder<'a> {
    fn drop(&mut self) {
        if !self.commited {
            // TODO maybe possible to replace and not clone here?
            // HOW Err should be handled?
            self.table
                .track_cancel(self.block_refs.iter().map(|(x, y)| (*x, *y)))
                .ok();
        }
    }
}

impl<'a> DynamicTableEncoder<'a> {
    pub(super) fn max_size(&self) -> usize {
        self.table.max_size
    }

    pub(super) fn base(&self) -> usize {
        self.base
    }

    pub(super) fn total_inserted(&self) -> usize {
        self.table.total_inserted()
    }

    pub(super) fn commit(&mut self, largest_ref: usize) {
        self.table.track_block(self.stream_id, self.block_refs.clone());
        self.table.register_blocked(largest_ref);
        self.commited = true;
    }

    pub(super) fn find(&mut self, field: &HeaderField) -> DynamicLookupResult {
        self.lookup_result(self.table.field_map.get(field).cloned())
    }

    fn lookup_result(&mut self, absolute: Option<usize>) -> DynamicLookupResult {
        match absolute {
            Some(absolute) if absolute <= self.base => {
                self.track_ref(absolute);
                DynamicLookupResult::Relative {
                    index: self.base - absolute,
                    absolute,
                }
            }
            Some(absolute) if absolute > self.base => {
                self.track_ref(absolute);
                DynamicLookupResult::PostBase {
                    index: absolute - self.base - 1,
                    absolute,
                }
            }
            _ => DynamicLookupResult::NotFound,
        }
    }

    pub(super) fn insert(&mut self, field: &HeaderField) -> Result<DynamicInsertionResult, Error> {
        if self.table.blocked_count >= self.table.blocked_max {
            return Ok(DynamicInsertionResult::NotInserted(self.find_name(&field.name)));
        }

        let index = match self.table.insert(field.clone()) {
            Ok(Some(index)) => index,
            Err(Error::MaxTableSizeReached) | Ok(None) => {
                return Ok(DynamicInsertionResult::NotInserted(self.find_name(&field.name)));
            }
            Err(e) => return Err(e),
        };
        self.track_ref(index);

        let field_index = match self.table.field_map.entry(field.clone()) {
            Entry::Occupied(mut e) => {
                let ref_index = e.insert(index);
                self.table.name_map.entry(field.name.clone()).and_modify(|i| *i = index);

                Some((
                    ref_index,
                    DynamicInsertionResult::Duplicated {
                        relative: index - ref_index - 1,
                        postbase: index - self.base - 1,
                        absolute: index,
                    },
                ))
            }
            Entry::Vacant(e) => {
                e.insert(index);
                None
            }
        };

        if let Some((ref_index, result)) = field_index {
            self.track_ref(ref_index);
            return Ok(result);
        }

        if let Some(static_idx) = StaticTable::find_name(&field.name) {
            return Ok(DynamicInsertionResult::InsertedWithStaticNameRef {
                postbase: index - self.base - 1,
                index: static_idx,
                absolute: index,
            });
        }

        let result = match self.table.name_map.entry(field.name.clone()) {
            Entry::Occupied(mut e) => {
                let ref_index = e.insert(index);
                self.track_ref(ref_index);

                DynamicInsertionResult::InsertedWithNameRef {
                    postbase: index - self.base - 1,
                    relative: index - ref_index - 1,
                    absolute: index,
                }
            }
            Entry::Vacant(e) => {
                e.insert(index);
                DynamicInsertionResult::Inserted {
                    postbase: index - self.base - 1,
                    absolute: index,
                }
            }
        };
        Ok(result)
    }

    fn find_name(&mut self, name: &[u8]) -> DynamicLookupResult {
        if let Some(index) = StaticTable::find_name(name) {
            return DynamicLookupResult::Static(index);
        }

        self.lookup_result(self.table.name_map.get(name).cloned())
    }

    fn track_ref(&mut self, reference: usize) {
        self.block_refs.entry(reference).and_modify(|c| *c += 1).or_insert(1);
        self.table.track_ref(reference);
    }
}

#[derive(Debug, PartialEq)]
pub enum DynamicLookupResult {
    Static(usize),
    Relative { index: usize, absolute: usize },
    PostBase { index: usize, absolute: usize },
    NotFound,
}

#[derive(Debug, PartialEq)]
pub enum DynamicInsertionResult {
    Inserted {
        postbase: usize,
        absolute: usize,
    },
    Duplicated {
        relative: usize,
        postbase: usize,
        absolute: usize,
    },
    InsertedWithNameRef {
        postbase: usize,
        relative: usize,
        absolute: usize,
    },
    InsertedWithStaticNameRef {
        postbase: usize,
        index: usize,
        absolute: usize,
    },
    NotInserted(DynamicLookupResult),
}

#[derive(Default)]
pub struct DynamicTable {
    fields: VecDeque<HeaderField>,
    curr_size: usize,
    max_size: usize,
    vas: VirtualAddressSpace,
    field_map: HashMap<HeaderField, usize>,
    name_map: HashMap<Cow<'static, [u8]>, usize>,
    track_map: BTreeMap<usize, usize>,
    track_blocks: HashMap<u64, VecDeque<HashMap<usize, usize>>>,
    largest_known_received: usize,
    blocked_max: usize,
    blocked_count: usize,
    blocked_streams: BTreeMap<usize, usize>, // <required_ref, blocked_count>
}

impl DynamicTable {
    pub fn new() -> DynamicTable {
        DynamicTable::default()
    }

    pub fn decoder(&self, base: usize) -> DynamicTableDecoder<'_> {
        DynamicTableDecoder { table: self, base }
    }

    pub fn encoder(&mut self, stream_id: u64) -> DynamicTableEncoder<'_> {
        for (idx, field) in self.fields.iter().enumerate() {
            self.name_map.insert(field.name.clone(), self.vas.index(idx).unwrap());
            self.field_map.insert(field.clone(), self.vas.index(idx).unwrap());
        }

        DynamicTableEncoder {
            base: self.vas.largest_ref(),
            table: self,
            block_refs: HashMap::new(),
            commited: false,
            stream_id,
        }
    }

    pub fn set_max_blocked(&mut self, max: usize) -> Result<(), Error> {
        // TODO handle existing data
        if max >= SETTINGS_MAX_BLOCKED_STREAMS_MAX {
            return Err(Error::MaxBlockedStreamsTooLarge);
        }
        self.blocked_max = max;
        Ok(())
    }

    pub fn set_max_size(&mut self, size: usize) -> Result<(), Error> {
        if size > SETTINGS_MAX_TABLE_CAPACITY_MAX {
            return Err(Error::MaximumTableSizeTooLarge);
        }

        if size >= self.max_size {
            self.max_size = size;
            return Ok(());
        }

        let required = self.max_size - size;

        if let Some(to_evict) = self.can_free(required)? {
            self.evict(to_evict)?;
        }

        self.max_size = size;
        Ok(())
    }

    pub(super) fn put(&mut self, field: HeaderField) -> Result<(), Error> {
        let index = match self.insert(field.clone())? {
            Some(index) => index,
            None => return Ok(()),
        };

        self.field_map
            .entry(field.clone())
            .and_modify(|e| *e = index)
            .or_insert(index);

        if StaticTable::find_name(&field.name).is_some() {
            return Ok(());
        }

        self.name_map
            .entry(field.name.clone())
            .and_modify(|e| *e = index)
            .or_insert(index);
        Ok(())
    }

    pub(super) fn get_relative(&self, index: usize) -> Result<&HeaderField, Error> {
        let real_index = self.vas.relative(index)?;
        self.fields.get(real_index).ok_or(Error::BadIndex(real_index))
    }

    pub(super) fn total_inserted(&self) -> usize {
        self.vas.total_inserted()
    }

    pub(super) fn untrack_block(&mut self, stream_id: u64) -> Result<(), Error> {
        let mut entry = self.track_blocks.entry(stream_id);
        let block = match entry {
            Entry::Occupied(ref mut blocks) if blocks.get().len() > 1 => blocks.get_mut().pop_front(),
            Entry::Occupied(blocks) => blocks.remove().pop_front(),
            Entry::Vacant { .. } => return Err(Error::UnknownStreamId(stream_id)),
        };

        if let Some(b) = block {
            self.track_cancel(b.iter().map(|(x, y)| (*x, *y)))?;
        }
        Ok(())
    }

    fn insert(&mut self, field: HeaderField) -> Result<Option<usize>, Error> {
        if self.max_size == 0 {
            return Ok(None);
        }

        match self.can_free(field.mem_size())? {
            None => return Ok(None),
            Some(to_evict) => {
                self.evict(to_evict)?;
            }
        }

        self.curr_size += field.mem_size();
        self.fields.push_back(field);
        let absolute = self.vas.add();

        Ok(Some(absolute))
    }

    fn evict(&mut self, to_evict: usize) -> Result<(), Error> {
        for _ in 0..to_evict {
            let field = self.fields.pop_front().ok_or(Error::MaxTableSizeReached)?; //TODO better type
            self.curr_size -= field.mem_size();

            self.vas.drop();

            if let Entry::Occupied(e) = self.name_map.entry(field.name.clone())
                && self.vas.evicted(*e.get())
            {
                e.remove();
            }

            if let Entry::Occupied(e) = self.field_map.entry(field)
                && self.vas.evicted(*e.get())
            {
                e.remove();
            }
        }
        Ok(())
    }

    fn can_free(&mut self, required: usize) -> Result<Option<usize>, Error> {
        if required > self.max_size {
            return Err(Error::MaxTableSizeReached);
        }

        if self.max_size - self.curr_size >= required {
            return Ok(Some(0));
        }
        let lower_bound = self.max_size - required;

        let mut hypothetic_mem_size = self.curr_size;
        let mut evictable = 0;

        for (idx, to_evict) in self.fields.iter().enumerate() {
            if hypothetic_mem_size <= lower_bound {
                break;
            }

            if self.is_tracked(self.vas.index(idx).unwrap()) {
                // TODO handle out of bounds error
                break;
            }

            evictable += 1;
            hypothetic_mem_size -= to_evict.mem_size();
        }

        if required <= self.max_size - hypothetic_mem_size {
            Ok(Some(evictable))
        } else {
            Ok(None)
        }
    }

    fn track_ref(&mut self, reference: usize) {
        self.track_map.entry(reference).and_modify(|c| *c += 1).or_insert(1);
    }

    fn is_tracked(&self, reference: usize) -> bool {
        matches!(self.track_map.get(&reference), Some(count) if *count > 0)
    }

    fn track_block(&mut self, stream_id: u64, refs: HashMap<usize, usize>) {
        match self.track_blocks.entry(stream_id) {
            Entry::Occupied(mut e) => {
                e.get_mut().push_back(refs);
            }
            Entry::Vacant(e) => {
                let mut blocks = VecDeque::with_capacity(2);
                blocks.push_back(refs);
                e.insert(blocks);
            }
        }
    }

    fn track_cancel<T>(&mut self, refs: T) -> Result<(), Error>
    where
        T: IntoIterator<Item = (usize, usize)>,
    {
        for (reference, count) in refs {
            match self.track_map.entry(reference) {
                BTEntry::Occupied(mut e) => {
                    use std::cmp::Ordering;
                    match e.get().cmp(&count) {
                        Ordering::Less => {
                            return Err(Error::InvalidTrackingCount);
                        }
                        Ordering::Equal => {
                            e.remove(); // TODO just pu 0 ?
                        }
                        _ => *e.get_mut() -= count,
                    }
                }
                BTEntry::Vacant(_) => return Err(Error::InvalidTrackingCount),
            }
        }
        Ok(())
    }

    fn register_blocked(&mut self, largest: usize) {
        if largest <= self.largest_known_received {
            return;
        }

        self.blocked_count += 1;

        match self.blocked_streams.entry(largest) {
            BTEntry::Occupied(mut e) => {
                let entry = e.get_mut();
                *entry += 1;
            }
            BTEntry::Vacant(e) => {
                e.insert(1);
            }
        }
    }

    pub fn update_largest_received(&mut self, increment: usize) {
        self.largest_known_received += increment;

        if self.blocked_count == 0 {
            return;
        }

        let blocked = self.blocked_streams.split_off(&(self.largest_known_received + 1));
        let acked = std::mem::replace(&mut self.blocked_streams, blocked);

        if !acked.is_empty() {
            let total_acked = acked.iter().fold(0usize, |t, (_, v)| t + v);
            self.blocked_count -= total_acked;
        }
    }

    pub(super) fn max_mem_size(&self) -> usize {
        self.max_size
    }
}

impl From<vas::Error> for Error {
    fn from(e: vas::Error) -> Self {
        match e {
            vas::Error::RelativeIndex(e) => Error::BadRelativeIndex(e),
            vas::Error::PostbaseIndex(e) => Error::BadPostbaseIndex(e),
            vas::Error::Index(e) => Error::BadIndex(e),
        }
    }
}

#[cfg(test)]
mod tests;
