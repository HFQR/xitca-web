unreleased version 0.2

# Add
- add catch all matching for `/*`. It will match on `/*p` and `/`. This enables nesting `Router` types and other similar use case where the path matching is break up into separate segments between multiple scopes. 

    Example:
   ```rust
    // a scoped router match against relative path of / and /login
    let mut router_scope = xitca_router::Router::new();
    router_scope.insert("/", true);
    router_scope.insert("/login", true);

    // add scoped router to root router where /users is used as
    // prefix
    let mut router = xitca_router::Router::new();
    router.insert("/users/*", router_scope);

    // handling /users as prefix and / and /login as suffix
    assert!(m.at("/users/").is_ok());
    assert!(m.at("/users/login").is_ok());
   ```