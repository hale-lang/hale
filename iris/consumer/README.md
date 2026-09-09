# consumer — attach/decode library + fusion

The consumer side of the observation protocol. This is iris's
foundation layer; `fuse` is its first caller.

| File | What it is |
|---|---|
| `obs_attach.{h,c}` | Library: attach one segment from a registration file, manifest decode with generation rescan, per-ring EPOCH timestamp reconstruction, overrun-safe snapshot reads, merged event pull, counter access. |
| `fuse.c` | Multi-segment fusion: discovers every live registration, joins topics across segments by `(shape_hash, name)`, matches NET_SEND→NET_DELIVER across processes by `(topic key, seq)` into directed cross-process edges with measured latency, renders a top-style fused view at 1 Hz. |

## Quick start (two-process system)

```
make -C ../emitter && make
../emitter/synth --role consumer --sock /tmp/orders.sock --churn 3 &
../emitter/synth --role producer --sock /tmp/orders.sock --loss 5000 &
./fuse
```

The producer sends real packets over the unix dgram socket;
the consumer's seq gaps are therefore real wire behavior plus
injected loss. The fused view shows: per-process record/
overrun/restart counts, topics aggregated across segments
(join key = shape_hash), cross-process edges with matched
rate, mean/max latency (same-host CLOCK_MONOTONIC, per
PROTOCOL.md), an unmatched count (loss evidence), and a
structural-event tail (dissolve/restart/binding-down).

Verified at ~15k msgs/s across two processes: ~30 µs mean
edge latency, injected 0.5% loss tracking within rounding of
counter truth, zero consumer overruns.

Fusion has been proven against a real production-shape
workload: a market-data producer/consumer binary pair
communicating over a declared-layout shm ring, each carrying
~4 lines of `observe` instrumentation — full-rate seq
matching over the real transport, mean ~73 µs edge latency,
zero unmatched.

## Next

- Latency histograms (mean/max → percentiles)
- Dump-file attach (post-mortem via the same obs_attach path)
- Dead-segment retention + scrubbing (flight recorder UI)
- This library is the substrate the iris renderer consumes;
  `fuse`'s tables are the flower's data model in text form.
