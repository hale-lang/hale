# Contributing to Hale

Hale is early and moving fast, but contributions are very welcome —
especially a second set of hands on the compiler, the standard library,
and the docs.

## Two kinds of work

- **Writing Hale programs / libraries.** Start with
  [`AGENTS.md`](./AGENTS.md) — the canonical guide for `.hl` authorship
  (a tight read for humans, too) — and
  [`agents/library-dev.md`](./agents/library-dev.md) for stdlib and
  library work.
- **Working on the language itself** (compiler / runtime / spec). See
  [`agents/compiler-dev.md`](./agents/compiler-dev.md), plus the build
  notes below.

## Build & test

Codegen needs **LLVM 18** dev libraries with `llvm-config-18` on `PATH`
(or `LLVM_SYS_180_PREFIX` set); `inkwell` is pinned to `llvm18-0` —
LLVM 17 / 19 / 20 will not link.

```sh
cargo build --release
cargo test --release --workspace -- --test-threads=1
```

The serial flag avoids "text file busy" flakes from parallel test
binaries racing on the same temp path. To spot-check a change against a
real program without installing:

```sh
cargo run -p hale-cli --bin hale -- run   path/to/prog.hl
cargo run -p hale-cli --bin hale -- build path/to/prog.hl
```

The in-tree `.hl` corpus lives at
`crates/hale-codegen/tests/fixtures/examples/` — the broadest
acceptance surface.

## Conventions

- **`spec/` is the canonical contract.** It describes shipped behavior,
  not aspirations. If a change alters user-visible behavior, the spec
  changes in the *same* commit, and the matching `docs/src/` chapter is
  updated alongside it.
- **The substrate is model-checked.** If you touch a concurrent
  primitive in the C runtime, add or update its model under
  [`verification/`](./verification/README.md) — the `genmc` CI job gates
  on it. See [Verification](./docs/src/verification.md) for the
  user-facing overview.
- **The author owns commit cadence.** Open a PR; don't assume direct
  pushes to `main`.

## Picking something up

Open an issue describing what you want to change before a large PR, so
direction can be agreed early. Small, self-contained wins that are
always useful: a new `.hl` program that exercises a real workload for
the fixture corpus, or a thin docs chapter fleshed out. Issues tagged
[`good first issue`](https://github.com/hale-lang/hale/labels/good%20first%20issue)
are scoped to be approachable when they're available.
