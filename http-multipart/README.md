# an async http multipart crate.

## Features
- common http types and streaming interface for easy integration.
- native async/await support focus on stack pinned streaming type.
- in place streaming parsing first with reduced memory copy and reduced additional allocation.

## Requirement
- Rust 1.75
- [http](https://crates.io/crates/http) and [futures](https://crates.io/crates/futures) for http types and async streaming interaction[^1]

[^1]: see project `Cargo.toml` for dependency versioning.
