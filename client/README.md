# HTTP client built on top of `xitca-http` and `tokio`

## Requirement

- rustc 1.66.0-nightly (5c8bff74b 2022-10-21)

## Motivation

- 100% safe rust. All unsafe codes are outsourced to dependencies.
- Low memory footprint. Avoid pre-allocation whenever possible.

## Limitation

- This is a WIP project.
