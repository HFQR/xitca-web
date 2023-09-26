# An alternative http library and web framework inspired by hyper

## Requirement
- rustc 1.74.0-nightly (0288f2e19 2023-09-25)

## Motivation
- Prefer less synchronization when possible and thread per core design is used.
- 100% safe Rust.[^1]
- Low memory footprint. Avoid (pre)allocation when possible.
- Lightweight dependency tree. Avoid adding unnecessary import when possible. Prefer no proc macro code generation when possible(proc macro feature are still offered as opt-in instead of opt-out).
- Make async web frameworks easier to use. Experimental nightly Rust feature: [impl_trait_in_assoc_type](https://github.com/rust-lang/rust/issues/63063) is used.
- Make code base compact and simple. Reduce the barrier of understanding of source code for easier contributing.
- Simplify ecosystem with no homebrew new type/crate wrapper. `tokio`(for async runtime) and `http`(for http types) are used directly as dependency.

## Limitation
- Experimental.
- No stable API.
- Feature in-complete.
- Test cover is poor.

[^1]: only guaranteed inside project's own code. unsafe can and is used by dependencies
