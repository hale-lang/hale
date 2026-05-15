# Install

Aperio currently builds from source. You'll need a Rust toolchain
(stable or newer; tested on 1.95+) and a system LLVM 18.

## Build the compiler

```sh
git clone <this repo>
cd lotus-lang
cargo build --release
```

The `aperio` binary lands at `target/release/aperio`. You can
either symlink it onto your `PATH` or always invoke it via cargo:

```sh
cargo run -p aperio-cli --bin aperio -- run hello.ap
```

## Run the test suite

```sh
cargo test --release --workspace
```

The test suite is the source of truth for what the compiler
supports today. If a test fails on a clean checkout, that's a
bug — please file an issue.
