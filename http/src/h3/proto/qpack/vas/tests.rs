use super::*;
use proptest::proptest;

#[test]
fn test_no_relative_index_when_empty() {
    let vas = VirtualAddressSpace::default();
    let res = vas.relative_base(0, 0);
    assert_eq!(res, Err(Error::RelativeIndex(0)));
}

#[test]
fn test_relative_underflow_protected() {
    let mut vas = VirtualAddressSpace::default();
    vas.add();
    assert_eq!(vas.relative(2), Err(Error::RelativeIndex(2)));
}

proptest! {
    #[test]
    fn test_first_insertion_without_drop(
        ref count in 1..2200usize
    ) {
        let mut vas = VirtualAddressSpace::default();
        vas.add();
        (1..*count).for_each(|_| { vas.add(); });

        assert_eq!(vas.relative_base(*count, count - 1), Ok(0), "{:?}", vas);
    }

    #[test]
    fn test_first_insertion_with_drop(
        ref count in 2..2200usize
    ) {
        let mut vas = VirtualAddressSpace::default();
        vas.add();
        (1..*count).for_each(|_| { vas.add(); });
        (0..*count - 1).for_each(|_| vas.drop());

        assert_eq!(vas.relative_base(*count, count - 1), Err(Error::RelativeIndex(count - 1)), "{:?}", vas);
    }

    #[test]
    fn test_last_insertion_without_drop(
        ref count in 1..2200usize
    ) {
        let mut vas = VirtualAddressSpace::default();
        (1..*count).for_each(|_| { vas.add(); });
        vas.add();

        assert_eq!(vas.relative_base(*count, 0), Ok(count -1),
                   "{:?}", vas);
    }

    #[test]
    fn test_last_insertion_with_drop(
        ref count in 2..2200usize
    ) {
        let mut vas = VirtualAddressSpace::default();
        (0..*count - 1).for_each(|_| { vas.add(); });
        vas.add();
        (0..*count - 1).for_each(|_| { vas.drop(); });

        assert_eq!(vas.relative_base(*count, 0), Ok(0),
                   "{:?}", vas);
    }
}

#[test]
fn test_post_base_index() {
    /*
     * Base index: D
     * Target value: B
     *
     * VAS: ]GFEDCBA]
     * abs:  1234567
     * rel:  3210---
     * pst:  ----012
     * pos:  0123456
     */
    let mut vas = VirtualAddressSpace::default();
    (0..7).for_each(|_| {
        vas.add();
    });

    assert_eq!(vas.post_base(4, 1), Ok(5));
}

#[test]
fn largest_ref() {
    let mut vas = VirtualAddressSpace::default();
    (0..7).for_each(|_| {
        vas.add();
    });
    assert_eq!(vas.largest_ref(), 7);
}

#[test]
fn relative() {
    let mut vas = VirtualAddressSpace::default();

    (0..7).for_each(|_| {
        vas.add();
    });

    assert_eq!(vas.relative(0), Ok(6));
    assert_eq!(vas.relative(1), Ok(5));
    assert_eq!(vas.relative(6), Ok(0));
    assert_eq!(vas.relative(7), Err(Error::RelativeIndex(7)));
}

#[test]
fn absolute_from_real_index() {
    let mut vas = VirtualAddressSpace::default();
    assert_eq!(vas.index(0), Err(Error::Index(0)));
    vas.add();
    assert_eq!(vas.index(0), Ok(1));
    vas.add();
    vas.drop();
    assert_eq!(vas.index(0), Ok(2));
    vas.drop();
    assert_eq!(vas.index(0), Err(Error::Index(0)));
    vas.add();
    vas.add();
    assert_eq!(vas.index(0), Ok(3));
    assert_eq!(vas.index(1), Ok(4));
    assert_eq!(vas.index(2), Err(Error::Index(2)));
}

#[test]
fn evicted() {
    let mut vas = VirtualAddressSpace::default();
    assert!(!vas.evicted(0));
    assert!(!vas.evicted(1));
    vas.add();
    vas.add();
    assert!(!vas.evicted(1));
    vas.drop();
    assert!(!vas.evicted(0));
    assert!(vas.evicted(1));
    assert!(!vas.evicted(2));
    vas.drop();
    assert!(vas.evicted(2));
}
