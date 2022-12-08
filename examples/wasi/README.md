# a http/1 server example targeting wasm32-wasi

## Requirement
- [wasmtime](https://docs.wasmtime.dev/)

## Setup
- install wasmtime cli. <https://docs.wasmtime.dev/cli-install.html>
- compile project.
    - unix
      ```bash
      RUSTFLAGS="--cfg tokio_unstable" cargo build --release --target wasm32-wasi
      ```
    - windows
      ```commandline
      set RUSTFLAGS=--cfg tokio_unstable
      cargo build --release --target wasm32-wasi
      ```
- (optional) optimize compiled wasm file. <https://github.com/WebAssembly/binaryen>
- run compiled wasm file.
  ```commandline
  wasmtime run ../target/wasm32-wasi/release/xitca-web-wasi.wasm --tcplisten 127.0.0.1:8080 --env FD_COUNT=3
  ```
- open browser and visit <http://127.0.0.1:8080>
