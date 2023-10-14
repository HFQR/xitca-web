# An alternative http library and web framework inspired by hyper

## Requirement
- rustc 1.75.0-nightly (fcab24817 2023-10-13)

## Motivation
- Prefer less synchronization when possible and thread per core design is used.
- 100% safe Rust.[^1]
- Low memory footprint. Avoid (pre)allocation when possible.
- Lightweight dependency tree. Avoid adding unnecessary import when possible. Prefer no proc macro code generation when possible(proc macro feature are still offered as opt-in instead of opt-out).
- Make async web frameworks easier to use. nightly Rust feature: [async_fn_in_trait](https://github.com/rust-lang/rust/pull/115822) and [return_position_impl_trait_in_trait](https://github.com/rust-lang/rust/pull/115822) are used.
- Make code base compact and simple. Reduce the barrier of understanding of source code for easier contributing.
- Simplify ecosystem with no homebrew new type/crate wrapper. `tokio`(for async runtime) and `http`(for http types) are used directly as dependency.

## Limitation
- Experimental.
- No stable API.
- Feature in-complete.
- Test cover is poor.

[^1]: only guaranteed inside project's own code. unsafe can and is used by dependencies
