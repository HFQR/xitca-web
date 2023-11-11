# HTTP client built on top of `xitca-http` and `tokio`

## Requirement
- rustc 1.75.0-nightly (c543b6f35 2023-10-14)

## Motivation
- 100% safe rust. All unsafe codes are outsourced to dependencies.
- Low memory footprint. Avoid pre-allocation whenever possible.

## Limitation
- This is a WIP project.
