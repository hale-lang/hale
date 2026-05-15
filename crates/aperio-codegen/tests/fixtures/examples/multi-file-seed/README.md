# multi-file-seed — per-directory seed model

The smallest demonstration of the per-directory seed model:
two `.ap` files in one directory, built into one binary.

## Files

- `main.ap` — declares `AppL` (the app locus) and `fn main()`.
- `greet.ap` — declares `greeting` and `say` helper fns.

`main` calls `greeting` and `say` directly by bare name; no
`import` statement, no `use`, no path. They live in the same
seed (this directory), so they share one top-level scope.

## Build and run

```
aperio build examples/51-multi-file-app
./examples/51-multi-file-app/51-multi-file-app
```

Output: `hello, world!`

The binary name is the directory's basename. File order in
the merged bundle is alphabetical (`greet.ap` first, then
`main.ap`); order doesn't affect resolution because the
typechecker flattens all top-level decls into one shared scope
before resolving names.

## Why dir seeds

Single-file apps grow unwieldy quickly (ferryman hit 2,300
lines before this milestone landed). The per-directory seed
model lets a project decompose into focused files — one for
the app locus, one for IO helpers, one for rendering, etc. —
without a module system. Inspired by Go's per-package
visibility model.

## Single-file builds still work

`aperio build examples/hello-world/main.ap` builds a single
file as before. The directory shape is additive, not a
replacement.
