# Install

> Get the `hale` toolchain on your path.

There are two ways to get `hale`: download a **prebuilt binary**
(quickest), or **build from source** (for contributors, or a
platform without a prebuilt). Either way, read [What you need to
run programs](#what-you-need-to-run-programs) — `hale` is a
compiler that shells out to a C toolchain, so it has a couple of
runtime requirements no matter how you install it.

## Quickest: prebuilt binary

Grab the tarball for your platform from the
[releases page](https://github.com/hale-lang/hale/releases):

| Platform | Asset |
|---|---|
| Linux x86_64 (glibc) | `hale-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon  | `hale-<version>-aarch64-apple-darwin.tar.gz` |

```sh
tar -xzf hale-<version>-<triple>.tar.gz
# The archive contains `hale` AND `libhale_ts_shim.a` — keep them
# in the SAME directory: the compiler looks for the shim next to
# its own binary and can't link programs without it.
sudo cp hale libhale_ts_shim.a /usr/local/bin/   # or anywhere on PATH, together
hale --help
```

The binary is **self-contained with respect to LLVM** — LLVM 18 is
statically linked in, so you do *not* need to install LLVM to run
the compiler. (Intel Macs: run the Apple-Silicon build under
Rosetta 2.)

## What you need to run programs

Regardless of how you installed `hale`, compiling a program
(`hale run` / `hale build`) recompiles and links the runtime on
your machine, so you need a C toolchain present:

- **`clang`** on your `PATH` (bare or `clang-18`) — used to
  assemble and link the emitted native code. `lld` is additionally
  needed only if you build with `LOTUS_LTO=1` or target `wasm32`.
- **OpenSSL** shared libraries (`libssl` / `libcrypto`) — the
  standard library's TLS client links against them unconditionally.

Installing `clang` pulls in `libLLVM` as *clang's own* dependency —
that's expected and harmless; `hale` itself doesn't need it.

## Build from source

Requirements:

- **Rust** 1.95 or newer (the compiler is written in Rust).
- **LLVM 18** development libraries, with `llvm-config-18` on your
  `PATH` (or `LLVM_SYS_180_PREFIX` pointing at the install). LLVM
  17, 19, and 20 will *not* link — the backend is pinned to 18.
- **clang** (+ **lld** for LTO / wasm), **OpenSSL** headers, and
  **git**.

**Debian / Ubuntu** (LLVM 18 is in stock apt on 24.04+):

```sh
sudo apt install llvm-18-dev libpolly-18-dev libzstd-dev \
                 clang-18 libclang-18-dev lld-18 zlib1g-dev \
                 libssl-dev pkg-config git
```

**Fedora**

```sh
sudo dnf install llvm18-devel clang18 lld openssl-devel git
```

**macOS (Homebrew)**

```sh
brew install llvm@18 openssl git
export LLVM_SYS_180_PREFIX="$(brew --prefix llvm@18)"
```

Then:

```sh
git clone https://github.com/hale-lang/hale
cd hale
cargo build --release
```

The `hale` binary lands at `target/release/hale` (and
`libhale_ts_shim.a` beside it). Put the binary on your path, or
invoke it through Cargo as shown below.

### Reproducible / release build

`release/docker-compose.yml` builds a self-contained Linux tarball
in a pinned `ubuntu:24.04` + LLVM 18 container, so you don't have
to match the toolchain locally:

```sh
docker compose -f release/docker-compose.yml run --rm build
# -> dist/hale-x86_64-unknown-linux-gnu.tar.gz
```

## Platform support

| Platform | Status |
|---|---|
| **Linux x86_64** (glibc) | First-class — hosts the compiler and runs compiled programs, all features. |
| **macOS** (Apple Silicon) | Supported — hosts the compiler and targets itself, with two carve-outs. **`async_io` pools** fail at compile time with a clear diagnostic (use a cooperative pool, or build on Linux). **Cross-process `unix(...)` bindings** use a framed byte-stream transport on macOS (Darwin has no `SOCK_SEQPACKET`) — same semantics, message boundaries preserved by a per-message header rather than the kernel; both ends of a socket must be Hale binaries on the same wire format (always true on one host). The prebuilt toolchain currently links Homebrew `llvm@18`'s libunwind and emitted binaries link Homebrew OpenSSL — machines without those Homebrew packages need them installed (`brew install llvm@18 openssl@3`); self-contained binaries are tracked upstream. Intel Macs run the arm64 build via Rosetta 2. |
| **Windows** | No native support (the runtime is POSIX). Use **WSL2** (Ubuntu) and follow the Linux instructions. |
| **wasm32** | `hale build --target wasm32` for the browser. |

## Verify

```sh
hale --help
```

Or through Cargo from a source checkout:

```sh
cargo run -p hale-cli --bin hale -- --help
```

To run the compiler's own test suite (single-threaded avoids "text
file busy" flakes from parallel test binaries racing on the same
temp path):

```sh
cargo test --release --workspace -- --test-threads=1
```

## The two ways to run a program

Both go through the **same LLVM-native compiler** — there's no
separate interpreter, so they never disagree:

- **`hale run prog.hl`** — compiles and runs in one step (the
  binary is temporary). The fast inner loop while you write.
- **`hale build prog.hl`** — compiles to a native binary on disk
  via LLVM. This is the artifact you ship.

```sh
hale run   prog.hl   # compile + run
hale build prog.hl   # compile to ./prog
./prog
```

Throughout this guide we write `hale run` / `hale build` as if
`hale` is on your path. From a source checkout without it
installed, prefix with `cargo run -p hale-cli --bin hale --`.

Next: [Your first run](./first-run.md).
