# Install

> Get the `hale` toolchain built and on your path.

Hale builds from source. You need:

- **Rust** 1.95 or newer (the compiler is written in Rust).
- **LLVM 18** development libraries, with `llvm-config-18` on
  your `PATH` (or `LLVM_SYS_180_PREFIX` pointing at the install).
  LLVM 17, 19, and 20 will *not* link — the codegen backend is
  pinned to 18.
- **clang** (used to assemble and link emitted native code).
- **git** (for fetching library dependencies).
- **OpenSSL** headers (`libssl` + `libcrypto`), for the TLS
  client in the standard library.

## Platform setup

**Debian / Ubuntu**

```sh
sudo apt install llvm-18-dev libclang-18-dev clang-18 \
                 libssl-dev pkg-config git
```

**macOS (Homebrew)**

```sh
brew install llvm@18 openssl git
export LLVM_SYS_180_PREFIX="$(brew --prefix llvm@18)"
```

**Fedora**

```sh
sudo dnf install llvm18-devel clang18 openssl-devel git
```

## Build

```sh
git clone https://github.com/hale-lang/hale
cd hale
cargo build --release
```

The `hale` binary lands at `target/release/hale`. Put it on your
path, or invoke it through Cargo as shown below.

## Verify

```sh
cargo run -p hale-cli --bin hale -- --help
```

To run the test suite (single-threaded avoids "text file busy"
flakes from parallel test binaries racing on the same temp
path):

```sh
cargo test --release --workspace -- --test-threads=1
```

## The two ways to run a program

Hale ships two execution paths, and you'll use both:

- **Interpreter** — `hale run prog.hl`. A tree-walking
  interpreter for fast feedback while you write.
- **Native** — `hale build prog.hl` produces a native ELF binary
  via LLVM. This is what you ship.

```sh
cargo run -p hale-cli --bin hale -- run  prog.hl   # interpret
cargo run -p hale-cli --bin hale -- build prog.hl  # compile
./prog
```

Throughout this guide we'll write `hale run` / `hale build` as
if `hale` is on your path. If it isn't, prefix with
`cargo run -p hale-cli --bin hale --`.

Next: [Your first run](./first-run.md).
