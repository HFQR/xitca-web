use super::*;
use std::io::Cursor;

#[test]
fn insert_with_name_ref() {
    let instruction = InsertWithNameRef::new_static(0, "value");
    let mut buf = vec![];
    instruction.encode(&mut buf).unwrap();
    let mut read = Cursor::new(&buf);
    assert_eq!(InsertWithNameRef::decode(&mut read), Ok(Some(instruction)));
}

#[test]
fn insert_without_name_ref() {
    let instruction = InsertWithoutNameRef::new("name", "value");
    let mut buf = vec![];
    instruction.encode(&mut buf).unwrap();
    let mut read = Cursor::new(&buf);
    assert_eq!(InsertWithoutNameRef::decode(&mut read), Ok(Some(instruction)));
}

#[test]
fn insert_duplicate() {
    let instruction = Duplicate(42);
    let mut buf = vec![];
    instruction.encode(&mut buf);
    let mut read = Cursor::new(&buf);
    assert_eq!(Duplicate::decode(&mut read), Ok(Some(instruction)));
}

#[test]
fn dynamic_table_size_update() {
    let instruction = DynamicTableSizeUpdate(42);
    let mut buf = vec![];
    instruction.encode(&mut buf);
    let mut read = Cursor::new(&buf);
    assert_eq!(DynamicTableSizeUpdate::decode(&mut read), Ok(Some(instruction)));
}

#[test]
fn insert_count_increment() {
    let instruction = InsertCountIncrement(42);
    let mut buf = vec![];
    instruction.encode(&mut buf);
    let mut read = Cursor::new(&buf);
    assert_eq!(InsertCountIncrement::decode(&mut read), Ok(Some(instruction)));
}

#[test]
fn header_ack() {
    let instruction = HeaderAck(42);
    let mut buf = vec![];
    instruction.encode(&mut buf);
    let mut read = Cursor::new(&buf);
    assert_eq!(HeaderAck::decode(&mut read), Ok(Some(instruction)));
}

#[test]
fn stream_cancel() {
    let instruction = StreamCancel(42);
    let mut buf = vec![];
    instruction.encode(&mut buf);
    let mut read = Cursor::new(&buf);
    assert_eq!(StreamCancel::decode(&mut read), Ok(Some(instruction)));
}
