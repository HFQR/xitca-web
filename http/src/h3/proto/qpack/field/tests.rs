use super::*;

/**
 * https://tools.ietf.org/html/rfc7541#section-4.1
 * "The size of an entry is the sum of its name's length in octets (as
 *  defined in Section 5.2), its value's length in octets, and 32."
 * "The size of an entry is calculated using the length of its name and
 *  value without any Huffman encoding applied."
 */
#[test]
fn test_field_size_is_offset_by_32() {
    let field = HeaderField {
        name: Cow::Borrowed(b"Name"),
        value: Cow::Borrowed(b"Value"),
    };
    assert_eq!(field.mem_size(), 4 + 5 + 32);
}

#[test]
fn with_value() {
    let field = HeaderField {
        name: Cow::Borrowed(b"Name"),
        value: Cow::Borrowed(b"Value"),
    };
    assert_eq!(
        field.with_value("New value"),
        HeaderField {
            name: Cow::Borrowed(b"Name"),
            value: Cow::Borrowed(b"New value"),
        }
    );
}
