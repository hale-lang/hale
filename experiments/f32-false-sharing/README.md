# F.32-1 false-sharing ceiling

Standalone C microbench establishing the theoretical max gain
for `notes/f32-cache-aware-delivery-plan.md` § F.32-1.

Two pthreads pinned to sibling cores hammer adjacent counters
with three layouts: shared cache line (`packed`), 64-byte
padded, 128-byte padded. Median ns/op across 5 rounds.

```sh
./run.sh
```

If `padded_64` is not measurably faster than `packed` on your
host, false sharing is not actually a contributor on this
hardware and F.32-1 won't pay off here — investigate cache
topology (`lscpu --extended=CPU,CORE,SOCKET,CACHE`) before
proceeding.

This bench is the C twin for the `.hl`-side coverage that lives
in the separate `hale-lang/bench` repo (not in this tree). The
two together bracket the F.32-1 acceptance window:

- C twin = theoretical max (pure increments, no hash/probe).
- `.hl` bench = realistic Hale @form(hashmap) gain.
- F.32-1 acceptance gate: the out-of-tree `.hl` bench targets
  >= 2x post-padding.

Per-bench rationale and methodology are in the bench.c header.
