use core::{convert::TryInto, fmt, num::TryFromIntError};

use std::io::Cursor;

use crate::bytes::{Buf, BufMut};

use super::{
    dynamic::{DynamicTable, DynamicTableDecoder, Error as DynamicTableError},
    field::HeaderField,
    static_::{Error as StaticError, StaticTable},
    vas,
};

use super::{
    block::{
        HeaderBlockField, HeaderPrefix, Indexed, IndexedWithPostBase, Literal, LiteralWithNameRef,
        LiteralWithPostBaseNameRef,
    },
    parse_error::ParseError,
    stream::{
        Duplicate, DynamicTableSizeUpdate, EncoderInstruction, HeaderAck, InsertCountIncrement, InsertWithNameRef,
        InsertWithoutNameRef, StreamCancel,
    },
};

use super::{prefix_int, prefix_string};

#[derive(Debug, PartialEq)]
pub enum DecoderError {
    InvalidInteger(prefix_int::Error),
    InvalidString(prefix_string::Error),
    InvalidIndex(vas::Error),
    DynamicTable(DynamicTableError),
    InvalidStaticIndex(usize),
    UnknownPrefix(u8),
    MissingRefs(usize),
    BadBaseIndex(isize),
    UnexpectedEnd,
    HeaderTooLong(u64),
    BufSize(TryFromIntError),
}

impl std::error::Error for DecoderError {}

impl std::fmt::Display for DecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecoderError::InvalidInteger(e) => write!(f, "invalid integer: {}", e),
            DecoderError::InvalidString(e) => write!(f, "invalid string: {:?}", e),
            DecoderError::InvalidIndex(e) => write!(f, "invalid dynamic index: {:?}", e),
            DecoderError::DynamicTable(e) => write!(f, "dynamic table error: {:?}", e),
            DecoderError::InvalidStaticIndex(i) => write!(f, "unknown static index: {}", i),
            DecoderError::UnknownPrefix(p) => write!(f, "unknown instruction code: 0x{}", p),
            DecoderError::MissingRefs(n) => write!(f, "missing {} refs to decode bloc", n),
            DecoderError::BadBaseIndex(i) => write!(f, "out of bounds base index: {}", i),
            DecoderError::UnexpectedEnd => write!(f, "unexpected end"),
            DecoderError::HeaderTooLong(_) => write!(f, "header too long"),
            DecoderError::BufSize(_) => write!(f, "number in buffer wrong size"),
        }
    }
}

pub fn ack_header<W: BufMut>(stream_id: u64, decoder: &mut W) {
    HeaderAck(stream_id).encode(decoder);
}

pub fn stream_canceled<W: BufMut>(stream_id: u64, decoder: &mut W) {
    StreamCancel(stream_id).encode(decoder);
}

#[derive(PartialEq, Debug)]
pub struct Decoded {
    /// The decoded fields
    pub fields: Vec<HeaderField>,
    /// Whether one or more encoded fields were referencing the dynamic table
    pub dyn_ref: bool,
    /// Decoded size, calculated as stated in "4.1.1.3. Header Size Constraints"
    pub mem_size: u64,
}

