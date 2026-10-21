use super::*;

#[test]
fn local_resets_evict_oldest() {
    let mut resets = LocalResets::new(2);
    for id in [1, 3, 5] {
        resets.push(StreamId::from(id));
    }
    assert_eq!(resets.ids.len(), 2);
    assert!(!resets.contains(&StreamId::from(1)));
    assert!(resets.contains(&StreamId::from(3)));
    assert!(resets.contains(&StreamId::from(5)));

    // Zero capacity still remembers the most recent reset.
    let mut resets = LocalResets::new(0);
    for id in [1, 3] {
        resets.push(StreamId::from(id));
    }
    assert_eq!(resets.ids.len(), 1);
    assert!(!resets.contains(&StreamId::from(1)));
    assert!(resets.contains(&StreamId::from(3)));
}
