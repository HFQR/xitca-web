use super::*;
use std::convert::TryInto;
use std::io::Cursor;

const TABLE_SIZE: usize = 4096;

#[test]
fn indexed_static() {
    let field = Indexed::Static(42);
    let mut buf = vec![];
    field.encode(&mut buf);
    let mut read = Cursor::new(&buf);
    assert_eq!(Indexed::decode(&mut read), Ok(field));
}

#[test]
fn indexed_dynamic() {
    let field = Indexed::Dynamic(42);
    let mut buf = vec![];
    field.encode(&mut buf);
    let mut read = Cursor::new(&buf);
    assert_eq!(Indexed::decode(&mut read), Ok(field));
}

#[test]
fn indexed_with_postbase() {
    let field = IndexedWithPostBase(42);
    let mut buf = vec![];
    field.encode(&mut buf);
    let mut read = Cursor::new(&buf);
    assert_eq!(IndexedWithPostBase::decode(&mut read), Ok(field));
}

#[test]
fn literal_with_name_ref() {
    let field = LiteralWithNameRef::new_static(42, "foo");
    let mut buf = vec![];
    field.encode(&mut buf).unwrap();
    let mut read = Cursor::new(&buf);
    assert_eq!(LiteralWithNameRef::decode(&mut read), Ok(field));
}

#[test]
fn literal_with_post_base_name_ref() {
    let field = LiteralWithPostBaseNameRef::new(42, "foo");
    let mut buf = vec![];
    field.encode(&mut buf).unwrap();
    let mut read = Cursor::new(&buf);
    assert_eq!(LiteralWithPostBaseNameRef::decode(&mut read), Ok(field));
}

#[test]
fn literal() {
    let field = Literal::new("foo", "bar");
    let mut buf = vec![];
    field.encode(&mut buf).unwrap();
    let mut read = Cursor::new(&buf);
    assert_eq!(Literal::decode(&mut read), Ok(field));
}

#[test]
fn header_prefix() {
    let prefix = HeaderPrefix::new(10, 5, 12, TABLE_SIZE);
    let mut buf = vec![];
    prefix.encode(&mut buf);
    let mut read = Cursor::new(&buf);
    let decoded = HeaderPrefix::decode(&mut read);
    assert_eq!(decoded, Ok(prefix));
    assert_eq!(decoded.unwrap().get(13, 3332).unwrap(), (10, 5));
}

#[test]
fn header_prefix_table_size_0() {
    HeaderPrefix::new(10, 5, 12, 0).get(1, 0).unwrap();
}

#[test]
fn base_index_too_small() {
    let mut buf = vec![];
    let encoded_largest_ref: u64 = ((2 % (2 * TABLE_SIZE / 32)) + 1).try_into().unwrap();
    prefix_int::encode(8, 0, encoded_largest_ref, &mut buf);
    prefix_int::encode(7, 1, 2, &mut buf); // base index negative = 0

    let mut read = Cursor::new(&buf);
    assert_eq!(
        HeaderPrefix::decode(&mut read).unwrap().get(2, TABLE_SIZE),
        Err(ParseError::InvalidBase(-1))
    );
}
