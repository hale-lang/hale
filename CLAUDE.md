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
cargo nextest run --release --workspace
```

**Run the suite in parallel.** It used to require
`--test-threads=1` because ~131 test files wrote their compiled
binary to a temp path with no uniquifier (mostly
`temp_dir()/lotus_test_{name}`), so two tests could race on one
path — "text file busy", or worse, a test silently executing
another test's binary.

That is fixed structurally: every test builds through
`harness::unique_bin` (pid + process-local counter), and
`harness_paths_are_unique.rs` fails the build if a new test
rolls its own. Ports come from `harness::free_port()` rather
than the hand-maintained 57xxx/47xxx registry. Serial runs still
work, they are just slower and no longer buy anything:

```sh
# one integration test in hale-codegen
cargo test --release -p hale-codegen --test topic_phase2
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
