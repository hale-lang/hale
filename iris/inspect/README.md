# inspect — the model you are editing vs. the fleet that is running

Slice 1 of [`INSPECTOR.md`](../INSPECTOR.md): the wire-shape drift
check, plus declared-but-silent. Pure Hale, no FFI — a client of
fuse-hl's HTTP surface, which is the same surface the MCP
co-debugger reads (DESIGN §11). The artifact is local; the fleet
need not be.

```
hale build .
./inspect [artifact.json] [http://host:port]     # defaults: topology.json, :8787
```

Exit status is the point: **0** clean, **1** drift found, **2**
could not run the check. It is a pre-deploy gate, not just a
readout.

## Two axes, both reported

Every run answers two independent questions, because checking
only one leaves you blind to half the drift:

- **Is this process built from my model?** — `model_hash`
  (PROTOCOL §3.1, proto 0.2) against the artifact's `shape_hash`.
  Moves when you add a locus, rewire, or change supervision.
- **Do the payload shapes agree?** — per-topic hashes. Move when
  you edit a payload type, and *not* when the model changes.

A payload edit trips the second and not the first; adding a locus
trips the first and not the second. Both report `MODEL DRIFTED` /
`DRIFTED` respectively and exit 1.

Three model states are kept distinct: a hash that matches, a hash
that differs, and **`no model`** — a real `0`, meaning a
synthetic or non-Hale emitter (this repo's `synth` reads exactly
that) — versus **`unknown`**, an emitter predating proto 0.2 that
carries no field at all. Absent is not zero.

## What it answers

```
iris inspect
  artifact  demo.topo.json  (digest verified, 3 declared topics)
  fleet     http://127.0.0.1:8811  (1 observed topics)

  silent   Audits  (subject "audit") declared, never observed
  DRIFTED  Fills  (subject "fills")
           tree  56d6a83209bb52ac  id:i;qty:i;venue:s
           fleet c17ac6730391909f  <- running processes are on this
           a mixed fleet will not fuse on this topic, and
           headered datagrams will not deserialize at the old peers.
  silent   Orders  (subject "orders") declared, never observed

  0 in sync, 1 drifted, 2 declared-silent
```

- **DRIFTED** — you edited a payload type; the running processes
  are still on the old shape. This is handoff-4's P16 landmine
  (the silent partition) caught at edit time. It is invisible to
  the type checker by construction: every binary is perfectly
  consistent with itself.
- **declared-silent** — a topic nothing has published or consumed.
  The manifest registers lazily, on first use, so a topic that
  never carries a message never appears; the verdict is a set
  difference and costs nothing.
- **in sync** — hashes agree, with the fused pub/dlv totals.

## Why the join is exact

The artifact's `topics[].subject` is the byte-exact key the
emitter's manifest rows use, and its `payload_hash` is the same
`hale_types::topic_identity` value the manifest carries as
`shape_hash` (PROTOCOL §4, pinned vectors). So a mismatch is not
a heuristic — it is the fusion key itself disagreeing.

`artifact_digest` (schema 1.3) is verified before anything is
joined, and a mismatch **refuses** rather than warns: the
`topics` rows this tool joins on are exactly the part
`shape_hash` does not cover, so a failed digest makes the join
meaningless. Three states are kept distinct, never two — verified,
wrong, and *absent* (any artifact before schema 1.3). A consumer
may accept an older artifact but must never mistake "nothing to
check" for "checked and intact".

## Try it

```
cd ../examples/inspect-demo && hale build .
LOTUS_OBS=1 ./inspect-demo &
cd ../../consumer/fuse-hl && ./fuse-hl 8811 ../../render/web &

cd ../../examples/inspect-demo
hale check . --dump-topology=/tmp/demo.topo.json
../../inspect/inspect /tmp/demo.topo.json http://127.0.0.1:8811
```

Then, **without restarting the app**, add a field to `Fill`,
re-cut the artifact, and run `inspect` again. The tree has the new
hash; the fleet still has the old one.

## Known limits

- Topics only. Loci, handlers and bindings are Tier 1 of
  INSPECTOR.md and want the source index.
- `(other)` rows — observed but not in this artifact — are
  informational, not findings: in a fleet that is simply another
  binary's topic, and one artifact is one seed's closed world.
- ~~**`declared-silent` is ambiguous**~~ **RESOLVED upstream
  (2026-08-11, `5567bf2`), one day after it was filed.** A topic
  whose subscribers all lived inside the publisher's own subtree
  was rewritten by `desugar_intra_locus_topics` into a direct
  handler call before lowering, so it never reached the bus and
  never registered a manifest row — reading exactly like a topic
  nobody publishes. The desugared call site now emits both probes
  branch-gated like every other flavor, so a trafficked intra-tree
  topic registers and counts like its bus-dispatched sibling, and
  a zero-traffic topic is absent on every flavor *uniformly*.

  That uniformity is the property this tool actually needs:
  **absence now means "never mentioned at runtime", on every
  dispatch flavor.** The demo's `Orders` was left in place as the
  standing witness — App publishes it to `Worker`, its own child.
  It read `declared-silent` while visibly driving `Fills`; on the
  fixed compiler it reads `in sync` at pub 80 / dlv 80, while
  `Audits` (genuinely never published) correctly stays silent.
  Both halves matter: the verdict flipped where it should and
  held where it should.
