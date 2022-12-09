# A fork of [matchit](https://github.com/ibraheemdev/matchit)

## Compare to matchit
- Pros
  - practical performance improvement. (less memory copy when transforming params to owned value)
  - TODO: clean public types with no lifetime pollution. (easier to pass params around) 
  - 100% safe Rust. (unsafe code still used through dependencies)
- Cons
  - more public types.
  - immutable router value.
  - potentially slower in micro benchmark.
  - TODO: Param value only store indices referencing original input matching path. mutation of said path can result in 
    unexpected param value corruption.