pub struct Decoder {
    table: DynamicTable,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            table: DynamicTable::new(),
        }
    }

    /// Configure the maximum dynamic table size we're willing to accept on
    /// inbound encoder-stream instructions. This is the same value we
    /// advertise to the peer via `QPACK_MAX_TABLE_CAPACITY`.
    pub fn set_max_table_size(&mut self, size: usize) -> Result<(), DynamicTableError> {
        self.table.set_max_size(size)
    }

    /// Configure the maximum number of streams we're willing to keep blocked
    /// waiting for dynamic-table inserts. Mirrors `QPACK_MAX_BLOCKED_STREAMS`.
    pub fn set_max_blocked_streams(&mut self, max: usize) -> Result<(), DynamicTableError> {
        self.table.set_max_blocked(max)
    }

    // Decode field lines received on Request of Push stream.
    // https://www.rfc-editor.org/rfc/rfc9204.html#name-field-line-representations
    pub fn decode_header<T: Buf>(&self, buf: &mut T) -> Result<Decoded, DecoderError> {
        let (required_ref, base) =
            HeaderPrefix::decode(buf)?.get(self.table.total_inserted(), self.table.max_mem_size())?;

        if required_ref > self.table.total_inserted() {
            return Err(DecoderError::MissingRefs(required_ref));
        }

        let decoder_table = self.table.decoder(base);

        let mut mem_size = 0;
        let mut fields = Vec::new();
        while buf.has_remaining() {
            let field = Self::parse_header_field(&decoder_table, buf)?;
            mem_size += field.mem_size() as u64;
            fields.push(field);
        }

        Ok(Decoded {
            fields,
            mem_size,
            dyn_ref: required_ref > 0,
        })
    }

    // The receiving side of encoder stream
    pub fn on_encoder_recv<R: Buf, W: BufMut>(&mut self, read: &mut R, write: &mut W) -> Result<usize, DecoderError> {
        let inserted_on_start = self.table.total_inserted();

        while let Some(instruction) = self.parse_instruction(read)? {
            match instruction {
                Instruction::Insert(field) => self.table.put(field)?,
                Instruction::TableSizeUpdate(size) => {
                    self.table.set_max_size(size)?;
                }
            }
        }

        if self.table.total_inserted() != inserted_on_start {
            InsertCountIncrement((self.table.total_inserted() - inserted_on_start).try_into()?).encode(write);
        }

        Ok(self.table.total_inserted())
    }

    fn parse_instruction<R: Buf>(&self, read: &mut R) -> Result<Option<Instruction>, DecoderError> {
        if read.remaining() < 1 {
            return Ok(None);
        }

        let mut buf = Cursor::new(read.chunk());
        let first = buf.chunk()[0];
        let instruction = match EncoderInstruction::decode(first) {
            EncoderInstruction::Unknown => return Err(DecoderError::UnknownPrefix(first)),
            EncoderInstruction::DynamicTableSizeUpdate => {
                DynamicTableSizeUpdate::decode(&mut buf)?.map(|x| Instruction::TableSizeUpdate(x.0))
            }
            EncoderInstruction::InsertWithoutNameRef => {
                InsertWithoutNameRef::decode(&mut buf)?.map(|x| Instruction::Insert(HeaderField::new(x.name, x.value)))
            }
            EncoderInstruction::Duplicate => match Duplicate::decode(&mut buf)? {
                Some(Duplicate(index)) => Some(Instruction::Insert(self.table.get_relative(index)?.clone())),
                None => None,
            },
            EncoderInstruction::InsertWithNameRef => match InsertWithNameRef::decode(&mut buf)? {
                Some(InsertWithNameRef::Static { index, value }) => {
                    Some(Instruction::Insert(StaticTable::get(index)?.with_value(value)))
                }
                Some(InsertWithNameRef::Dynamic { index, value }) => {
                    Some(Instruction::Insert(self.table.get_relative(index)?.with_value(value)))
                }
                None => None,
            },
        };

        if instruction.is_some() {
            let pos = buf.position();
            read.advance(pos as usize);
        }

        Ok(instruction)
    }

    fn parse_header_field<R: Buf>(table: &DynamicTableDecoder, buf: &mut R) -> Result<HeaderField, DecoderError> {
        let first = buf.chunk()[0];
        let field = match HeaderBlockField::decode(first) {
            HeaderBlockField::Indexed => match Indexed::decode(buf)? {
                Indexed::Static(index) => StaticTable::get(index)?.clone(),
                Indexed::Dynamic(index) => table.get_relative(index)?.clone(),
            },
            HeaderBlockField::IndexedWithPostBase => {
                let index = IndexedWithPostBase::decode(buf)?.0;
                table.get_postbase(index)?.clone()
            }
            HeaderBlockField::LiteralWithNameRef => match LiteralWithNameRef::decode(buf)? {
                LiteralWithNameRef::Static { index, value } => StaticTable::get(index)?.with_value(value),
                LiteralWithNameRef::Dynamic { index, value } => table.get_relative(index)?.with_value(value),
            },
            HeaderBlockField::LiteralWithPostBaseNameRef => {
                let literal = LiteralWithPostBaseNameRef::decode(buf)?;
                table.get_postbase(literal.index)?.with_value(literal.value)
            }
            HeaderBlockField::Literal => {
                let literal = Literal::decode(buf)?;
                HeaderField::new(literal.name, literal.value)
            }
            _ => return Err(DecoderError::UnknownPrefix(first)),
        };
        Ok(field)
    }
}

