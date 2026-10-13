#[test]
fn test_unpack_octets_4() {
    let buf: [u8; 4] = [0, 0, 0, 1];
    assert_eq!(1u32, unpack_octets_4!(buf, 0, u32));
}
