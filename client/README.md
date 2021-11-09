## HTTP client built on top of `xitca-http` and `tokio`

### Requirement:
- rust version 1.58.0-nightly (bd41e09da 2021-10-18)

### Motivation:
- 100% safe rust. All unsafe codes are outsourced to dependencies.
- Low memory footprint. Avoid pre-allocation whenever possible.

### Limitation:
- This is a WIP project.