// Decode field lines received on Request or Push stream.
// https://www.rfc-editor.org/rfc/rfc9204.html#name-field-line-representations
pub fn decode_stateless<T: Buf>(buf: &mut T, max_size: u64) -> Result<Decoded, DecoderError> {
    let (required_ref, _base) = HeaderPrefix::decode(buf)?.get(0, 0)?;

    if required_ref > 0 {
        return Err(DecoderError::MissingRefs(required_ref));
    }

    let mut mem_size = 0;
    let mut fields = Vec::new();
    while buf.has_remaining() {
        let field = match HeaderBlockField::decode(buf.chunk()[0]) {
            HeaderBlockField::IndexedWithPostBase => return Err(DecoderError::MissingRefs(0)),
            HeaderBlockField::LiteralWithPostBaseNameRef => return Err(DecoderError::MissingRefs(0)),
            HeaderBlockField::Indexed => match Indexed::decode(buf)? {
                Indexed::Static(index) => StaticTable::get(index)?.clone(),
                Indexed::Dynamic(_) => return Err(DecoderError::MissingRefs(0)),
            },
            HeaderBlockField::LiteralWithNameRef => match LiteralWithNameRef::decode(buf)? {
                LiteralWithNameRef::Dynamic { .. } => return Err(DecoderError::MissingRefs(0)),
                LiteralWithNameRef::Static { index, value } => StaticTable::get(index)?.with_value(value),
            },
            HeaderBlockField::Literal => {
                let literal = Literal::decode(buf)?;
                HeaderField::new(literal.name, literal.value)
            }
            _ => return Err(DecoderError::UnknownPrefix(buf.chunk()[0])),
        };
        mem_size += field.mem_size() as u64;
        // Cancel decoding if the header is considered too big
        if mem_size > max_size {
            return Err(DecoderError::HeaderTooLong(mem_size));
        }
        fields.push(field);
    }

    Ok(Decoded {
        fields,
        mem_size,
        dyn_ref: false,
    })
}

#[cfg(test)]
impl From<DynamicTable> for Decoder {
    fn from(table: DynamicTable) -> Self {
        Self { table }
    }
}

#[derive(PartialEq)]
enum Instruction {
    Insert(HeaderField),
    TableSizeUpdate(usize),
}

impl fmt::Debug for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Instruction::Insert(h) => write!(f, "Instruction::Insert {{ {} }}", h),
            Instruction::TableSizeUpdate(n) => {
                write!(f, "Instruction::TableSizeUpdate {{ {} }}", n)
            }
        }
    }
}

impl From<prefix_int::Error> for DecoderError {
    fn from(e: prefix_int::Error) -> Self {
        match e {
            prefix_int::Error::UnexpectedEnd => DecoderError::UnexpectedEnd,
            e => DecoderError::InvalidInteger(e),
        }
    }
}

impl From<prefix_string::Error> for DecoderError {
    fn from(e: prefix_string::Error) -> Self {
        match e {
            prefix_string::Error::UnexpectedEnd => DecoderError::UnexpectedEnd,
            e => DecoderError::InvalidString(e),
        }
    }
}

impl From<vas::Error> for DecoderError {
    fn from(e: vas::Error) -> Self {
        DecoderError::InvalidIndex(e)
    }
}

impl From<StaticError> for DecoderError {
    fn from(e: StaticError) -> Self {
        match e {
            StaticError::Unknown(i) => DecoderError::InvalidStaticIndex(i),
        }
    }
}

impl From<DynamicTableError> for DecoderError {
    fn from(e: DynamicTableError) -> Self {
        DecoderError::DynamicTable(e)
    }
}

impl From<ParseError> for DecoderError {
    fn from(e: ParseError) -> Self {
        match e {
            ParseError::Integer(x) => DecoderError::InvalidInteger(x),
            ParseError::String(x) => DecoderError::InvalidString(x),
            ParseError::InvalidPrefix(p) => DecoderError::UnknownPrefix(p),
            ParseError::InvalidBase(b) => DecoderError::BadBaseIndex(b),
        }
    }
}

impl From<TryFromIntError> for DecoderError {
    fn from(error: TryFromIntError) -> Self {
        DecoderError::BufSize(error)
    }
}

#[cfg(test)]
mod tests;
