use core::cmp;

use std::io::Cursor;

use crate::bytes::{Buf, BufMut};

use super::{
    HeaderField,
    block::{HeaderPrefix, Indexed, IndexedWithPostBase, Literal, LiteralWithNameRef, LiteralWithPostBaseNameRef},
    dynamic::{
        DynamicInsertionResult, DynamicLookupResult, DynamicTable, DynamicTableEncoder, Error as DynamicTableError,
    },
    parse_error::ParseError,
    prefix_int::Error as IntError,
    prefix_string::Error as StringError,
    static_::StaticTable,
    stream::{
        DecoderInstruction, Duplicate, DynamicTableSizeUpdate, HeaderAck, InsertCountIncrement, InsertWithNameRef,
        InsertWithoutNameRef, StreamCancel,
    },
};

#[derive(Debug, PartialEq)]
pub enum EncoderError {
    Insertion(DynamicTableError),
    InvalidString(StringError),
    InvalidInteger(IntError),
    UnknownDecoderInstruction(u8),
}

impl std::error::Error for EncoderError {}

impl std::fmt::Display for EncoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncoderError::Insertion(e) => write!(f, "dynamic table insertion: {:?}", e),
            EncoderError::InvalidString(e) => write!(f, "could not parse string: {}", e),
            EncoderError::InvalidInteger(e) => write!(f, "could not parse integer: {}", e),
            EncoderError::UnknownDecoderInstruction(e) => {
                write!(f, "got unkown decoder instruction: {}", e)
            }
        }
    }
}

pub struct Encoder {
    table: DynamicTable,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    pub fn new() -> Self {
        Self {
            table: DynamicTable::new(),
        }
    }

    /// Configure the maximum dynamic table size we're willing to use when
    /// encoding outbound headers. This must not exceed the peer's advertised
    /// `QPACK_MAX_TABLE_CAPACITY`.
    pub fn set_max_table_size(&mut self, size: usize) -> Result<(), DynamicTableError> {
        self.table.set_max_size(size)
    }

    pub fn set_max_blocked_streams(&mut self, max: usize) -> Result<(), DynamicTableError> {
        self.table.set_max_blocked(max)
    }

    pub fn encode<W, T, H>(
        &mut self,
        stream_id: u64,
        block: &mut W,
        encoder_buf: &mut W,
        fields: T,
    ) -> Result<usize, EncoderError>
    where
        W: BufMut,
        T: IntoIterator<Item = H>,
        H: AsRef<HeaderField>,
    {
        let mut required_ref = 0;
        let mut block_buf = Vec::new();
        let mut encoder = self.table.encoder(stream_id);

        for field in fields {
            if let Some(reference) = Self::encode_field(&mut encoder, &mut block_buf, encoder_buf, field.as_ref())? {
                required_ref = cmp::max(required_ref, reference);
            }
        }

        HeaderPrefix::new(
            required_ref,
            encoder.base(),
            encoder.total_inserted(),
            encoder.max_size(),
        )
        .encode(block);
        block.put(block_buf.as_slice());

        encoder.commit(required_ref);

        Ok(required_ref)
    }

    pub fn on_decoder_recv<R: Buf>(&mut self, read: &mut R) -> Result<(), EncoderError> {
        while let Some(instruction) = Action::parse(read)? {
            match instruction {
                Action::Untrack(stream_id) => self.table.untrack_block(stream_id)?,
                Action::StreamCancel(stream_id) => {
                    // Untrack block twice, as this stream might have a trailer in addition to
                    // the header. Failures are ignored as blocks might have been acked before
                    // cancellation.
                    if self.table.untrack_block(stream_id).is_ok() {
                        let _ = self.table.untrack_block(stream_id);
                    }
                }
                Action::ReceivedRefIncrement(increment) => self.table.update_largest_received(increment),
            }
        }
        Ok(())
    }

