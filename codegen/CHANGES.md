# unreleased 0.4.0
## Change
- macro is refactored to target xitca-web `0.7.0`
- bump MSRV to `1.85` and Rust edition 2024

## Fix
- fix `xitca_web::WebContext` parsing when generic body type is presented.

# 0.3.1
## Fix
- fix route macro impl not implementing `xitca_web::handler::state::BorrowState` trait.

# 0.3.0
## Change
- macro is refactored to target xitca-web `0.6.0`

# 0.2.0
## Change
- macro is refactored to target xitca-web `0.4.0`

# 0.1.1
## Fix
- route macro is able to handle multiple `StateRef` and `StateOwn` arguments.
