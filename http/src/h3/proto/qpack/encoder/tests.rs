use super::*;

use super::super::tests::helpers::{TABLE_SIZE, build_table};

#[allow(clippy::type_complexity)]
fn check_encode_field(
    init_fields: &[HeaderField],
    field: &[HeaderField],
    check: &dyn Fn(&mut Cursor<&mut Vec<u8>>, &mut Cursor<&mut Vec<u8>>),
) {
    let mut table = build_table();
    table.set_max_size(TABLE_SIZE).unwrap();
    check_encode_field_table(&mut table, init_fields, field, 1, check);
}

#[allow(clippy::type_complexity)]
fn check_encode_field_table(
    table: &mut DynamicTable,
    init_fields: &[HeaderField],
    field: &[HeaderField],
    stream_id: u64,
    check: &dyn Fn(&mut Cursor<&mut Vec<u8>>, &mut Cursor<&mut Vec<u8>>),
) {
    for field in init_fields {
        table.put(field.clone()).unwrap();
    }

    let mut encoder = Vec::new();
    let mut block = Vec::new();
    let mut enc_table = table.encoder(stream_id);

    for field in field {
        Encoder::encode_field(&mut enc_table, &mut block, &mut encoder, field).unwrap();
    }

    enc_table.commit(field.len());

    let mut read_block = Cursor::new(&mut block);
    let mut read_encoder = Cursor::new(&mut encoder);
    check(&mut read_block, &mut read_encoder);
}

#[test]
fn encode_static() {
    let field = HeaderField::new(":method", "GET");
    check_encode_field(&[], &[field], &|mut b, e| {
        assert_eq!(Indexed::decode(&mut b), Ok(Indexed::Static(17)));
        assert_eq!(e.get_ref().len(), 0);
    });
}

#[test]
fn encode_static_nameref() {
    let field = HeaderField::new("location", "/bar");
    check_encode_field(&[], &[field], &|mut b, mut e| {
        assert_eq!(IndexedWithPostBase::decode(&mut b), Ok(IndexedWithPostBase(0)));
        assert_eq!(
            InsertWithNameRef::decode(&mut e),
            Ok(Some(InsertWithNameRef::new_static(12, "/bar")))
        );
    });
}

#[test]
fn encode_static_nameref_indexed_in_dynamic() {
    let field = HeaderField::new("location", "/bar");
    check_encode_field(std::slice::from_ref(&field.clone()), &[field], &|mut b, e| {
        assert_eq!(Indexed::decode(&mut b), Ok(Indexed::Dynamic(0)));
        assert_eq!(e.get_ref().len(), 0);
    });
}

#[test]
fn encode_dynamic_insert() {
    let field = HeaderField::new("foo", "bar");
    check_encode_field(&[], &[field], &|mut b, mut e| {
        assert_eq!(IndexedWithPostBase::decode(&mut b), Ok(IndexedWithPostBase(0)));
        assert_eq!(
            InsertWithoutNameRef::decode(&mut e),
            Ok(Some(InsertWithoutNameRef::new("foo", "bar")))
        );
    });
}

#[test]
fn encode_dynamic_insert_nameref() {
    let field = HeaderField::new("foo", "bar");
    check_encode_field(
        &[field.clone(), HeaderField::new("baz", "bar")],
        &[field.with_value("quxx")],
        &|mut b, mut e| {
            assert_eq!(IndexedWithPostBase::decode(&mut b), Ok(IndexedWithPostBase(0)));
            assert_eq!(
                InsertWithNameRef::decode(&mut e),
                Ok(Some(InsertWithNameRef::new_dynamic(1, "quxx")))
            );
        },
    );
}

#[test]
fn encode_literal() {
    let mut table = build_table();
    table.set_max_size(0).unwrap();
    let field = HeaderField::new("foo", "bar");
    check_encode_field_table(&mut table, &[], &[field], 1, &|mut b, e| {
        assert_eq!(Literal::decode(&mut b), Ok(Literal::new("foo", "bar")));
        assert_eq!(e.get_ref().len(), 0);
    });
}

#[test]
fn encode_literal_nameref() {
    let mut table = build_table();
    table.set_max_size(63).unwrap();
    let field = HeaderField::new("foo", "bar");

    check_encode_field_table(&mut table, &[], std::slice::from_ref(&field), 1, &|mut b, _| {
        assert_eq!(IndexedWithPostBase::decode(&mut b), Ok(IndexedWithPostBase(0)));
    });
    check_encode_field_table(
        &mut table,
        std::slice::from_ref(&field),
        &[field.with_value("quxx")],
        2,
        &|mut b, e| {
            assert_eq!(
                LiteralWithNameRef::decode(&mut b),
                Ok(LiteralWithNameRef::new_dynamic(0, "quxx"))
            );
            assert_eq!(e.get_ref().len(), 0);
        },
    );
}

#[test]
fn encode_literal_postbase_nameref() {
    let mut table = build_table();
    table.set_max_size(63).unwrap();
    let field = HeaderField::new("foo", "bar");
    check_encode_field_table(
        &mut table,
        &[],
        &[field.clone(), field.with_value("quxx")],
        1,
        &|mut b, mut e| {
            assert_eq!(IndexedWithPostBase::decode(&mut b), Ok(IndexedWithPostBase(0)));
            assert_eq!(
                LiteralWithPostBaseNameRef::decode(&mut b),
                Ok(LiteralWithPostBaseNameRef::new(0, "quxx"))
            );
            assert_eq!(
                InsertWithoutNameRef::decode(&mut e),
                Ok(Some(InsertWithoutNameRef::new("foo", "bar")))
            );
        },
    );
}

