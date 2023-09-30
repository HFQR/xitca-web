# A fork of [matchit](https://github.com/ibraheemdev/matchit)

## Compare to matchit
- Pros
  - clean public types with no lifetime pollution. (easier to pass params around) 
  - 100% safe Rust. (unsafe code still used through dependencies)
- Cons
  - immutable router value.
  - potentially slower in micro benchmark.
