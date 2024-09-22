# unreleased 0.1.1
## Fix
- remove `Clone` trait impl from `Statement`. this is a bug where `Statement` type is not meant to be duplicateable by library user
