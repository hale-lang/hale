# 04-modes

One kernel, three projections. An aggregator over leaf children
that reads the same underlying state through `bulk`, `harmonic`,
and `resolution` modes.

```
locus LeafL {
    params { value: int = 0; }
    contract { expose value: int; }
}

locus AggregatorL {
    params { B = 100; c = 1; sigma = 1; phi = 1.0; }
    contract { consume value: int; }

    mode bulk()       -> int { ... }    // total sum
    mode harmonic()   -> int { ... }    // count
    mode resolution() -> int { ... }    // max single value

    run() {
        let _l1 = LeafL { value: 10 };
        let _l2 = LeafL { value: 20 };
        let _l3 = LeafL { value: 30 };

        println("bulk (total):     ", self.bulk());
        println("harmonic (count): ", self.harmonic());
        println("resolution (max): ", self.resolution());
    }
}
```

Expected output:
```
bulk (total):     60
harmonic (count): 3
resolution (max): 30
```

## What runs

1. `main()` invoked. Aggregator instantiates as anonymous
   child of `main`.
2. Aggregator's `run()` begins.
3. Three leaves instantiated, each bound to a local handle.
   `accept(leaf)` runs on aggregator before each leaf's region
   is allocated. Then leaves register in `self.children`.
4. `self.bulk()` invoked. Iterates `self.children`, sums values:
   10 + 20 + 30 = 60.
5. `self.harmonic()` invoked. Iterates, counts: 3.
6. `self.resolution()` invoked. Iterates, finds max: 30.
7. `run()` returns. Local handles `_l1`/`_l2`/`_l3` go out of
   scope; leaves dissolve (depth-first, per F.4).
8. Aggregator dissolves. No closures declared, so collapse.
9. `main()` returns. Process exits 0.

## Primitives this exercises (new vs. 03)

- **`mode bulk(...) -> T { }`** / **`mode harmonic(...) -> T { }`**
  / **`mode resolution(...) -> T { }`** — three mode declarations
  on a single locus. Each is a mode-projection of the same
  underlying kernel (locus state). The compiler emits one
  implementation per declared mode; all three share the locus's
  arena (F.5).
- **`self.children`** — built-in typed iterable of accepted
  coordinatees. For AggregatorL with single-type `accept(leaf:
  LeafL)`, `self.children` is `[LeafL]`. Multi-type accept
  (heterogeneous children) is deferred to a future example.
- **`for child in self.children`** — iteration over the
  collection. Cost reflects the locus's projection class
  (chunked here = O(N) per iteration).
- **`self.bulk()` / `self.harmonic()` / `self.resolution()` —
  modes invoked as methods.** Mode keywords appearing post-`.`
  parse as member names; the grammar's `postfix_op` accepts
  `bulk` / `harmonic` / `resolution` as identifier-equivalent
  in member-access position.
- **Bound child handle to extend lifetime.** `let _l1 =
  LeafL { ... }` binds the leaf to a local in `run()`'s scope;
  the leaf lives until the binding goes out of scope (here:
  `run()` returns). Without binding, the unbound leaf would
  dissolve at statement boundary (per §A) since LeafL has no
  `run()`.

## What writing this surfaced (for the spec)

Three issues, all resolved in this commit:

1. **Mode keywords as member names.** The grammar has `bulk`,
   `harmonic`, `resolution` reserved as keywords (per
   `tokens.md`). But `self.bulk()` is the natural invocation
   syntax. Resolution: post-`.` and post-`::`, mode keywords
   are accepted as member names. The grammar's `postfix_op`
   updated; new `member_name` non-terminal added. Documented
   in design-rationale §F.10.

2. **`self.children` semantics and typing.** For a locus with
   a single `accept(c: ChildType)` lifecycle, `self.children`
   is typed `[ChildType]` and yields the currently-attached
   coordinatees. `accept()` runs before registration (per
   F.7); after `accept()` returns normally, the child is in
   `self.children`. On child dissolution, it's removed. v0
   permits only single-accept-type loci; multi-type children
   require a sum type and are deferred. Documented in
   design-rationale §F.11.

3. **Mode invocation from inside a lifecycle method.** The
   modes are members of the locus; `self.bulk()` invokes the
   bulk mode as a method. Modes can also be invoked from
   outside the locus via the contract surface (i.e., a parent
   calling `child.bulk()` if bulk is contract-exposed) — but
   that's a future example. For 04, modes are internal-only.

## What this still does *not* exercise

- Bus interface — 05
- Multi-type children in `self.children` — later
- Mode-projection contract exposure (`contract { expose mode
  bulk: int }`) — later
- `run()` that responds to inbound messages — later (needs
  bus)
- Cross-locus coordination beyond strict parent-child — later
- Closure tests over `self.children` (the natural
  cyclic-closure pattern) — could be a 04.5 follow-up

## Decision I made on my own (push back if wrong)

I made the call to **allow keyword tokens after `.` and `::` as
member names** rather than renaming the mode keywords or
introducing a separate mode-invocation syntax (`self::bulk()`
or `mode_bulk(self)` or whatever). The reasoning:

- `self.bulk()` is the natural, ergonomic syntax for invoking
  a member of a locus.
- Renaming the modes to non-keywords (e.g., `BulkMode`) would
  ruin the framework-vocabulary alignment.
- Other languages (Rust, with raw identifiers) handle this
  routinely.
- The grammar change is local to `postfix_op` and well-scoped.

If you want a different syntax for mode invocation — or want to
keep mode names out of the post-dot identifier space — push
back and we'll redesign.

## Next on the ladder

`05-bus` — a locus that consumes inbound messages from a typed
bus channel and emits typed messages back. Adds the `bus { }`
block, transport binding via deployment config, and the
fitter/applier split's messaging primitive. After 05, the
fitter/applier pair is reachable.
