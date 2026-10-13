#![allow(clippy::identity_op)]

use super::super::{static_::StaticTable, tests::helpers::build_table};
use super::*;

const STREAM_ID: u64 = 0x4;

// Test on table size
/**
 * https://tools.ietf.org/html/rfc7541#section-4.1
 * "The size of the dynamic table is the sum of the size of its entries."
 */
#[test]
fn test_table_size_is_sum_of_its_entries() {
    let mut table = build_table();

    let fields: [(&'static str, &'static str); 2] = [
        ("Name", "Value"),
        ("Another-Name", ""), // no value
    ];
    let table_size = 4 + 5 + 12 + 0 + /* ESTIMATED_OVERHEAD_BYTES */ 32 * 2;

    for pair in fields.iter() {
        let field = HeaderField::new(pair.0, pair.1);
        table.insert(field).unwrap();
    }

    assert_eq!(table.curr_size, table_size);
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-maximum-dynamic-table-capac
 * "To bound the memory requirements of the decoder, the decoder
 * limits the maximum value the encoder is permitted to set for the
 * dynamic table capacity. In HTTP/3, this limit is determined by
 * the value of SETTINGS_QPACK_MAX_TABLE_CAPACITY sent by the
 * decoder; see Section 5."
 */
#[test]
fn test_try_set_too_large_maximum_table_size() {
    let mut table = build_table();
    let invalid_size = SETTINGS_MAX_TABLE_CAPACITY_MAX + 10;
    let res_change = table.set_max_size(invalid_size);
    assert_eq!(res_change, Err(Error::MaximumTableSizeTooLarge));
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-dynamic-table-capacity-and-
 * "This mechanism can be used to completely clear entries from the
 *  dynamic table by setting a maximum size of 0, which can subsequently
 *  be restored."
 */
#[test]
fn test_maximum_table_size_can_reach_zero() {
    let mut table = build_table();
    let res_change = table.set_max_size(0);
    assert!(res_change.is_ok());
    assert_eq!(table.max_mem_size(), 0);
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-maximum-dynamic-table-capac
 * "To bound the memory requirements of the decoder, the decoder
 * limits the maximum value the encoder is permitted to set for the
 * dynamic table capacity. In HTTP/3, this limit is determined by
 * the value of SETTINGS_QPACK_MAX_TABLE_CAPACITY sent by the
 * decoder; see Section 5."
 */
#[test]
fn test_maximum_table_size_can_reach_maximum() {
    let mut table = build_table();
    let res_change = table.set_max_size(SETTINGS_MAX_TABLE_CAPACITY_MAX);
    assert!(res_change.is_ok());
    assert_eq!(table.max_mem_size(), SETTINGS_MAX_TABLE_CAPACITY_MAX);
}

// Test duplicated fields

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-dynamic-table
 * "The dynamic table can contain duplicate entries (i.e., entries with
 *  the same name and same value).  Therefore, duplicate entries MUST NOT
 *  be treated as an error by a decoder."
 */
#[test]
fn test_table_supports_duplicated_entries() {
    let mut table = build_table();
    table.insert(HeaderField::new("Name", "Value")).unwrap();
    table.insert(HeaderField::new("Name", "Value")).unwrap();
    assert_eq!(table.fields.len(), 2);
}

// Test adding fields

/** functional test */
#[test]
fn test_add_field_fitting_free_space() {
    let mut table = build_table();

    table.insert(HeaderField::new("Name", "Value")).unwrap();
    assert_eq!(table.fields.len(), 1);
}

/** functional test */
#[test]
fn test_add_field_reduce_free_space() {
    let mut table = build_table();

    let field = HeaderField::new("Name", "Value");
    table.insert(field.clone()).unwrap();
    assert_eq!(table.curr_size, field.mem_size());
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-dynamic-table-capacity-and-
 * "Before a new entry is added to the dynamic table, entries are evicted
 *  from the end of the dynamic table until the size of the dynamic table
 *  is less than or equal to (maximum size - new entry size) or until the
 *  table is empty."
 */
#[test]
fn test_add_field_drop_older_fields_to_have_enough_space() {
    let mut table = build_table();

    table.insert(HeaderField::new("Name-A", "Value-A")).unwrap();
    table.insert(HeaderField::new("Name-B", "Value-B")).unwrap();
    let perfect_size = table.curr_size;
    assert!(table.set_max_size(perfect_size).is_ok());

    let field = HeaderField::new("Name-Large", "Value-Large");
    table.insert(field).unwrap();

    assert_eq!(table.fields.len(), 1);
    assert_eq!(
        table.fields.front(),
        Some(&HeaderField::new("Name-Large", "Value-Large"))
    );
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-dynamic-table-capacity-and-
 * "It is an error if the encoder attempts to add an entry that is
 * larger than the dynamic table capacity; the decoder MUST treat
 * this as a connection error of type QPACK_ENCODER_STREAM_ERROR."
 */
#[test]
fn test_try_add_field_larger_than_maximum_size() {
    let mut table = build_table();

    table.insert(HeaderField::new("Name-A", "Value-A")).unwrap();
    let perfect_size = table.curr_size;
    assert!(table.set_max_size(perfect_size).is_ok());

    let field = HeaderField::new("Name-Large", "Value-Large");
    assert_eq!(table.insert(field), Err(Error::MaxTableSizeReached));
}

fn insert_fields(table: &mut DynamicTable, fields: Vec<HeaderField>) {
    for field in fields {
        table.insert(field).unwrap();
    }
}

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-dynamic-table-capacity-and-
 * "This mechanism can be used to completely clear entries from the
 *  dynamic table by setting a maximum size of 0, which can subsequently
 *  be restored."
 */
#[test]
fn test_set_maximum_table_size_to_zero_clear_entries() {
    let mut table = build_table();
    insert_fields(
        &mut table,
        vec![HeaderField::new("Name", "Value"), HeaderField::new("Name", "Value")],
    );
    assert_eq!(table.fields.len(), 2);

    table.set_max_size(0).unwrap();
    assert_eq!(table.fields.len(), 0);
}

/** functional test */
#[test]
fn test_eviction_is_fifo() {
    let mut table = build_table();

    insert_fields(
        &mut table,
        vec![
            HeaderField::new("Name-A", "Value-A"),
            HeaderField::new("Name-B", "Value-B"),
        ],
    );
    let perfect_size = table.curr_size;
    assert!(table.set_max_size(perfect_size).is_ok());

    insert_fields(&mut table, vec![HeaderField::new("Name-C", "Value-C")]);

    assert_eq!(table.fields.front(), Some(&HeaderField::new("Name-B", "Value-B")));
    assert_eq!(table.fields.get(1), Some(&HeaderField::new("Name-C", "Value-C")));
    assert_eq!(table.fields.get(2), None);
}

#[test]
fn encoder_build() {
    let mut table = build_table();
    let field_a = HeaderField::new("Name-A", "Value-A");
    let field_b = HeaderField::new("Name-B", "Value-B");
    insert_fields(&mut table, vec![field_a.clone(), field_b.clone()]);

    let encoder = table.encoder(STREAM_ID);
    assert_eq!(encoder.base, 2);
    assert_eq!(encoder.table.name_map.len(), 2);
    assert_eq!(encoder.table.field_map.len(), 2);
    assert_eq!(encoder.table.name_map.get(&field_a.name).copied(), Some(1));
    assert_eq!(encoder.table.name_map.get(&field_b.name).copied(), Some(2));
    assert_eq!(encoder.table.field_map.get(&field_a).copied(), Some(1));
    assert_eq!(encoder.table.field_map.get(&field_b).copied(), Some(2));
}

#[test]
fn encoder_find_relative() {
    let mut table = build_table();
    let field_a = HeaderField::new("Name-A", "Value-A");
    let field_b = HeaderField::new("Name-B", "Value-B");
    insert_fields(&mut table, vec![field_a.clone(), field_b.clone()]);

    let mut encoder = table.encoder(STREAM_ID);
    assert_eq!(
        encoder.find(&field_a),
        DynamicLookupResult::Relative { index: 1, absolute: 1 }
    );
    assert_eq!(
        encoder.find(&field_b),
        DynamicLookupResult::Relative { index: 0, absolute: 2 }
    );
    assert_eq!(
        encoder.find(&HeaderField::new("Name-C", "Value-C")),
        DynamicLookupResult::NotFound
    );
    assert_eq!(
        encoder.find_name(&field_a.name),
        DynamicLookupResult::Relative { index: 1, absolute: 1 }
    );
    assert_eq!(
        encoder.find_name(&field_b.name),
        DynamicLookupResult::Relative { index: 0, absolute: 2 }
    );
    assert_eq!(encoder.find_name(&b"Name-C"[..]), DynamicLookupResult::NotFound);
}

#[test]
fn encoder_insert() {
    let mut table = build_table();
    let field_a = HeaderField::new("Name-A", "Value-A");
    let field_b = HeaderField::new("Name-B", "Value-B");
    insert_fields(&mut table, vec![field_a.clone(), field_b.clone()]);

    let mut encoder = table.encoder(STREAM_ID);
    assert_eq!(
        encoder.insert(&field_a),
        Ok(DynamicInsertionResult::Duplicated {
            postbase: 0,
            relative: 1,
            absolute: 3
        })
    );
    assert_eq!(
        encoder.insert(&field_b.with_value("New Value-B")),
        Ok(DynamicInsertionResult::InsertedWithNameRef {
            postbase: 1,
            relative: 1,
            absolute: 4,
        })
    );
    assert_eq!(
        encoder.insert(&field_b.with_value("Newer Value-B")),
        Ok(DynamicInsertionResult::InsertedWithNameRef {
            postbase: 2,
            relative: 0,
            absolute: 5,
        })
    );

    let field_c = HeaderField::new("Name-C", "Value-C");
    assert_eq!(
        encoder.insert(&field_c),
        Ok(DynamicInsertionResult::Inserted {
            postbase: 3,
            absolute: 6,
        })
    );

    assert_eq!(encoder.table.fields.len(), 6);

    assert_eq!(
        encoder.table.fields,
        &[
            field_a.clone(),
            field_b.clone(),
            field_a.clone(),
            field_b.with_value("New Value-B"),
            field_b.with_value("Newer Value-B"),
            field_c
        ]
    );
    assert_eq!(encoder.table.name_map.get(&field_a.name).copied(), Some(3));
    assert_eq!(encoder.table.name_map.get(&field_b.name).copied(), Some(5));
    assert_eq!(encoder.table.field_map.get(&field_a).copied(), Some(3));
    assert_eq!(encoder.table.field_map.get(&field_b).copied(), Some(2));
    assert_eq!(
        encoder.table.field_map.get(&field_b.with_value("New Value-B")).copied(),
        Some(4)
    );
    assert_eq!(
        encoder
            .table
            .field_map
            .get(&field_b.with_value("Newer Value-B"))
            .copied(),
        Some(5)
    );
}

#[test]
fn encode_insert_in_empty() {
    let mut table = build_table();
    let field_a = HeaderField::new("Name-A", "Value-A");

    let mut encoder = table.encoder(STREAM_ID);
    assert_eq!(
        encoder.insert(&field_a),
        Ok(DynamicInsertionResult::Inserted {
            postbase: 0,
            absolute: 1,
        })
    );

    assert_eq!(encoder.table.fields.len(), 1);
    assert_eq!(encoder.table.fields, std::slice::from_ref(&field_a));
    assert_eq!(encoder.table.name_map.get(&field_a.name).copied(), Some(1));
    assert_eq!(encoder.table.field_map.get(&field_a).copied(), Some(1));
}

#[test]
fn insert_static() {
    let mut table = build_table();
    let field = HeaderField::new(":method", "Value-A");
    table.insert(field.clone()).unwrap();

    assert_eq!(StaticTable::find_name(&field.name), Some(15));
    let mut encoder = table.encoder(STREAM_ID);
    assert_eq!(
        encoder.insert(&field),
        Ok(DynamicInsertionResult::Duplicated {
            relative: 0,
            postbase: 0,
            absolute: 2
        })
    );
    assert_eq!(
        encoder.insert(&field.with_value("Value-B")),
        Ok(DynamicInsertionResult::InsertedWithStaticNameRef {
            postbase: 1,
            index: 15,
            absolute: 3
        })
    );
    assert_eq!(
        encoder.insert(&HeaderField::new(":path", "/baz")),
        Ok(DynamicInsertionResult::InsertedWithStaticNameRef {
            postbase: 2,
            index: 1,
            absolute: 4,
        })
    );
    assert_eq!(encoder.table.fields.len(), 4);
}

#[test]
fn cannot_insert_field_greater_than_total_size() {
    let mut table = build_table();
    table.set_max_size(33).unwrap();
    let mut encoder = table.encoder(4);
    assert_eq!(
        encoder.insert(&HeaderField::new("foo", "bar")),
        Ok(DynamicInsertionResult::NotInserted(DynamicLookupResult::NotFound))
    );
}

#[test]
fn encoder_maps_are_cleaned_on_eviction() {
    let mut table = build_table();
    table.set_max_size(64).unwrap();

    {
        let mut encoder = table.encoder(4);
        assert_eq!(
            encoder.insert(&HeaderField::new("foo", "bar")),
            Ok(DynamicInsertionResult::Inserted {
                postbase: 0,
                absolute: 1
            })
        );
        encoder.commit(1);
    }
    table.untrack_block(4).unwrap();

    {
        let mut encoder = table.encoder(4);
        assert_eq!(
            encoder.insert(&HeaderField::new("foo2", "bar")),
            Ok(DynamicInsertionResult::Inserted {
                postbase: 0,
                absolute: 2
            })
        );
        assert_eq!(
            encoder.find(&HeaderField::new("foo", "bar")),
            DynamicLookupResult::NotFound
        );
        assert_eq!(encoder.find_name(b"foo"), DynamicLookupResult::NotFound);
        encoder.commit(2);
    }
}

#[test]
fn encoder_can_evict_unreferenced() {
    let mut table = build_table();
    table.set_max_size(63).unwrap();
    table.insert(HeaderField::new("foo", "bar")).unwrap();

    assert_eq!(table.fields.len(), 1);
    assert_eq!(
        table.encoder(4).insert(&HeaderField::new("baz", "quxx")),
        Ok(DynamicInsertionResult::Inserted {
            postbase: 0,
            absolute: 2,
        })
    );
    assert_eq!(table.fields.len(), 1);
}

#[test]
fn encoder_insertion_tracks_ref() {
    let mut table = build_table();
    let mut encoder = table.encoder(4);
    assert_eq!(
        encoder.insert(&HeaderField::new("baz", "quxx")),
        Ok(DynamicInsertionResult::Inserted {
            postbase: 0,
            absolute: 1,
        })
    );
    assert_eq!(encoder.table.track_map.get(&1).copied(), Some(1));
    assert_eq!(encoder.block_refs.get(&1).copied(), Some(1));
}

#[test]
fn encoder_insertion_refs_commited() {
    let mut table = build_table();
    let stream_id = 42;
    {
        let mut encoder = table.encoder(stream_id);
        for idx in 1..4 {
            encoder
                .insert(&HeaderField::new(format!("foo{}", idx), "quxx"))
                .unwrap();
        }
        assert_eq!(encoder.block_refs.len(), 3);
        encoder.commit(2);
    }

    for idx in 1..4 {
        assert!(table.is_tracked(idx));
        assert_eq!(table.track_map.get(&1), Some(&1));
    }
    let track_blocks = table.track_blocks;
    let block = track_blocks.get(&stream_id).unwrap().front().unwrap();
    assert_eq!(block.get(&1), Some(&1));
    assert_eq!(block.get(&2), Some(&1));
    assert_eq!(block.get(&3), Some(&1));
}

#[test]
fn encoder_insertion_refs_not_commited() {
    let mut table = build_table();
    table.track_blocks = HashMap::new();
    let stream_id = 42;
    {
        let mut encoder = table.encoder(stream_id);
        for idx in 1..4 {
            encoder
                .insert(&HeaderField::new(format!("foo{}", idx), "quxx"))
                .unwrap();
        }
        assert_eq!(encoder.block_refs.len(), 3);
    } // dropped without ::commit()

    assert_eq!(table.track_map.len(), 0);
    assert_eq!(table.track_blocks.len(), 0);
}

#[test]
fn encoder_insertion_with_ref_tracks_both() {
    let mut table = build_table();
    table.insert(HeaderField::new("foo", "bar")).unwrap();
    table.track_blocks = HashMap::new();

    let stream_id = 42;
    let mut encoder = table.encoder(stream_id);
    assert_eq!(
        encoder.insert(&HeaderField::new("foo", "quxx")),
        Ok(DynamicInsertionResult::InsertedWithNameRef {
            postbase: 0,
            relative: 0,
            absolute: 2,
        })
    );

    assert_eq!(encoder.table.track_map.get(&1), Some(&1));
    assert_eq!(encoder.table.track_map.get(&2), Some(&1));
    assert_eq!(encoder.block_refs.get(&1), Some(&1));
    assert_eq!(encoder.block_refs.get(&2), Some(&1));
}

#[test]
fn encoder_ref_count_are_incremented() {
    let mut table = build_table();
    table.insert(HeaderField::new("foo", "bar")).unwrap();
    table.track_blocks = HashMap::new();
    table.track_ref(1);

    let stream_id = 42;
    {
        let mut encoder = table.encoder(stream_id);
        encoder.track_ref(1);
        encoder.track_ref(2);
        encoder.track_ref(2);

        assert_eq!(encoder.table.track_map.get(&1), Some(&2));
        assert_eq!(encoder.table.track_map.get(&2), Some(&2));
        assert_eq!(encoder.block_refs.get(&1), Some(&1));
        assert_eq!(encoder.block_refs.get(&2), Some(&2));
    }

    // check ref count is correctly decremented after uncommited drop()
    assert_eq!(table.track_map.get(&1), Some(&1));
    assert_eq!(table.track_map.get(&2), None);
}

#[test]
fn encoder_does_not_evict_referenced() {
    let mut table = build_table();
    table.set_max_size(95).unwrap();
    table.insert(HeaderField::new("foo", "bar")).unwrap();

    let stream_id = 42;
    let mut encoder = table.encoder(stream_id);
    assert_eq!(
        encoder.insert(&HeaderField::new("foo", "quxx")),
        Ok(DynamicInsertionResult::InsertedWithNameRef {
            postbase: 0,
            relative: 0,
            absolute: 2,
        })
    );
    assert!(encoder.table.is_tracked(1));
    assert_eq!(
        encoder.insert(&HeaderField::new("foo", "baz")),
        Ok(DynamicInsertionResult::NotInserted(DynamicLookupResult::PostBase {
            index: 0,
            absolute: 2,
        }))
    );
    assert_eq!(encoder.table.fields.len(), 2);
}

fn tracked_table(stream_id: u64) -> DynamicTable {
    let mut table = build_table();
    table.track_blocks = HashMap::new();
    {
        let mut encoder = table.encoder(stream_id);
        for idx in 1..4 {
            encoder
                .insert(&HeaderField::new(format!("foo{}", idx), "quxx"))
                .unwrap();
        }
        assert_eq!(encoder.block_refs.len(), 3);
        encoder.commit(3);
    }
    table
}

#[test]
fn untrack_block() {
    let mut table = tracked_table(42);
    assert_eq!(table.track_map.len(), 3);
    assert_eq!(table.track_blocks.len(), 1);
    table.untrack_block(42).unwrap();
    assert_eq!(table.track_map.len(), 0);
    assert_eq!(table.track_blocks.len(), 0);
}

#[test]
fn untrack_block_not_in_map() {
    let mut table = tracked_table(42);
    table.track_map.remove(&2);
    assert_eq!(table.untrack_block(42), Err(Error::InvalidTrackingCount));
}

#[test]
fn untrack_block_wrong_count() {
    let mut table = tracked_table(42);
    table.track_blocks.entry(42).and_modify(|x| {
        x.get_mut(0).unwrap().entry(2).and_modify(|c| *c += 1);
    });
    assert_eq!(table.untrack_block(42), Err(Error::InvalidTrackingCount));
}

#[test]
fn untrack_bloc_wrong_stream() {
    let mut table = tracked_table(41);
    assert_eq!(table.untrack_block(42), Err(Error::UnknownStreamId(42)));
}

#[test]
fn untrack_trailers() {
    const STREAM_ID: u64 = 42;
    let mut table = tracked_table(STREAM_ID);
    {
        // encode trailers
        let mut encoder = table.encoder(STREAM_ID);
        for idx in 4..=9 {
            encoder
                .insert(&HeaderField::new(format!("foo{}", idx), "quxx"))
                .unwrap();
        }
        assert_eq!(encoder.block_refs.len(), 6);
        encoder.commit(6);
    }
    assert_eq!(table.untrack_block(STREAM_ID), Ok(()));
    assert!(!table.is_tracked(3));
    assert!(table.is_tracked(5));
    assert_eq!(table.untrack_block(STREAM_ID), Ok(()));
    assert!(!table.is_tracked(6));
    assert_eq!(table.untrack_block(STREAM_ID), Err(Error::UnknownStreamId(STREAM_ID)));
}

#[test]
fn put_updates_maps() {
    let mut table = tracked_table(42);
    assert_eq!(table.name_map.len(), 3);
    assert_eq!(table.field_map.len(), 3);

    table.put(HeaderField::new("foo", "bar")).unwrap();
    assert_eq!(table.name_map.len(), 4);
    assert_eq!(table.field_map.len(), 4);

    let field = HeaderField::new("foo1", "quxx");
    table.put(field.clone()).unwrap();
    assert_eq!(table.name_map.len(), 4);
    assert_eq!(table.field_map.len(), 4);
    assert_eq!(table.name_map.get(&b"foo1"[..]), Some(&5usize));
    assert_eq!(table.field_map.get(&field), Some(&5usize));
}

#[test]
fn blocked_stream_registered() {
    let mut table = tracked_table(42);
    table.set_max_blocked(100).unwrap();

    assert_eq!(table.blocked_count, 1);
    assert_eq!(table.blocked_streams.get(&3), Some(&1usize))
}

#[test]
fn blocked_stream_not_registered() {
    let mut table = tracked_table(42);
    table.set_max_blocked(100).unwrap();

    table.encoder(44).insert(&HeaderField::new("foo", "bar")).unwrap();
    // encoder dropped without commit

    assert_eq!(table.blocked_count, 1);
    assert_eq!(table.blocked_streams.get(&5), None);
}

#[test]
fn blocked_stream_register_accumulate() {
    let mut table = tracked_table(42);
    table.set_max_blocked(100).unwrap();

    {
        let mut encoder = table.encoder(44);

        assert_eq!(
            encoder.find(&HeaderField::new("foo3", "quxx")),
            DynamicLookupResult::Relative { index: 0, absolute: 3 }
        );
        // the encoder inserts a reference to foo3 in a block (absolte index = 3)
        encoder.commit(3);
    }

    assert_eq!(table.blocked_count, 2);
    assert_eq!(table.blocked_streams.get(&3), Some(&2));
}

#[test]
fn blocked_stream_register_put_smaller() {
    let mut table = tracked_table(42);
    table.set_max_blocked(100).unwrap();

    {
        let mut encoder = table.encoder(44);
        encoder.commit(2);
    }

    assert_eq!(table.blocked_count, 2);
    assert_eq!(table.blocked_streams.get(&2), Some(&1));
}

#[test]
fn blocked_stream_register_put_larger() {
    let mut table = tracked_table(42);
    table.set_max_blocked(100).unwrap();

    {
        let mut encoder = table.encoder(44);
        encoder.commit(5);
    }

    assert_eq!(table.blocked_count, 2);
    assert_eq!(table.blocked_streams.get(&5), Some(&1));
}

#[test]
fn unblock_stream_smaller() {
    let mut table = tracked_table(42);
    table.set_max_blocked(100).unwrap();

    {
        let mut encoder = table.encoder(44);
        encoder.commit(2);
    }

    assert_eq!(table.blocked_count, 2);
    assert_eq!(table.blocked_streams.get(&2), Some(&1));

    table.update_largest_received(2);

    assert_eq!(table.blocked_count, 1);
    assert_eq!(table.blocked_streams.get(&2), None);
    assert_eq!(table.blocked_streams.get(&3), Some(&1));
}

#[test]
fn unblock_stream_larger() {
    let mut table = tracked_table(42);
    table.set_max_blocked(100).unwrap();

    table.encoder(44).commit(2);
    table.encoder(46).commit(5);

    assert_eq!(table.blocked_count, 3);
    assert_eq!(table.blocked_streams.get(&2), Some(&1));
    assert_eq!(table.blocked_streams.get(&3), Some(&1));

    table.update_largest_received(5);

    assert_eq!(table.blocked_count, 0);
    assert_eq!(table.blocked_streams.len(), 0);
}

#[test]
fn unblock_stream_decrement() {
    let mut table = tracked_table(42);
    table.set_max_blocked(100).unwrap();

    table.encoder(44).commit(3);

    assert_eq!(table.blocked_count, 2);
    assert_eq!(table.blocked_streams.get(&3), Some(&2));

    table.update_largest_received(5);

    assert_eq!(table.blocked_count, 0);
    assert_eq!(table.blocked_streams.len(), 0);
}

#[test]
fn no_insert_when_max_blocked_0() {
    let mut table = tracked_table(42);
    table.set_max_blocked(0).unwrap();

    assert_eq!(
        table.encoder(44).insert(&HeaderField::new("foo", "bar")),
        Ok(DynamicInsertionResult::NotInserted(DynamicLookupResult::NotFound))
    );
}

#[test]
fn no_insert_after_max_blocked_reached() {
    let mut table = tracked_table(42);
    table.set_max_blocked(2).unwrap();

    {
        let mut encoder = table.encoder(44);
        assert_eq!(
            encoder.insert(&HeaderField::new("foo", "bar")),
            Ok(DynamicInsertionResult::Inserted {
                postbase: 0,
                absolute: 4
            })
        );
        encoder.commit(4);
    }

    assert_eq!(table.blocked_count, 2);

    let mut encoder = table.encoder(46);
    assert_eq!(
        encoder.insert(&HeaderField::new("foo99", "bar")),
        Ok(DynamicInsertionResult::NotInserted(DynamicLookupResult::NotFound))
    );
}

#[test]
fn insert_again_after_encoder_ack() {
    let mut table = tracked_table(42);
    table.set_max_blocked(1).unwrap();

    assert_eq!(table.blocked_count, 1);

    {
        let mut encoder = table.encoder(44);
        assert_eq!(
            encoder.insert(&HeaderField::new("foo99", "bar")),
            Ok(DynamicInsertionResult::NotInserted(DynamicLookupResult::NotFound))
        );
        encoder.commit(0);
    }

    table.update_largest_received(3);
    assert_eq!(table.blocked_count, 0);

    let mut encoder = table.encoder(46);
    assert_eq!(
        encoder.insert(&HeaderField::new("foo", "bar")),
        Ok(DynamicInsertionResult::Inserted {
            postbase: 0,
            absolute: 4
        })
    );
}
