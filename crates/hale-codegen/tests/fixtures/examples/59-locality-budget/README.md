# 59-locality-budget

Per-locus cache-tier budget via `@locality(L1|L2|L3|any)`. The
compile-time working-set estimator (F.32-2) compares each
locus's projected bytes against its declared tier when the
build runs with `--target-cache lN` or `--locality-report`.

```hale
@locality(L1)
locus Counter {
    params { ticks: Int = 0; scaled: Int = 0; }
    fn bump(n: Int) { ... }
}

@form(hashmap, sync = lockfree, cap = 49152)
@locality(L2)
locus Registry {
    capacity { pool entries of Entry indexed_by k; }
}

@form(hashmap, sync = lockfree, cap = 2048)
@locality(any)
locus Cache {
    capacity { pool entries of Entry indexed_by k; }
}
```

## What `--locality-report` shows

```
$ hale build 59-locality-budget --locality-report
locality report (F.32-2 working-set estimator):
  App       ~  819472 B  (fits L2)  struct=64 capacity=0 children=819408
  Cache     ~   32832 B  (fits L1)  struct=64 capacity=32768 children=0
  Counter   ~      80 B  (fits L1)  struct=80 capacity=0 children=0
  Registry  ~  786496 B  (fits L2)  struct=64 capacity=786432 children=0
```

(Exact bytes vary with host alignment; the relative tiers are
what's load-bearing.)

## What `--target-cache lN [--strict]` does

* **`--target-cache l1`** — Counter fits (its `@locality(L1)`
  matches). Cache is exempt (its `@locality(any)` opts out).
  Registry has `@locality(L2)`, which wins over the global
  flag — and at 768 KB it fits L2 (1 MB on this host), so
  no breach. App has no annotation, falls through to global
  L1 = 48 KB, and at 819 KB exceeds. Result: one warning
  naming App.
* **`--target-cache l1 --strict`** — same breach as above
  but exit-1 before codegen.
* **`--target-cache l3`** — everything fits L3 (96 MB on
  this host); clean build.

The precedence rule: per-locus `@locality(...)` always wins
when present (it's a hard contract). `@locality(any)` opts
the locus out. Otherwise the global flag applies.

## Primitives this exercises

- **`@locality(L1|L2|L3)`** — per-locus cache-tier hard
  contract. Always evaluated (regardless of CLI flags).
- **`@locality(any)`** — explicit opt-out from any global
  `--target-cache` gate.
- **`@form(...) + @locality(...)` stacked** — both annotations
  decorate the same locus, in either order.
- **`--locality-report`** — informational per-locus byte
  decomposition.
- **`--target-cache lN`** — global per-tier gate; per-locus
  annotations override per-locus when present.
- **`--strict`** — converts breach warnings to build errors.

## How to read the report's columns

| Column | Meaning |
|---|---|
| `struct` | Sum of `params { }` field sizes + arena overhead (64 B). Alignment-padded since F.32-2 v0.2. |
| `capacity` | `@form(...)` `cap = N` × cell stride. Empty if no `@form` cap. |
| `children` | Recursive expansion through locus-typed param fields. |

The total is `struct + capacity + children`. Tiers come from
sysfs (`/sys/devices/system/cpu/cpu0/cache/index{0,2,3}/size`)
on Linux, static fallbacks elsewhere.

## What this still does *not* exercise

- Method-scratch high-water mark — the working-set estimator's
  v0.2 doesn't account for transient allocations inside
  method bodies (`BytesBuilder` accumulation, String concat).
- Auto-applied sync inference — F.32-1∞ picks a sync but
  doesn't (yet) inject the kwarg into the AST; users still
  paste `sync = X` by hand after reading the diagnostic.
