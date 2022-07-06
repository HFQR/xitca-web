## An alternative http library and web framework inspired by hyper and actix-web.

### Requirement:
- rustc 1.63.0-nightly (ca122c7eb 2022-06-13)

### Motivation:
- thread per core. Prefer less synchronization when possible.
- 100% safe Rust. All unsafe codes are outsourced to dependencies.
- Low memory footprint. Avoid (pre)allocation when possible. 
- Light weight dependency tree. Avoid adding unnecessary import when possible. Prefer no proc macro code generation when possible(proc macro feature are still offered as opt-in instead of opt-out).
- Experiment nightly Rust features: [generic_associated_types](https://github.com/rust-lang/rust/issues/44265) and [type_alias_impl_trait](https://github.com/rust-lang/rust/issues/63063) to make async web frameworks easier to use.
- Make code base compact and simple. Reduce the barrier of understanding of source code for easier contributing.
- Simplify ecosystem with no homebrew new type/crate wrapper. `tokio`(for async runtime) and `http`(for http types) are used directly as dependency.

### Limitation:
- Experimental.
- No stable API.
- Feature in-complete.
- Test cover is poor.
