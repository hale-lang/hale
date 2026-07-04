# Claude Code entry point

The canonical agent prompt for this repo is [`AGENTS.md`](./AGENTS.md).
Read it first.

`AGENTS.md` targets agents writing `.hl` programs. If you're
working on the language itself (compiler / runtime / spec), the
older role-organized briefs under [`agents/`](./agents/) still
apply:

- [`agents/library-dev.md`](./agents/library-dev.md) — adding to
  the stdlib or writing an Hale library.
- [`agents/compiler-dev.md`](./agents/compiler-dev.md) — working
  on the compiler / runtime / spec.

## Build + test (compiler work only)

```sh
cargo build --release
cargo test --release --workspace -- --test-threads=1
```

The serial flag avoids "text file busy" flakes from parallel
test binaries racing each other on the same temp path. Keep it
on single-crate runs too:

```sh
# one integration test in hale-codegen
cargo test --release -p hale-codegen --test topic_phase2 -- --test-threads=1
```

Codegen requires **LLVM 18** dev libs with `llvm-config-18` on
PATH (or `LLVM_SYS_180_PREFIX` set); `inkwell` is pinned to
`llvm18-0`. LLVM 17 / 19 / 20 will not link.

To spot-check a compiler change against a real `.hl` program
without installing:

```sh
cargo run -p hale-cli --bin hale -- run path/to/prog.hl
cargo run -p hale-cli --bin hale -- build path/to/prog.hl
```

The in-tree `.hl` corpus lives at
`crates/hale-codegen/tests/fixtures/examples/` (the broadest
acceptance surface — `crates/hale-syntax/tests/examples.rs`
parses all of them).

## Repo conventions

- **Hale** is the language. **lotus** is the runtime substrate.
  C-runtime symbols stay `lotus_*` by design.
- The spec under `spec/` is the canonical contract. It describes
  shipped behavior, not aspirations. If the impl changes
  user-visible behavior, the spec changes in the same commit.
- The `docs/` mdBook is the pedagogical companion to `spec/`. When
  a spec change alters user-facing surface or behavior (a new
  keyword, lifecycle method, sugar, diagnostic, or semantic
  rule), update the relevant `docs/src/` chapter in the same
  change — the book is easy to forget and drifts silently.
- The user owns commit cadence — never commit without an
  explicit ask.
- Don't generate planning / status / progress markdown files in
  the repo.
