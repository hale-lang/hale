# 04-child-aggregates

One locus, three reads over its children. An aggregator over leaf
children that reads the same underlying state through three methods.

```
locus LeafL {
    params { value: Int = 0; }
    contract { expose value: Int; }
}

locus AggregatorL {
    params { B: Int = 100; c: Int = 1; sigma: Int = 1; phi: Float = 1.0; }
    contract { consume value: Int; }

    fn total()   -> Int { ... }    // total sum
    fn count()   -> Int { ... }    // count
    fn largest() -> Int { ... }    // max single value

    run() {
        let _l1 = LeafL { value: 10 };
        let _l2 = LeafL { value: 20 };
        let _l3 = LeafL { value: 30 };

        println("total:   ", self.total());
        println("count:   ", self.count());
        println("largest: ", self.largest());
    }
}
```

Expected output:
```
total:   60
count:   3
largest: 30
```

## What runs

1. `main()` invoked. Aggregator instantiates as anonymous
   child of `main`.
2. Aggregator's `run()` begins.
3. Three leaves instantiated, each bound to a local handle.
   `accept(leaf)` runs on aggregator before each leaf's region
   is allocated. Then leaves register in `self.children`.
4. `self.total()` invoked. Iterates `self.children`, sums values:
   10 + 20 + 30 = 60.
5. `self.count()` invoked. Iterates, counts: 3.
6. `self.largest()` invoked. Iterates, finds max: 30.
7. `run()` returns. Local handles `_l1`/`_l2`/`_l3` go out of
   scope; leaves dissolve (depth-first, per F.4).
8. Aggregator dissolves. No closures declared, so collapse.
9. `main()` returns. Process exits 0.

## Primitives this exercises (new vs. 03)

- **`self.children`** — built-in typed iterable of accepted
  coordinatees. For AggregatorL with single-type `accept(leaf:
  LeafL)`, `self.children` is `[LeafL]` (F.11).
- **`for child in self.children`** — iteration over the
  collection. Cost reflects the locus's projection class
  (chunked here = O(N) per iteration).
- **Several reads of one locus's state.** Each `fn` member reads
  the same children in the locus's arena; no copy is made between
  them.
- **Bound child handle to extend lifetime.** `let _l1 =
  LeafL { ... }` binds the leaf to a local in `run()`'s scope;
  the leaf lives until the binding goes out of scope (here:
  `run()` returns). Without binding, the unbound leaf would
  dissolve at statement boundary (per §A) since LeafL has no
  `run()`.

This example was `04-modes` until locus modes were removed
(spec/decisions.md F.61): its three `mode` declarations are the
three `fn` members above.

## Next on the ladder

`05-bus` — a locus that consumes inbound messages from a typed
bus channel and emits typed messages back.
