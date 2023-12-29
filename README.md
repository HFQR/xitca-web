# An alternative http library and web framework inspired by hyper

## Minimum Supported Rust Version
- 1.75 [^1]

## Motivation
- Less synchronization and thread per core design is used.
- 100% safe Rust.[^2]
- Low memory footprint. Avoid (pre)allocation when possible.
- Lightweight dependency tree. Avoid adding unnecessary import when possible. Prefer no proc macro code generation when possible(proc macro feature are still offered as opt-in instead of opt-out).
- Compact and simple code base. Reduce the barrier of understanding of source code for easier contributing.
- Good interop with other crates. `tokio`(for async runtime) and `http`(for http types) are used directly as dependency.

[^1]: `http-file` excluded due to design issue.
[^2]: only guaranteed inside project's own code. unsafe can and is used by dependencies