#[test]
fn encode_with_header_block() {
    let mut table = build_table();

    for idx in 1..5 {
        table
            .put(HeaderField::new(format!("foo{}", idx), format!("bar{}", idx)))
            .unwrap();
    }

    let mut encoder_buf = Vec::new();
    let mut block = Vec::new();
    let mut encoder = Encoder::from(table);

    let fields = vec![
        HeaderField::new(":method", "GET"),
        HeaderField::new("foo1", "bar1"),
        HeaderField::new("foo3", "new bar3"),
        HeaderField::new(":method", "staticnameref"),
        HeaderField::new("newfoo", "newbar"),
    ]
    .into_iter();

    assert_eq!(encoder.encode(1, &mut block, &mut encoder_buf, fields), Ok(7));

    let mut read_block = Cursor::new(&mut block);
    let mut read_encoder = Cursor::new(&mut encoder_buf);

    assert_eq!(
        InsertWithNameRef::decode(&mut read_encoder),
        Ok(Some(InsertWithNameRef::new_dynamic(1, "new bar3")))
    );
    assert_eq!(
        InsertWithNameRef::decode(&mut read_encoder),
        Ok(Some(InsertWithNameRef::new_static(
            StaticTable::find_name(&b":method"[..]).unwrap(),
            "staticnameref"
        )))
    );
    assert_eq!(
        InsertWithoutNameRef::decode(&mut read_encoder),
        Ok(Some(InsertWithoutNameRef::new("newfoo", "newbar")))
    );

    assert_eq!(
        HeaderPrefix::decode(&mut read_block).unwrap().get(7, TABLE_SIZE),
        Ok((7, 4))
    );
    assert_eq!(Indexed::decode(&mut read_block), Ok(Indexed::Static(17)));
    assert_eq!(Indexed::decode(&mut read_block), Ok(Indexed::Dynamic(3)));
    assert_eq!(IndexedWithPostBase::decode(&mut read_block), Ok(IndexedWithPostBase(0)));
    assert_eq!(IndexedWithPostBase::decode(&mut read_block), Ok(IndexedWithPostBase(1)));
    assert_eq!(IndexedWithPostBase::decode(&mut read_block), Ok(IndexedWithPostBase(2)));
    assert_eq!(read_block.get_ref().len() as u64, read_block.position());
}

#[test]
fn decoder_block_ack() {
    let mut table = build_table();

    let field = HeaderField::new("foo", "bar");
    check_encode_field_table(
        &mut table,
        &[],
        &[field.clone(), field.with_value("quxx")],
        2,
        &|_, _| {},
    );

    let mut buf = vec![];
    let mut encoder = Encoder::from(table);

    HeaderAck(2).encode(&mut buf);
    let mut cur = Cursor::new(&buf);
    assert_eq!(Action::parse(&mut cur), Ok(Some(Action::Untrack(2))));

    let mut cur = Cursor::new(&buf);
    assert_eq!(encoder.on_decoder_recv(&mut cur), Ok(()),);

    let mut cur = Cursor::new(&buf);
    assert_eq!(
        encoder.on_decoder_recv(&mut cur),
        Err(EncoderError::Insertion(DynamicTableError::UnknownStreamId(2)))
    );
}

#[test]
fn decoder_stream_cacnceled() {
    let mut table = build_table();

    let field = HeaderField::new("foo", "bar");
    check_encode_field_table(
        &mut table,
        &[],
        &[field.clone(), field.with_value("quxx")],
        2,
        &|_, _| {},
    );

    let mut buf = vec![];

    StreamCancel(2).encode(&mut buf);
    let mut cur = Cursor::new(&buf);
    assert_eq!(Action::parse(&mut cur), Ok(Some(Action::StreamCancel(2))));
}

#[test]
fn decoder_accept_truncated() {
    let mut buf = vec![];
    StreamCancel(2321).encode(&mut buf);

    let mut cur = Cursor::new(&buf[..2]); // trucated prefix_int
    assert_eq!(Action::parse(&mut cur), Ok(None));

    let mut cur = Cursor::new(&buf);
    assert_eq!(Action::parse(&mut cur), Ok(Some(Action::StreamCancel(2321))));
}

#[test]
fn decoder_unknown_stream() {
    let mut table = build_table();

    check_encode_field_table(&mut table, &[], &[HeaderField::new("foo", "bar")], 2, &|_, _| {});
    let mut encoder = Encoder::from(table);

    let mut buf = vec![];
    HeaderAck(4).encode(&mut buf);

    let mut cur = Cursor::new(&buf);
    assert_eq!(
        encoder.on_decoder_recv(&mut cur),
        Err(EncoderError::Insertion(DynamicTableError::UnknownStreamId(4)))
    );
}

#[test]
fn insert_count() {
    let mut buf = vec![];
    InsertCountIncrement(4).encode(&mut buf);

    let mut cur = Cursor::new(&buf);
    assert_eq!(Action::parse(&mut cur), Ok(Some(Action::ReceivedRefIncrement(4))));

    let mut encoder = Encoder { table: build_table() };

    let mut cur = Cursor::new(&buf);
    assert_eq!(encoder.on_decoder_recv(&mut cur), Ok(()));
}
