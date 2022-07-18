## HTTP client built on top of `xitca-http` and `tokio`

### Requirement:
- rustc 1.64.0-nightly (263edd43c 2022-07-17)

### Motivation:
- 100% safe rust. All unsafe codes are outsourced to dependencies.
- Low memory footprint. Avoid pre-allocation whenever possible.

### Limitation:
- This is a WIP project.
