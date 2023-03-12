# A WIP postgresql client deeply integrated with xitca-web. Inspired and depend on [rust-postgres](https://github.com/sfackler/rust-postgres)

## Compare to tokio-postgres
- Pros
    - async/await native.
    - less heap allocation on query. 
    - zero copy row data parsing.
- Cons
    - feature absence. no transaction, no query canceling, etc.(being worked on)
    - require nightly rust. (the project's goal is to compile on stable eventually)
    - depend on other xitca-xxx crates.
    - expose liftime in public to type params.(harder to return from function or contained in new types)