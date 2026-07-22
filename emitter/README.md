# emitter — protocol test fixtures

Reference implementation of `../PROTOCOL.md`, in C11 on
purpose: the protocol's fixtures must not depend on the Hale
toolchain.

| File | What it is |
|---|---|
| `protocol.h` | The executable form of PROTOCOL.md. Layout facts pinned with `static_assert`s — drift fails the build. Consumers include this. |
| `synth.c` | Synthetic emitter. Simulates an order-processing system (main → supervisor → producer/router/3 workers, one networked unix binding) and emits protocol-v0 telemetry into shm. |
| `peek.c` | Minimal consumer / smoke tool: attach, decode manifest + rings, k-way merge by timestamp, stream events, track seq-based loss, print counters. The consumer library's seed, not its replacement. |

## Quick start

```
make
./synth --rate 20000 --loss 5000 --churn 2 &
./peek           # or: ./peek --summary-only
```

`synth` flags: `--rate` msgs/sec aggregate · `--rings` ·
`--slots` (power of two) · `--duration` seconds ·
`--loss` ppm of networked deliveries dropped (seq gaps are
the evidence) · `--churn` seconds between worker
kill/restart cycles.

Signals: `SIGUSR1` → post-mortem dump
(`hale-obs-<pid>.dump`, dump header + verbatim segment);
`SIGINT`/`SIGTERM` → clean shutdown (unregisters, unlinks
shm).

What the pair exercises end-to-end: registration/discovery,
manifest decode + dynamic append (a topic registers 2 s in,
`peek` rescans on `manifest_gen`), packed-record decode with
per-ring EPOCH reconstruction, k-way timestamp merge,
overwrite-oldest overrun accounting, dormant wake
(`observer_count`), per-topic mode mask, counters, NET seq
loss (counter truth + reorder-tolerant estimate),
supervision churn (dissolve → superv → restart → birth),
`current_locus` gauge updates, post-mortem dumps.

Not yet here: dump-file reading in `peek` (consumer library
work), SAMPLED-RICH/FIREHOSE modes, multi-process fusion
(run two synths — fusion is the consumer's job).
