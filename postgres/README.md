# A WIP postgresql client deeply integrated with xitca-web. Inspired and depend on [rust-postgres](https://github.com/sfackler/rust-postgres)

## Compare to tokio-postgres
- Pros
    - async/await native.
    - less heap allocation on query. 
    - zero copy row data parsing.
    - quic transport layer for remote lossy database connection.
- Cons
    - feature absence. no transaction portal and savepoint, no query canceling, etc.(being worked on)
    - depend on other xitca-xxx crates.
    - expose liftime in public type params.(harder to return from function or contained in new types)
