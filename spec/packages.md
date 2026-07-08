# Package management

Hale's v1 package-management surface is small and explicitly
git-based. A project declares its direct dependencies in
`hale.toml`; running `hale fetch` clones each one into
`vendor/<name>/` and records resolved commit SHAs in
`hale.lock`. `import "vendor/<name>" as alias;` directives pick
the cloned source up via the standard import resolution order
(see `spec/projects.md`).

The fetched tree lives at `vendor/` — toolchain-managed,
distinct from `lib/` which stays for hand-maintained sources
the user vendors directly. Both paths work identically through
the import resolver (each is just an `<importer-dir>/<path>/`
hit on path 1), but keeping them physically separate prevents
`hale fetch` from clobbering hand-maintained source on a name
collision.

The single design commitment driving the shape:

- **F.26 — vendoring is the v1 dependency primitive.** Hale
  ships no registry, no version-solver, no transitive
  resolution. Each project lists its direct dependencies and
  the consumer vendors any transitive ones explicitly.

See `spec/decisions.md` F.26 for the rationale.

## Manifest (`hale.toml`)

The manifest lives at the project root — the same directory
that hosts the top-level `.hl` sources and (after `hale
fetch`) the `vendor/` directory. It is a TOML file with one
required section, `[deps]`:

```toml
[deps]
helpers = { git = "https://github.com/me/helpers", rev = "abc123" }
finance = { git = "https://github.com/me/finance", tag = "v0.1.0" }
ui      = { git = "https://github.com/me/ui",      branch = "main" }
```

Each entry's key is the local namespace the consumer will use
to import the dep. The value is a table with one required field
and up to one optional pin:

| Field    | Required | Description                                            |
|----------|----------|--------------------------------------------------------|
| `git`    | yes      | The clone URL. Any scheme git understands works.       |
| `rev`    | no       | Pin to a specific commit SHA.                          |
| `tag`    | no       | Pin to a named tag.                                    |
| `branch` | no       | Track a branch (lockfile still pins the resolved SHA). |

Setting more than one of `rev` / `tag` / `branch` is a manifest
error — the spec must be unambiguous. Setting none uses the
remote's default branch.

There is no `[package]` table, no top-level metadata, no
authors / description / license fields. A project is identified
by its directory name and its source.

### Pin semantics

- `rev = "<sha>"` — the consumer pins to that exact commit.
  Triggers a full (non-shallow) clone, because git's
  `--depth 1 --branch <ref>` form rejects raw SHAs. After
  cloning, `git checkout <sha>` lands the working tree.
- `tag = "<tagname>"` — shallow clone of the tag's commit.
  Idempotent re-fetches don't update; the lockfile pins the
  SHA the tag pointed to when fetched.
- `branch = "<branchname>"` — shallow clone of the branch tip
  at fetch time. The lockfile still pins the SHA. To pick up
  upstream changes on a tracked branch, delete the lockfile
  (or just the lockfile entry for that dep) and re-run
  `hale fetch`.

## Lockfile (`hale.lock`)

Auto-written by `hale fetch`. Pins every declared dep to a
resolved commit SHA so re-cloning is reproducible across
machines and across time:

```toml
[[dep]]
name = "helpers"
git = "https://github.com/me/helpers"
sha = "abc1234567890abcdef..."

[[dep]]
name = "finance"
git = "https://github.com/me/finance"
sha = "deadbeefcafef00d..."
```

The lockfile is intended to be committed alongside the
manifest. A consumer running `hale fetch` on a fresh checkout
re-clones every dep at the locked SHA, producing the same
`vendor/` contents the author worked with.

If a dep listed in the manifest has no entry in the lockfile
(new dep), `hale fetch` resolves it freshly and appends to
the lockfile. If a dep is removed from the manifest, its
lockfile entry is dropped on the next fetch (the lockfile is
re-emitted from the current manifest, not edited in place).

The `hale.lock` shape is owned by the toolchain — manual
edits will be overwritten on the next `hale fetch`. To
upgrade or downgrade a dep, edit the manifest and re-run
`hale fetch`.

## The `hale fetch` command

```
hale fetch [repo-root]
```

`repo-root` defaults to the current working directory. The
behavior, per dep:

1. **First fetch (no `vendor/<name>/`).** Clone the URL into
   `vendor/<name>/`, checking out the requested ref (`--depth 1`
   for tag / branch / default-branch; full clone + `git
   checkout` for `rev`).
2. **Re-fetch, lockfile SHA matches current HEAD.** No-op —
   no network call.
3. **Re-fetch, lockfile SHA differs from current HEAD.** Run
   `git fetch --tags --prune origin`, then `git checkout` the
   requested ref. Updates the lockfile with the new resolved
   SHA.
4. **Re-fetch, no lockfile entry for the dep.** Same as case 1
   from the consumer's perspective — the dep is new to this
   project even if `vendor/<name>/` was somehow already present.
