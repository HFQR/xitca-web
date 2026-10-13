use super::*;

/**
 * https://www.rfc-editor.org/rfc/rfc9204.html#name-static-table
 *  3.1.  Static Table
 *  [...]
 *  Note the QPACK static table is indexed from 0, whereas the HPACK
 *  static table is indexed from 1.
 */
#[test]
fn test_static_table_index_is_0_based() {
    assert_eq!(StaticTable::get(0), Ok(&HeaderField::new(":authority", "")));
}

#[test]
fn test_static_table_is_full() {
    assert_eq!(PREDEFINED_HEADERS.len(), 99);
}

#[test]
fn test_static_table_can_get_field() {
    assert_eq!(
        StaticTable::get(98),
        Ok(&HeaderField::new("x-frame-options", "sameorigin"))
    );
}

#[test]
fn invalid_index() {
    assert_eq!(StaticTable::get(99), Err(Error::Unknown(99)));
}

#[test]
fn find_by_name() {
    assert_eq!(StaticTable::find_name(b"last-modified"), Some(10usize));
    assert_eq!(StaticTable::find_name(b"does-not-exist"), None);
}

#[test]
fn find() {
    assert_eq!(StaticTable::find(&HeaderField::new(":method", "GET")), Some(17usize));
    assert_eq!(StaticTable::find(&HeaderField::new("foo", "bar")), None);
}
