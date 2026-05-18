# Install

Aperio currently builds from source. You'll need:

- A Rust toolchain (stable or newer; tested on 1.95+).
- **LLVM 18** development libraries, with `llvm-config-18`
  on `PATH` (or `LLVM_SYS_180_PREFIX` pointing at an LLVM 18
  install). The compiler links against LLVM via
  [`inkwell`](https://github.com/TheDan64/inkwell) with the
  `llvm18-0` feature; LLVM 17 / 19 / 20 will *not* work.
- **`clang`** on `PATH`. The compiler invokes it as the linker
  when producing native binaries (`aperio build`).
- **`git`** on `PATH`. Used by `aperio fetch` to clone declared
  dependencies.

## Installing the host dependencies

### Debian / Ubuntu

```sh
sudo apt install llvm-18-dev libclang-18-dev clang-18 git
# Some apt layouts don't add `llvm-config-18` to PATH by default:
sudo ln -sf /usr/bin/llvm-config-18 /usr/local/bin/llvm-config
```

If `apt` doesn't have an `llvm-18-dev` package for your release,
add the official LLVM apt source (`https://apt.llvm.org/`)
following the instructions there for your distro.

### macOS (Homebrew)

```sh
brew install llvm@18 git
# Tell the build where LLVM 18 lives — Homebrew doesn't link
# llvm@18 into PATH by default to avoid colliding with system clang.
export LLVM_SYS_180_PREFIX="$(brew --prefix llvm@18)"
export PATH="$(brew --prefix llvm@18)/bin:$PATH"
```

Add the `export` lines to your shell rc file if you want them
to persist.

### Fedora / RHEL

```sh
sudo dnf install llvm18-devel clang18 git
```

### Verifying

```sh
llvm-config --version    # should print 18.x.x
clang --version          # should be present
```

## Build the compiler

```sh
git clone https://github.com/aperio-lang/aperio
cd aperio
cargo build --release
```

The `aperio` binary lands at `target/release/aperio`. You can
either symlink it onto your `PATH` or always invoke it via cargo:

```sh
cargo run -p aperio-cli --bin aperio -- run hello.ap
```

## Run the test suite

```sh
cargo test --release --workspace -- --test-threads=1
```

The `--test-threads=1` flag is load-bearing — parallel test
binaries can race each other on the same temp paths, surfacing
flaky "text file busy" failures. Run tests serially.

The test suite is the source of truth for what the compiler
supports today. If a test fails on a clean checkout, that's a
bug — please file an issue.

## Project layout (when you start your own)

A project is a directory with one or more `.ap` files. There's
no `src/`, no build directory, no package metadata beyond an
optional `aperio.toml`. The directory *is* the project.

See [Project layout & build commands](../how-tos/project-layout.md)
for the full treatment — single-file vs seed vs cross-seed
imports, what `aperio run` / `build` / `fetch` / `test` each
do, and the `aperio.toml` + `vendor/` shape.
