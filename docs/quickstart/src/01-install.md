# Install

Aperio is not yet published. Build the compiler from source.

## Prerequisites

- Rust toolchain (1.75+). [rustup.rs](https://rustup.rs).
- LLVM 18 development headers. On Debian/Ubuntu: `apt install
  llvm-18-dev libpolly-18-dev`. On macOS with Homebrew: `brew install
  llvm@18`.
- A C compiler (used to compile the bundled lotus runtime arena).
  `cc` from your platform toolchain is fine.

## Build

```bash
git clone <repo-url> aperio
cd aperio
cargo build --release
```

The compiler binary lands at `target/release/aperio`. Either run it
from there directly or symlink it onto your `PATH`:

```bash
ln -s "$PWD/target/release/aperio" ~/.local/bin/aperio
```

## Verify

```bash
aperio --help
```

You should see the subcommand list (`lex`, `parse`, `check`, `run`,
`build`).
