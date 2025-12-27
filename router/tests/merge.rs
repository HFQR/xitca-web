use xitca_router::{InsertError, Router};

#[test]
fn merge_ok() {
    let mut root = Router::new();
    assert!(root.insert("/foo", "foo").is_ok());
    assert!(root.insert("/bar/{id}", "bar").is_ok());

    let mut child = Router::new();
    assert!(child.insert("/baz", "baz").is_ok());
    assert!(child.insert("/xyz/{id}", "xyz").is_ok());

    assert!(root.merge(child).is_ok());

    assert_eq!(root.at("/foo").map(|m| *m.value), Ok("foo"));
    assert_eq!(root.at("/bar/1").map(|m| *m.value), Ok("bar"));
    assert_eq!(root.at("/baz").map(|m| *m.value), Ok("baz"));
    assert_eq!(root.at("/xyz/2").map(|m| *m.value), Ok("xyz"));
}

#[test]
fn merge_conflict() {
    let mut root = Router::new();
    assert!(root.insert("/foo", "foo").is_ok());
    assert!(root.insert("/bar", "bar").is_ok());

    let mut child = Router::new();
    assert!(child.insert("/foo", "changed").is_ok());
    assert!(child.insert("/bar", "changed").is_ok());
    assert!(child.insert("/baz", "baz").is_ok());

    let errors = root.merge(child).unwrap_err();

    assert_eq!(errors.first(), Some(&InsertError::Conflict { with: "/foo".into() }));

    assert_eq!(errors.get(1), Some(&InsertError::Conflict { with: "/bar".into() }));

    assert_eq!(root.at("/foo").map(|m| *m.value), Ok("foo"));
    assert_eq!(root.at("/bar").map(|m| *m.value), Ok("bar"));
    assert_eq!(root.at("/baz").map(|m| *m.value), Ok("baz"));
}

#[test]
fn merge_nested() {
    let mut root = Router::new();
    assert!(root.insert("/foo", "foo").is_ok());

    let mut child = Router::new();
    assert!(child.insert("/foo/bar", "bar").is_ok());

    assert!(root.merge(child).is_ok());

    assert_eq!(root.at("/foo").map(|m| *m.value), Ok("foo"));
    assert_eq!(root.at("/foo/bar").map(|m| *m.value), Ok("bar"));
}
