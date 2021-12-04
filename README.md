## An alternative http library inspired by actix-web.

### Requirement:
- Rust version 1.59.0-nightly (532d2b14c 2021-12-03)

### Motivation:
- 100% safe Rust. All unsafe codes are outsourced to dependencies.
- Low memory footprint. Avoid pre-allocation whenever possible.
- Experiment nightly Rust feature to make async web frameworks easier to use.
- Make code base compact and simple. Reduce the barrier of understanding of source code for easier contributing.
- Simplify ecosystem with no homebrew runtime wrapper. `tokio` is used directly as dependency.

### Limitation:
- Highly experimental.
- Feature in-complete.
- Test cover is poor.
