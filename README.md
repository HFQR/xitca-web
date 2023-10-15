# An alternative http library and web framework inspired by hyper

## Requirement
- rustc 1.75.0-nightly (c543b6f35 2023-10-14)

## Motivation
- Prefer less synchronization when possible and thread per core design is used.
- 100% safe Rust.[^1]
- Low memory footprint. Avoid (pre)allocation when possible.
- Lightweight dependency tree. Avoid adding unnecessary import when possible. Prefer no proc macro code generation when possible(proc macro feature are still offered as opt-in instead of opt-out).
- Make code base compact and simple. Reduce the barrier of understanding of source code for easier contributing.
- Simplify ecosystem with no homebrew new type/crate wrapper. `tokio`(for async runtime) and `http`(for http types) are used directly as dependency.

## Limitation
- Experimental.
- No stable API.
- Feature in-complete.
- Test cover is poor.

[^1]: only guaranteed inside project's own code. unsafe can and is used by dependencies
