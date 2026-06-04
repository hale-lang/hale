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

Both go through the **same LLVM-native compiler** — there's no
separate interpreter, so they never disagree:

- **`hale run prog.hl`** — compiles and runs in one step (the
  binary is temporary). The fast inner loop while you write.
- **`hale build prog.hl`** — compiles to a native ELF binary on
  disk via LLVM. This is the artifact you ship.

```sh
cargo run -p hale-cli --bin hale -- run  prog.hl   # compile + run
cargo run -p hale-cli --bin hale -- build prog.hl  # compile to ./prog
./prog
```

Throughout this guide we'll write `hale run` / `hale build` as
if `hale` is on your path. If it isn't, prefix with
`cargo run -p hale-cli --bin hale --`.

Next: [Your first run](./first-run.md).
