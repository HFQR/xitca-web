# unreleased

# 0.4.0
- sync with matchit 0.9

# 0.3.0
## Change
- bump MSRV to `1.79`

## Fix
- allow catch all matching for route without leading slash. `foo/*` and `foo/*bar` become valid pattern.

# 0.2.0
## Add
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