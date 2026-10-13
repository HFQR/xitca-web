use super::super::{
    encode_stateless,
    tests::helpers::{TABLE_SIZE, build_table_with_size},
};
use super::*;

#[test]
fn test_header_too_long() {
    let mut trailers = http::HeaderMap::new();
    trailers.insert("trailer", "value".parse().unwrap());
    trailers.insert("trailer2", "value2".parse().unwrap());
    let mut buf = crate::bytes::BytesMut::new();
    let _ = encode_stateless(&mut buf, crate::h3::proto::headers::Header::trailer(trailers));
    let result = decode_stateless(&mut buf, 2);
    assert_eq!(result, Err(DecoderError::HeaderTooLong(44)));
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-insert-with-name-reference
 * 4.3.2.  Insert With Name Reference
 */
#[test]
fn test_insert_field_with_name_ref_into_dynamic_table() {
    let mut buf = vec![];
    InsertWithNameRef::new_static(1, "serial value")
        .encode(&mut buf)
        .unwrap();
    let mut decoder = Decoder::from(build_table_with_size(0));
    let mut enc = Cursor::new(&buf);
    let mut dec = vec![];
    assert!(decoder.on_encoder_recv(&mut enc, &mut dec).is_ok());

    assert_eq!(
        decoder.table.decoder(1).get_relative(0),
        Ok(&StaticTable::get(1).unwrap().with_value("serial value"))
    );

    let mut dec_cursor = Cursor::new(&dec);
    assert_eq!(
        InsertCountIncrement::decode(&mut dec_cursor),
        Ok(Some(InsertCountIncrement(1)))
    );
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-insert-with-name-reference
 * 4.3.2.  Insert With Name Reference
 */
#[test]
fn test_insert_field_with_wrong_name_index_from_static_table() {
    let mut buf = vec![];
    InsertWithNameRef::new_static(3000, "").encode(&mut buf).unwrap();
    let mut enc = Cursor::new(&buf);
    let mut decoder = Decoder::from(build_table_with_size(0));
    let res = decoder.on_encoder_recv(&mut enc, &mut vec![]);
    assert_eq!(res, Err(DecoderError::InvalidStaticIndex(3000)));
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-insert-with-name-referencehtml
 * 4.3.2.  Insert With Name Reference
 */
#[test]
fn test_insert_field_with_wrong_name_index_from_dynamic_table() {
    let mut buf = vec![];
    InsertWithNameRef::new_dynamic(3000, "").encode(&mut buf).unwrap();
    let mut enc = Cursor::new(&buf);
    let mut dec = vec![];
    let mut decoder = Decoder::from(build_table_with_size(0));
    let res = decoder.on_encoder_recv(&mut enc, &mut dec);
    assert_eq!(
        res,
        Err(DecoderError::DynamicTable(DynamicTableError::BadRelativeIndex(3000)))
    );

    assert!(dec.is_empty());
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-insert-with-literal-name
 * 4.3.3.  Insert with Literal Name
 */
#[test]
fn test_insert_field_without_name_ref() {
    let mut buf = vec![];
    InsertWithoutNameRef::new("key", "value").encode(&mut buf).unwrap();

    let mut decoder = Decoder::from(build_table_with_size(0));
    let mut enc = Cursor::new(&buf);
    let mut dec = vec![];
    assert!(decoder.on_encoder_recv(&mut enc, &mut dec).is_ok());

    assert_eq!(
        decoder.table.decoder(1).get_relative(0),
        Ok(&HeaderField::new("key", "value"))
    );

    let mut dec_cursor = Cursor::new(&dec);
    assert_eq!(
        InsertCountIncrement::decode(&mut dec_cursor),
        Ok(Some(InsertCountIncrement(1)))
    );
}

fn insert_fields(table: &mut DynamicTable, fields: Vec<HeaderField>) {
    for field in fields {
        table.put(field).unwrap();
    }
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-duplicate
 * 4.3.4.  Duplicate
 */
#[test]
fn test_duplicate_field() {
    // let mut table = build_table_with_size(0);
    let mut table = build_table_with_size(0);
    insert_fields(&mut table, vec![HeaderField::new("", ""), HeaderField::new("", "")]);
    let mut decoder = Decoder::from(table);

    let mut buf = vec![];
    Duplicate(1).encode(&mut buf);

    let mut enc = Cursor::new(&buf);
    let mut dec = vec![];
    let res = decoder.on_encoder_recv(&mut enc, &mut dec);
    assert_eq!(res, Ok(3));

    let mut dec_cursor = Cursor::new(&dec);
    assert_eq!(
        InsertCountIncrement::decode(&mut dec_cursor),
        Ok(Some(InsertCountIncrement(1)))
    );
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-set-dynamic-table-capacity
 * 4.3.1.  Set Dynamic Table Capacity
 */
#[test]
fn test_dynamic_table_size_update() {
    let mut buf = vec![];
    DynamicTableSizeUpdate(25).encode(&mut buf);

    let mut enc = Cursor::new(&buf);
    let mut dec = vec![];
    let mut decoder = Decoder::from(build_table_with_size(0));
    let res = decoder.on_encoder_recv(&mut enc, &mut dec);
    assert_eq!(res, Ok(0));

    let actual_max_size = decoder.table.max_mem_size();
    assert_eq!(actual_max_size, 25);
    assert!(dec.is_empty());
}

#[test]
fn enc_recv_buf_too_short() {
    let decoder = Decoder::from(build_table_with_size(0));
    let mut buf = vec![];
    {
        let mut enc = Cursor::new(&buf);
        assert_eq!(decoder.parse_instruction(&mut enc), Ok(None));
    }

    buf.push(0b1000_0000);
    let mut enc = Cursor::new(&buf);
    assert_eq!(decoder.parse_instruction(&mut enc), Ok(None));
}

#[test]
fn enc_recv_accepts_truncated_messages() {
    let mut buf = vec![];
    InsertWithoutNameRef::new("keyfoobarbaz", "value")
        .encode(&mut buf)
        .unwrap();

    let mut decoder = Decoder::from(build_table_with_size(0));
    // cut in middle of the first int
    let mut enc = Cursor::new(&buf[..2]);
    let mut dec = vec![];
    assert!(decoder.on_encoder_recv(&mut enc, &mut dec).is_ok());
    assert_eq!(enc.position(), 0);

    // cut the last byte of the 2nd string
    let mut enc = Cursor::new(&buf[..buf.len() - 1]);
    let mut dec = vec![];
    assert!(decoder.on_encoder_recv(&mut enc, &mut dec).is_ok());
    assert_eq!(enc.position(), 0);

    InsertWithoutNameRef::new("keyfoobarbaz2", "value")
        .encode(&mut buf)
        .unwrap();

    // the first valid field is inserted and buf is left at the first byte of incomplete string
    let mut enc = Cursor::new(&buf[..buf.len() - 1]);
    let mut dec = vec![];
    assert!(decoder.on_encoder_recv(&mut enc, &mut dec).is_ok());
    assert_eq!(enc.position(), 15);

    let mut dec_cursor = Cursor::new(&dec);
    assert_eq!(
        InsertCountIncrement::decode(&mut dec_cursor),
        Ok(Some(InsertCountIncrement(1)))
    );
}

#[test]
fn largest_ref_too_big() {
    let decoder = Decoder::from(build_table_with_size(0));
    let mut buf = vec![];
    HeaderPrefix::new(8, 8, 10, TABLE_SIZE).encode(&mut buf);

    let mut read = Cursor::new(&buf);
    assert_eq!(decoder.decode_header(&mut read), Err(DecoderError::MissingRefs(8)));
}

fn field(n: usize) -> HeaderField {
    HeaderField::new(format!("foo{}", n), "bar")
}

// Largest Reference
//   Base Index = 2
//       |
//     foo2   foo1
//    +-----+-----+
//    |  2  |  1  |  Absolute Index
//    +-----+-----+
//    |  0  |  1  |  Relative Index
//    --+---+-----+

#[test]
fn decode_indexed_header_field() {
    let mut buf = vec![];
    HeaderPrefix::new(2, 2, 2, TABLE_SIZE).encode(&mut buf);
    Indexed::Dynamic(0).encode(&mut buf);
    Indexed::Dynamic(1).encode(&mut buf);
    Indexed::Static(18).encode(&mut buf);

    let mut read = Cursor::new(&buf);
    let decoder = Decoder::from(build_table_with_size(2));
    let Decoded { fields, dyn_ref, .. } = decoder.decode_header(&mut read).unwrap();
    assert!(dyn_ref);
    assert_eq!(fields, &[field(2), field(1), StaticTable::get(18).unwrap().clone()])
}

//      Largest Reference
//        Base Index = 2
//             |
// foo4 foo3  foo2  foo1
// +---+-----+-----+-----+
// | 4 |  3  |  2  |  1  |  Absolute Index
// +---+-----+-----+-----+
//           |  0  |  1  |  Relative Index
// +-----+-----+---+-----+
// | 1 |  0  |              Post-Base Index
// +---+-----+

#[test]
fn decode_post_base_indexed() {
    let mut buf = vec![];
    HeaderPrefix::new(4, 2, 4, TABLE_SIZE).encode(&mut buf);
    Indexed::Dynamic(0).encode(&mut buf);
    IndexedWithPostBase(0).encode(&mut buf);
    IndexedWithPostBase(1).encode(&mut buf);

    let mut read = Cursor::new(&buf);
    let decoder = Decoder::from(build_table_with_size(4));
    let Decoded { fields, dyn_ref, .. } = decoder.decode_header(&mut read).unwrap();
    assert!(dyn_ref);
    assert_eq!(fields, &[field(2), field(3), field(4)])
}

#[test]
fn decode_name_ref_header_field() {
    let mut buf = vec![];
    HeaderPrefix::new(2, 2, 4, TABLE_SIZE).encode(&mut buf);
    LiteralWithNameRef::new_dynamic(1, "new bar1").encode(&mut buf).unwrap();
    LiteralWithNameRef::new_static(18, "PUT").encode(&mut buf).unwrap();

    let mut read = Cursor::new(&buf);
    let decoder = Decoder::from(build_table_with_size(4));
    let Decoded { fields, dyn_ref, .. } = decoder.decode_header(&mut read).unwrap();
    assert!(dyn_ref);
    assert_eq!(
        fields,
        &[
            field(1).with_value("new bar1"),
            StaticTable::get(18).unwrap().with_value("PUT")
        ]
    )
}

#[test]
fn decode_post_base_name_ref_header_field() {
    let mut buf = vec![];
    HeaderPrefix::new(2, 2, 4, TABLE_SIZE).encode(&mut buf);
    LiteralWithPostBaseNameRef::new(0, "new bar3").encode(&mut buf).unwrap();

    let mut read = Cursor::new(&buf);
    let decoder = Decoder::from(build_table_with_size(4));
    let Decoded { fields, .. } = decoder.decode_header(&mut read).unwrap();
    assert_eq!(fields, &[field(3).with_value("new bar3")]);
}

#[test]
fn decode_without_name_ref_header_field() {
    let mut buf = vec![];
    HeaderPrefix::new(0, 0, 0, TABLE_SIZE).encode(&mut buf);
    Literal::new("foo", "bar").encode(&mut buf).unwrap();

    let mut read = Cursor::new(&buf);
    let decoder = Decoder::from(build_table_with_size(0));
    let Decoded { fields, .. } = decoder.decode_header(&mut read).unwrap();
    assert_eq!(fields, &[HeaderField::new(b"foo".to_vec(), b"bar".to_vec())]);
}

// Largest Reference = 4
//  |            Base Index = 0
//  |                |
// foo4 foo3  foo2  foo1
// +---+-----+-----+-----+
// | 4 |  3  |  2  |  1  |  Absolute Index
// +---+-----+-----+-----+
//                          Relative Index
// +---+-----+-----+-----+
// | 2 |   2 |  1  |  0  |  Post-Base Index
// +---+-----+-----+-----+

#[test]
fn decode_single_pass_encoded() {
    let mut buf = vec![];
    HeaderPrefix::new(4, 0, 4, TABLE_SIZE).encode(&mut buf);
    IndexedWithPostBase(0).encode(&mut buf);
    IndexedWithPostBase(1).encode(&mut buf);
    IndexedWithPostBase(2).encode(&mut buf);
    IndexedWithPostBase(3).encode(&mut buf);

    let mut read = Cursor::new(&buf);
    let decoder = Decoder::from(build_table_with_size(4));
    let Decoded { fields, .. } = decoder.decode_header(&mut read).unwrap();
    assert_eq!(fields, &[field(1), field(2), field(3), field(4)]);
}

#[test]
fn largest_ref_greater_than_max_entries() {
    let max_entries = TABLE_SIZE / 32;
    // some fields evicted
    let table = build_table_with_size(max_entries + 10);
    let mut buf = vec![];

    // Pre-base relative reference
    HeaderPrefix::new(max_entries + 5, max_entries + 5, max_entries + 10, TABLE_SIZE).encode(&mut buf);
    Indexed::Dynamic(10).encode(&mut buf);

    let mut read = Cursor::new(&buf);
    let decoder = Decoder::from(build_table_with_size(max_entries + 10));
    let Decoded { fields, .. } = decoder.decode_header(&mut read).expect("decode");
    assert_eq!(fields, &[field(max_entries - 5)]);

    let mut buf = vec![];

    // Post-base reference
    HeaderPrefix::new(max_entries + 10, max_entries + 5, max_entries + 10, TABLE_SIZE).encode(&mut buf);
    IndexedWithPostBase(0).encode(&mut buf);
    IndexedWithPostBase(4).encode(&mut buf);

    let mut read = Cursor::new(&buf);
    let decoder = Decoder::from(table);
    let Decoded { fields, .. } = decoder.decode_header(&mut read).unwrap();
    assert_eq!(fields, &[field(max_entries + 6), field(max_entries + 10)]);
}
