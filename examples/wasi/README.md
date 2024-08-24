# a http/1 server example targeting wasm32-wasi

## Requirement
- [wasmtime](https://docs.wasmtime.dev/)

## Setup
- add target platform to Rust compiler
  ```commandline
  rustup target add wasm32-wasip1-threads
  ```
- install wasmtime cli (version > 13.0). <https://docs.wasmtime.dev/cli-install.html>
- compile project.
    - unix
      ```bash
      RUSTFLAGS="--cfg tokio_unstable" cargo build --release --target wasm32-wasip1-threads
      ```
    - windows
      ```commandline
      set RUSTFLAGS=--cfg tokio_unstable
      cargo build --release --target wasm32-wasip1-threads
      ```
- (optional) optimize compiled wasm file. <https://github.com/WebAssembly/binaryen>
- run compiled wasm file.
  ```commandline
  wasmtime run -W threads=y -S threads=y,preview2=n -S tcplisten=127.0.0.1:8080 --env FD_COUNT=3 ../target/wasm32-wasip1-threads/release/xitca-web-wasi.wasm
  ```
- open browser and visit <http://127.0.0.1:8080>