    fn encode_field<W: BufMut>(
        table: &mut DynamicTableEncoder,
        block: &mut Vec<u8>,
        encoder: &mut W,
        field: &HeaderField,
    ) -> Result<Option<usize>, EncoderError> {
        if let Some(index) = StaticTable::find(field) {
            Indexed::Static(index).encode(block);
            return Ok(None);
        }

        if let DynamicLookupResult::Relative { index, absolute } = table.find(field) {
            Indexed::Dynamic(index).encode(block);
            return Ok(Some(absolute));
        }

        let reference = match table.insert(field)? {
            DynamicInsertionResult::Duplicated {
                relative,
                postbase,
                absolute,
            } => {
                Duplicate(relative).encode(encoder);
                IndexedWithPostBase(postbase).encode(block);
                Some(absolute)
            }
            DynamicInsertionResult::Inserted { postbase, absolute } => {
                InsertWithoutNameRef::new(field.name.clone(), field.value.clone()).encode(encoder)?;
                IndexedWithPostBase(postbase).encode(block);
                Some(absolute)
            }
            DynamicInsertionResult::InsertedWithStaticNameRef {
                postbase,
                index,
                absolute,
            } => {
                InsertWithNameRef::new_static(index, field.value.clone()).encode(encoder)?;
                IndexedWithPostBase(postbase).encode(block);
                Some(absolute)
            }
            DynamicInsertionResult::InsertedWithNameRef {
                postbase,
                relative,
                absolute,
            } => {
                InsertWithNameRef::new_dynamic(relative, field.value.clone()).encode(encoder)?;
                IndexedWithPostBase(postbase).encode(block);
                Some(absolute)
            }
            DynamicInsertionResult::NotInserted(lookup_result) => match lookup_result {
                DynamicLookupResult::Static(index) => {
                    LiteralWithNameRef::new_static(index, field.value.clone()).encode(block)?;
                    None
                }
                DynamicLookupResult::Relative { index, absolute } => {
                    LiteralWithNameRef::new_dynamic(index, field.value.clone()).encode(block)?;
                    Some(absolute)
                }
                DynamicLookupResult::PostBase { index, absolute } => {
                    LiteralWithPostBaseNameRef::new(index, field.value.clone()).encode(block)?;
                    Some(absolute)
                }
                DynamicLookupResult::NotFound => {
                    Literal::new(field.name.clone(), field.value.clone()).encode(block)?;
                    None
                }
            },
        };
        Ok(reference)
    }
}

pub fn encode_stateless<W, T, H>(block: &mut W, fields: T) -> Result<u64, EncoderError>
where
    W: BufMut,
    T: IntoIterator<Item = H>,
    H: AsRef<HeaderField>,
{
    let mut size = 0;

    HeaderPrefix::new(0, 0, 0, 0).encode(block);
    for field in fields {
        let field = field.as_ref();

        if let Some(index) = StaticTable::find(field) {
            Indexed::Static(index).encode(block);
        } else if let Some(index) = StaticTable::find_name(&field.name) {
            LiteralWithNameRef::new_static(index, field.value.clone()).encode(block)?;
        } else {
            Literal::new(field.name.clone(), field.value.clone()).encode(block)?;
        }

        size += field.mem_size() as u64;
    }
    Ok(size)
}

#[cfg(test)]
impl From<DynamicTable> for Encoder {
    fn from(table: DynamicTable) -> Encoder {
        Encoder { table }
    }
}

// Action to apply to the encoder table, given an instruction received from the decoder.
#[derive(Debug, PartialEq)]
enum Action {
    ReceivedRefIncrement(usize),
    Untrack(u64),
    StreamCancel(u64),
}

impl Action {
    fn parse<R: Buf>(read: &mut R) -> Result<Option<Action>, EncoderError> {
        if read.remaining() < 1 {
            return Ok(None);
        }

        let mut buf = Cursor::new(read.chunk());
        let first = buf.chunk()[0];
        let instruction = match DecoderInstruction::decode(first) {
            DecoderInstruction::Unknown => return Err(EncoderError::UnknownDecoderInstruction(first)),
            DecoderInstruction::InsertCountIncrement => {
                InsertCountIncrement::decode(&mut buf)?.map(|x| Action::ReceivedRefIncrement(x.0 as usize))
            }
            DecoderInstruction::HeaderAck => HeaderAck::decode(&mut buf)?.map(|x| Action::Untrack(x.0)),
            DecoderInstruction::StreamCancel => StreamCancel::decode(&mut buf)?.map(|x| Action::StreamCancel(x.0)),
        };

        if instruction.is_some() {
            let pos = buf.position();
            read.advance(pos as usize);
        }

        Ok(instruction)
    }
}

pub fn set_dynamic_table_size<W: BufMut>(
    table: &mut DynamicTable,
    encoder: &mut W,
    size: usize,
) -> Result<(), EncoderError> {
    table.set_max_size(size)?;
    DynamicTableSizeUpdate(size).encode(encoder);
    Ok(())
}

impl From<DynamicTableError> for EncoderError {
    fn from(e: DynamicTableError) -> Self {
        EncoderError::Insertion(e)
    }
}

impl From<StringError> for EncoderError {
    fn from(e: StringError) -> Self {
        EncoderError::InvalidString(e)
    }
}

impl From<ParseError> for EncoderError {
    fn from(e: ParseError) -> Self {
        match e {
            ParseError::Integer(x) => EncoderError::InvalidInteger(x),
            ParseError::String(x) => EncoderError::InvalidString(x),
            ParseError::InvalidPrefix(x) => EncoderError::UnknownDecoderInstruction(x),
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests;
