## An alternative http library inspired by actix-web.

### Requirement:
- rustc 1.56.0-nightly (b70888601 2021-07-28)

### Motivation:
- 100% safe rust. All unsafe codes are outsourced to dependencies.
- Low memory footprint. Avoid pre-allocation whenever possible.
- Experiment nightly rust feature to make async web frameworks easier to use.
- Make code base compact and simple. Reduce the barrier of understanding of source code for easier contributing.
- Simplify ecosystem with no homebrew runtime wrapper. `tokio` is used directly as dependency.

### Limitation:
- Highly experimental.
- Http/1 feature is unfinished.
- Http/3 feature is untested.
