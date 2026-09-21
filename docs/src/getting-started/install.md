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
| Linux ARM64 (glibc)  | `hale-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon  | `hale-<version>-aarch64-apple-darwin.tar.gz` |

The installer picks the right one for you:

```sh
curl -fsSL https://hale-lang.org/install.sh | sh
```

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

Three different questions hide behind "does Hale support X", and they
have different answers. Keeping them apart:

**1. Where a prebuilt compiler exists** — Linux x86_64, Linux ARM64,
and macOS on Apple Silicon, per the table above.

**2. Where the compiler can be built from source** — the three above,
plus Intel macOS. Needs LLVM 18 dev libraries and `clang`; see
[building from source](#building-from-source).

**3. What a build can emit** — `hale --list-targets` is the
authority. Native binaries for the platform `hale` itself runs on — a
Linux `hale` builds Linux programs, a macOS `hale` builds macOS ones;
`wasm32` objects for the browser from either. A **Linux** triple from
any other host — `--target x86_64-unknown-linux-gnu` or
`aarch64-unknown-linux-gnu` on a Mac, or the other architecture on
Linux — is cross-compiled and linked here; see
[Cross-compiling for Linux](#cross-compiling-for-linux) below. A macOS
triple from anywhere else gets as far as a relocatable object for that
platform (`app.o`) and stops with a note — there is no Apple SDK to
link against off a Mac. `x86_64-pc-windows-msvc` is named and refused
with a precise error, because Windows codegen does not exist yet
([GH #445](https://github.com/hale-lang/hale/issues/445)).

### Cross-compiling for Linux

The everyday case: develop on a Mac, deploy to Linux servers. A Hale
program always links the lotus C runtime, OpenSSL, zlib and (for
`std::ts`) a tree-sitter staticlib, so unlike a pure-Go binary it needs
a C toolchain and those libraries *for the target*. Two pieces supply
them ([GH #970](https://github.com/hale-lang/hale/issues/970)):

1. **zig**, as the C compiler and linker. `zig cc -target
   x86_64-linux-gnu.2.31` carries its own glibc headers and stubs for
   every Linux target, so nothing has to be installed for the target's
   libc. `brew install zig` (or a release from ziglang.org); `HALE_ZIG`
   names the binary if it is not on `PATH`.
2. **A target sysroot**: OpenSSL and zlib for the target, as static
   archives, plus the tree-sitter shim. `scripts/target-sysroot.sh
   <triple>` builds one into `~/.cache/hale/sysroot/<triple>/` — both
   libraries from pinned source tarballs, compiled with zig against the
   same glibc floor (a minute or two, once per target) — and cross-builds
   `libhale_ts_shim.a` when run from a checkout with `rustup target add
   <triple>` done. Needs `curl`, `perl` and `make` besides zig.
   `HALE_TARGET_SYSROOT` points at one kept elsewhere.

Then:

```sh
scripts/target-sysroot.sh x86_64-unknown-linux-gnu     # once
hale build --target x86_64-unknown-linux-gnu app.hl    # ELF x86-64, runs on any glibc ≥ 2.31
```

The emitted binary depends on the target's glibc and nothing else —
OpenSSL and zlib are linked in. `hale run` and `hale test` refuse a
foreign target (nothing it builds runs here); `LOTUS_ASAN` and the
other sanitizers are host-only. Without zig or the sysroot the build
fails before linking and says which one is missing. `HALE_TARGET_GLIBC`
picks a different glibc floor (the script and the compiler read the
same variable).

The rest of this section is about the *host* — where `hale` itself
runs, and what changes about a program compiled there.

| Platform | Status |
|---|---|
| **Linux x86_64** (glibc) | First-class — hosts the compiler and runs compiled programs, all features. |
| **Linux ARM64** (glibc) | Supported, prebuilt — the release matrix builds it on a native aarch64 runner (AWS Graviton, EKS arm64 nodes, Ampere). Same feature set as x86_64. |
| **macOS** (Apple Silicon) | Supported — hosts the compiler and targets itself, with two carve-outs. **`async_io` pools** fail at compile time with a clear diagnostic when the build *targets* macOS (use a cooperative pool, or build for Linux — a Mac building `--target x86_64-unknown-linux-gnu` may place one). **Cross-process `unix(...)` bindings** use a framed byte-stream transport on macOS (Darwin has no `SOCK_SEQPACKET`) — same semantics, message boundaries preserved by a per-message header rather than the kernel; both ends of a socket must be Hale binaries on the same wire format (always true on one host). The prebuilt toolchain currently links Homebrew `llvm@18`'s libunwind and emitted binaries link Homebrew OpenSSL — machines without those Homebrew packages need them installed (`brew install llvm@18 openssl@3`); self-contained binaries are tracked upstream. Intel Macs run the arm64 build via Rosetta 2. |
| **Windows** | No native support yet — the runtime is POSIX. Use **WSL2** (Ubuntu) and follow the Linux instructions. The compiler now *names* `x86_64-pc-windows-msvc` (`hale --list-targets`) and refuses it with a precise error rather than a link failure; the codegen and runtime work is tracked in [GH #445](https://github.com/hale-lang/hale/issues/445). |
| **wasm32** | `hale build --target wasm32` for the browser. |

## Verify

```sh
hale --help
```

Or from a source checkout:

```sh
cargo build --release          # the whole workspace
./target/release/hale --help
```

To run the compiler's own test suite (single-threaded avoids "text
file busy" flakes from parallel test binaries racing on the same
temp path):

```sh
cargo nextest run --release --workspace
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
installed, build the workspace once with `cargo build --release`
and use `./target/release/hale`. Build the *whole* workspace:
`cargo build -p hale-cli` skips the tree-sitter staticlib that
`std::ts` links against, and a `hale` built that way refuses
`std::ts` programs with a build error saying so.

Starting a project rather than a scratch file? `hale init my-app`
scaffolds the canonical minimal shape — `hale.toml`, a hello-world
seed, a first test — ready for `hale run` / `hale test` / `hale
verify` out of the box.

Next: [Your first run](./first-run.md).