5. **Collision with a hand-maintained directory.** If
   `vendor/<name>/` exists but has no `.git/`, `hale fetch`
   errors and refuses to overwrite it. Move or delete the
   directory and re-run. This guards against silently
   clobbering sources the user vendored by hand (e.g. before
   adding the dep to `hale.toml`).

After processing every dep, `hale fetch` writes a fresh
`hale.lock`. The write is whole-file (no in-place editing)
so a partial / failed fetch never leaves a corrupt lockfile.

Exit codes:

- `0` — every dep resolved cleanly.
- `1` — a manifest error, a lockfile parse error, a network
  failure, or a `git` invocation returning non-zero.

## Resolution order interaction

The compiler's import resolver doesn't know about
`hale.toml` or `hale.lock`. It only knows that an `import
"vendor/<name>" as alias;` (or `import "lib/<name>" as alias;`,
or any other path) directive looks for source on disk at the
paths described in `spec/projects.md` § Resolution order.
`hale fetch` puts the source at `<repo-root>/vendor/<name>/`,
which is exactly where path-1 of the resolver looks first when
the import string is `"vendor/<name>"`.

This separation is deliberate: the fetcher is a small,
optional tool that produces an on-disk tree. The compiler
treats that tree the same way it treats hand-vendored source
under `lib/`. A project that already vendors its libraries
(committed into `lib/`) can ignore the package manager
entirely; a project that uses `hale fetch` exclusively gets
the same compile behavior with less manual maintenance; and a
project that does both keeps the trees physically separate so
the toolchain never overwrites work it didn't put there.

## Library author conventions

A library is just a git repository whose root directory holds
one or more `.hl` files. From the consumer's perspective the
clone lands at `vendor/<name>/`, which becomes one Hale seed
(per F.19). What this implies for library authors:

- **Source goes at the repo root**, not under `src/`. Nested
  directories are NOT crawled — they exist for the author's
  organization only.
- **Include `hale.toml` in a library only if it has FFI
  bindings.** As of 2026-05-22, a library that declares
  `@ffi("c")` functions ships an `hale.toml` with an `[ffi]`
  section listing `link = [...]` (C libraries the consumer's
  link line should pull in) and `csrc = [...]` (C glue files
  compiled alongside the runtime). The consumer's build reads
  the section automatically at import time — no manual
  `--link` / `--csrc` flags needed. See [`spec/ffi.md`](./ffi.md)
  for the FFI contract and
  [`agents/binding-packages.md`](../agents/binding-packages.md)
  for the authoring brief. Pure-Hale libraries (no `@ffi`)
  have no reason to ship `hale.toml`; their consumers
  resolve them through the standard import lookup.

  v1 still has no transitive `[deps]` resolution: a library
  declaring its own `[deps]` won't have those auto-fetched by
  the consumer's `hale fetch`. The consumer vendors every
  dependency themselves. Document any indirect requirements in
  your README.
- **Tag releases** if you want consumers to pin via
  `tag = "vX.Y.Z"`. The toolchain doesn't enforce semver — the
  tag is just a git ref — but consumers will read your tag
  history when deciding what to pin to.
- **Keep top-level decl names short and namespace-friendly.**
  Consumers will see your decls as `alias::Name`; the alias is
  theirs to choose, so design decls that read fluently under
  any short prefix. See `spec/styleguide.md` for naming rules.

## What's NOT in v1

Explicit non-features. A future milestone may relax some of
these when concrete friction demonstrates the need.

- **No registry.** All deps are git URLs.
- **No transitive deps.** A library's own dependencies are not
  followed. The consumer must vendor them too.
- **No version ranges or semver.** Pins are exact strings (a
  rev, tag, or branch name). There is no
  highest-compatible-version solver.
- **No `hale publish`.** Distribution happens via `git push`
  to a hosting service of the author's choice.
- **No checksum verification beyond git's own.** A pinned SHA
  is the integrity guarantee — git's content-addressing makes
  swapping content under a fixed SHA infeasible.
- **No build scripts or post-fetch hooks.** A library is
  source only; no arbitrary code runs during `hale fetch`
  beyond the `git` invocations the toolchain controls.

## Implementation entry points

- `crates/hale-cli/src/pkg.rs` — `Manifest`, `DepSpec`,
  `Lockfile`, `LockedDep` types (serde) and the `fetch()`
  entry point.
- `crates/hale-cli/src/main.rs` — the `hale fetch`
  dispatch arm.
- `crates/hale-cli/tests/pkg_fetch.rs` — integration tests
  that exercise a real `git clone` against a `file://` URL.

The compiler's import-resolution path (which consumes the
cloned source) lives in `hale-cli`'s `parse_with_imports` /
`resolve_import` / `find_workspace_root` family — see
`spec/projects.md` § Implementation entry points.
