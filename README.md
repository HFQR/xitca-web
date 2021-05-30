## An alternative http library inspired by actix-web.

### Requirement:
- rust version 1.54.0-nightly

### Motivation:
- 100% safe rust. All unsafe code are outsourced to dependencies.
- Low memory footprint. Avoid pre-allocation whenever possible.
- Experiment nightly rust feature to make async web frameworks easier to use.
- Make code base compat and simple. Reduce the barrier of understanding of source code for easier contributing.

### Limitation:
- Highly experimental.
- Http/1 feature is unfinished.
- Http/3 feature is untested.
