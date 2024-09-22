# async postgresql client deeply integrated with [xitca-web](https://github.com/HFQR/xitca-web). Inspired and depend on [rust-postgres](https://github.com/sfackler/rust-postgres)

## Compare to tokio-postgres
- Pros
    - async/await native
    - less heap allocation on query
    - zero copy row data parsing
    - quic transport layer for lossy database connection
- Cons
    - no built in back pressure mechanism. possible to cause excessive memory usage if database requests are unbounded or not rate limited
    - expose lifetime in public type params.(hard to return from function or contained in new types)
