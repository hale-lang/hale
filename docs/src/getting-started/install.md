# Install

You can install `hale` using a **prebuilt release archive** (fastest) or by **building from source**. Because `hale` compiles programs to native machine code and links the C runtime (`lotus`), every host also requires a standard C toolchain (`clang`, `openssl`, `zlib`) to build and run `.hl` programs.

---

## 1. Install Prebuilt Binary (Recommended)

Run the install script to download the latest release for your platform into `~/.hale/bin`:

```sh
curl -fsSL https://hale-lang.org/install.sh | sh
```

- Set `HALE_INSTALL=/custom/dir` to change the installation directory.
- Set `HALE_VERSION=v0.32.0` to pin a specific version.

### Manual Archive Installation

Download the tarball for your platform from the [GitHub Releases page](https://github.com/hale-lang/hale/releases):

| Platform | Release Asset |
|---|---|
| Linux x86_64 (glibc) | `hale-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 (glibc) | `hale-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon | `hale-<version>-aarch64-apple-darwin.tar.gz` |

Extract the archive and add the directory to your `PATH`:

```sh
mkdir -p ~/.hale/bin
tar -xzf hale-<version>-<triple>.tar.gz -C ~/.hale/bin
export PATH="$HOME/.hale/bin:$PATH"
hale --help
```

> [!IMPORTANT]
> Keep all extracted files (`hale`, `libhale_ts_shim.a`, and any bundled macOS dylibs) in the **same directory**. The compiler looks for `libhale_ts_shim.a` alongside its own executable when linking `std::ts` programs.

The prebuilt `hale` binary statically links LLVM 18; you do not need LLVM installed to run the prebuilt compiler. (Intel macOS does not have a prebuilt archive—use [Build from Source](#3-build-from-source) below.)

---

## 2. Host Dependencies for Compiling `.hl` Programs

When you run `hale build` or `hale run`, the compiler emits native object code, compiles the C runtime, and links against OpenSSL and zlib. Install these packages on your machine:

| OS | Required Host Packages | Install Command |
|---|---|---|
| **Ubuntu / Debian** | `clang`, `libssl-dev`, `zlib1g-dev` (`lld` optional for faster linking / LTO / WASM) | `sudo apt install clang lld libssl-dev zlib1g-dev` |
| **Fedora / RHEL** | `clang`, `lld`, `openssl-devel`, `zlib-devel` | `sudo dnf install clang lld openssl-devel zlib-devel` |
| **macOS (Homebrew)** | Xcode Command Line Tools (`clang`) + `openssl@3` | `brew install openssl@3` |

> **Debian/Ubuntu note:** Ensure the binary named `clang` is on your `PATH` (installing `clang-18` alone only provides `/usr/bin/clang-18`; install the `clang` wrapper package as well).

> **macOS note:** the developer tools `xcode-select -p` names must be at least as new as the macOS SDK clang links against. An older Xcode selected beside newer Command Line Tools cannot read their SDK, and every link fails with `tapi error: malformed file … unknown architecture`; `hale` says so and names the fix: `sudo xcode-select -s /Library/Developer/CommandLineTools`, or update Xcode.

---

## 3. Build from Source

Building the `hale` compiler from source requires:

- **Rust 1.95+**
- **LLVM 18 development libraries** (`llvm-config-18` on `PATH` or `LLVM_SYS_180_PREFIX` set; LLVM 17, 19, and 20 are not compatible)
- **clang**, **lld**, **OpenSSL** headers, **zlib** headers, and **git**

### Install Build Dependencies

**Debian / Ubuntu (24.04+):**

```sh
sudo apt install llvm-18-dev libpolly-18-dev libzstd-dev \
                 clang clang-18 libclang-18-dev lld-18 zlib1g-dev \
                 libssl-dev pkg-config git
```

**Fedora:**

```sh
sudo dnf install llvm18-devel clang18 lld openssl-devel zlib-devel git
```

**macOS (Homebrew):**

```sh
brew install llvm@18 openssl@3 git
export LLVM_SYS_180_PREFIX="$(brew --prefix llvm@18)"
```

### Compile the Workspace

```sh
git clone https://github.com/hale-lang/hale
cd hale
cargo build --release
```

The compiled binary is placed at `target/release/hale` (with `libhale_ts_shim.a` alongside it). Always build the whole workspace (`cargo build --release`) rather than `cargo build -p hale-cli` so that `libhale_ts_shim.a` is built.

### Containerized Release Build

To build a self-contained Linux `x86_64` tarball inside a pinned Ubuntu 24.04 + LLVM 18 container:

```sh
docker compose -f release/docker-compose.yml run --rm build
# Output: dist/hale-x86_64-unknown-linux-gnu.tar.gz
```

---

## 4. Platform & Target Matrix

Run `hale targets` (or `hale --list-targets`) to inspect all targets supported by your compiler binary.

| Target / Platform | Host Support | Target Codegen Support | Notes |
|---|---|---|---|
| **Linux x86_64 (`x86_64-unknown-linux-gnu`)** | Prebuilt & source | Native & cross-compiled | Full feature support (`epoll` for `async_io`, `SOCK_SEQPACKET` for `unix` bus bindings). |
| **Linux ARM64 (`aarch64-unknown-linux-gnu`)** | Prebuilt & source | Native & cross-compiled | Full feature support (AWS Graviton, Ampere, ARM servers). |
| **Linux musl (`x86_64` / `aarch64-unknown-linux-musl`)** | — | Cross-compiled static binary | Produces a zero-dependency static ELF binary. Note: `where async_io` pools are rejected on musl at check time because musl lacks `ucontext`. |
| **macOS Apple Silicon (`aarch64-apple-darwin`)** | Prebuilt & source | Native on macOS | Uses `kqueue` for `async_io` and length-framed stream sockets for `unix(...)` bindings. Emitted binaries statically link OpenSSL. |
| **macOS Intel (`x86_64-apple-darwin`)** | Build from source | Native on macOS | Build compiler from source with Homebrew `llvm@18`. |
| **WebAssembly (`wasm32`)** | — | `hale build --target wasm32` | Emits `.wasm` + `.mjs` loader from any host (requires `wasm-ld` / `lld`). |
| **Windows (`*-pc-windows-msvc`)** | Use WSL2 (Ubuntu) | Not yet supported ([GH #445](https://github.com/hale-lang/hale/issues/445)) | Refused at check/build time with an explicit target error. |

---

## 5. Cross-Compiling to Linux (from macOS or Linux)

To compile Linux binaries (`gnu` or static `musl`) from macOS or another architecture, Hale uses **Zig** as a cross-C compiler/linker plus a **target sysroot** containing static OpenSSL, zlib, and `libhale_ts_shim.a`:

1. **Install Zig:** `brew install zig` (or download from [ziglang.org](https://ziglang.org/)). Set `HALE_ZIG` if `zig` is not on `PATH`.
2. **Build the target sysroot once:** Run `scripts/target-sysroot.sh <triple>` (requires `zig`, `curl`, `perl`, and `make`). This populates `~/.cache/hale/sysroot/<triple>/` (override with `HALE_TARGET_SYSROOT`; set `HALE_TARGET_GLIBC` to customize the glibc version floor, default `2.31`).

```sh
# Build sysroot once per target triple:
scripts/target-sysroot.sh x86_64-unknown-linux-gnu
scripts/target-sysroot.sh x86_64-unknown-linux-musl

# Cross-compile for glibc >= 2.31:
hale build --target x86_64-unknown-linux-gnu app.hl

# Cross-compile a fully static binary for Alpine / scratch containers:
hale build --target x86_64-unknown-linux-musl app.hl
```

---

## 6. Verifying Your Installation

```sh
hale --help
```

If you built from a source checkout and want to run the compiler's test suite:

```sh
# On macOS, add Homebrew's lib path so test binaries find libzstd:
export LIBRARY_PATH="$(brew --prefix)/lib"

cargo nextest run --release --workspace
```

Next: **[Quickstart & Toolchain](./first-run.md)**.
