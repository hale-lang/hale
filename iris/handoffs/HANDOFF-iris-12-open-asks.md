# iris → hale: open asks

Self-contained. Nothing here needs a prior handoff to act on.
Everything verified against `main` @ `de44ab6` on 2026-08-12.

| # | Ask | Cost | Unblocks |
|---|---|---|---|
| P25 | Supervision in the topology artifact | medium | inspector Tier 1 (the richest live signal has nothing to anchor to) |
| P22 | Write per-binding cells 3–5 | small–medium | iris M2 (backpressure), DESIGN §7 |
| P24 | Topic declaration spans in `provenance` | small | inspector lens on the line users look at |
| P26 | Model identity in the segment header | small | exact drift detection |
| P20 | Decision: fix or close as stale | — | clears the board |

---

## P25 — supervision has no representation in the topology artifact

`on_failure` appears **nowhere**: not in `provenance.decls`, not
in `sorts.fns`, and there is no supervision section.

Verified on schema 1.9 with a locus declaring
`on_failure(c: Worker, err: ClosureViolation) { restart(c); }`:

```
sorts.fns   : ['App::run', 'Router::on_fill', 'Worker::on_order', 'main', 'score']
decls keys  : ['App', 'Fill', 'Order', 'Router', 'Worker', 'main', 'score']
supervision section? NO
```

Why it matters more than the others: supervision is where the
**richest live signal already is** — `RESTART`, `SUPERV_TRANS`
and `LOCUS_DISSOLVE` are all emitted, attributed, and flowing
today. iris can see every restart and cannot say which declared
policy it belongs to, because the model has no supervision in it.
"Declared retry cap 3, observed 3 in 40 s" is the single most
valuable annotation the inspector could draw, and it is the one
that is structurally impossible.

**Ask:** represent supervision in the artifact — at minimum the
`on_failure` handler as a spanned decl (so it is addressable),
ideally the policy it declares (restart / restart_in_place /
absorb / escalate / bubble) and any retry bound, per supervised
child type.

**Acceptance:** the artifact names the supervising locus, the
supervised child type, and a source span, for the fixture above.

## P22 — the per-binding backpressure counters are written by no release

PROTOCOL §6 has reserved seven cells on a binding line since v0:

```
bindings: sent, delivered, bytes, queue_depth (gauge),
          send_block_ns, retries, seq_high_water
```

The emitter writes **cells 0, 1, 2 only** — the three
`obs_count(MK_BINDING, id, 0|1|2, …)` calls in `lotus_obs.c`.
Cells 3–5 are written on no path, in any release. (Topic lines
are the same: 0/1/2 live, nothing else.)

Not a regression — a gap that stayed invisible because the demand
side was missing in the same place: iris never read them either.

Why it matters: iris's M2 is "loss and backpressure made
visible," and these three cells are exactly what separates a
**lossy** edge from a **saturated** one. Loss already renders
(the wire seq's gaps are the evidence, and that half works). A
consumer falling behind shows up first as depth climbing and
send-block time accruing, *before* anything drops — so without
3–5 the observer can only report the drop after the fact, which
is the failure mode M2 exists to get ahead of.

**Ask**, in priority order — the first alone unblocks the
milestone:

1. `queue_depth` (cell 3) — gauge, last-write-wins, stored at
   enqueue/dequeue.
2. `send_block_ns` (cell 4) — accumulated ns blocked in the send
   path, monotonic.
3. `retries` (cell 5) — monotonic transport-level retry count.

`seq_high_water` (cell 6) we derive consumer-side; no ask.

Cost note: all three are relaxed atomic ops on a line already in
cache from the `sent`/`bytes` bumps. These belong on the
**counters** tier — bumped under `obs_on()` without `obs_gate()`,
exactly as cells 0–2 are today, since counters are the
enabled-but-unobserved contract.

**Acceptance:** a deliberately overloaded consumer shows the
binding's depth climbing ahead of any loss.

## P24 — topic declarations carry no source span

`provenance.decls` covers loci, types and free fns.
`provenance.publishes` / `.subscribes` carry the publish and
subscribe **sites**. Nothing carries the `topic Orders { … }`
line itself — topics appear in `sorts.topics` and in the
`topics[]` rows, both span-free.

Verified on schema 1.9: `sorts.topics: ['Fills', 'Orders']`,
neither present in `provenance.decls`.

Consequence: an editor lens can be anchored on every publish and
subscribe site and not on the declaration a developer actually
looks at when asking "is this topic live?"

**Ask:** a `provenance.decls` entry for each topic, same
`{source, span}` shape as the others.

**Acceptance:** `provenance.decls` contains every name in
`sorts.topics`.

## P26 — put the model identity in the observation segment

The segment header is one page and uses `0x80` of it
(`manifest_gen` is the last field, at `0x78`) — 3968 bytes free.

**Ask:** write the artifact's model identity (`shape_hash`) into
a new header field at segment creation. Additive, so a
`proto_minor` bump; iris will amend PROTOCOL §3.

Why: iris now cuts a topology artifact from the working tree and
joins it against the live manifest. The one thing it cannot do is
establish that the *running binary* was built from the model it
is comparing against — today that is inferred from per-file
digests in the artifact, which is a guess about the build, not a
fact about the process. One u64 makes it exact.

Note this is complementary to payload drift, not a substitute:
model `shape_hash` deliberately excludes payload field shape,
which is precisely why the manifest's per-topic `shape_hash`
already covers the other half. Both together are complete.

(`.hale.topo` as an ELF section would subsume this — PROTOCOL §4
still says "when it exists upstream". The u64 is the cheap 90%.)

**Acceptance:** two builds of a program differing only in a
comment produce segments with the same value; adding a locus
changes it.

## P20 — decision needed, and "stale, closing" is a fine answer

Filed 2026-07-29, unanswered through three rounds. Restated once
so it can be closed either way.

Claim: every **statically declared** publisher counts
`CT_PUBLISHED`; every **`accept()`-spawned** publisher counts
zero — while its messages demonstrably deliver (consumers' dlv
counters and cross-process edges are full). Measured uniform
across three unrelated applications on v0.11.22:

| publisher | declaration | CT_PUBLISHED |
|---|---|---|
| gateway reader (keyed in-process topic) | static param default | 12.6k |
| coordinator loci publishing products | static param default | ~2k each |
| per-symbol loci → a remote-only plane | **accept()-spawned** | **0** (dlv 9.8k) |
| per-symbol loci → a second remote plane | **accept()-spawned** | **0** (dlv 2.0k) |

Not nested-handler related: a static publisher inside a delivery
handler counts fine.

Your four-flavor repro did not reproduce it. Our best guess at
the missing conjunction: the existing fleet-contract
accept()-spawned case asserts **attribution** on a keyed
**local** topic; the field shape is a spawned child's publish
being **counted** on a **remote-only plain** topic, observer
attached after steady state.

**Honest status:** the evidence is from v0.11.22 in July, and
that fleet is not currently up, so iris cannot re-measure on
demand. Several things have shipped since that plausibly touch
it. If that conjunction passes on current HEAD, close it — we
will take that as the answer and stop carrying it. Say the word
and we will build a local two-process repro of that exact shape
instead of asking you to chase a July measurement.

---

# Acceptance (iris, 2026-08-12)

All five verified against `main` @ `db7864d`, independently of
the changelog.

**P24 — topic decl spans. CONFIRMED.** Every name in
`sorts.topics` now has a `provenance.decls` entry with a span.

**P25 — supervision in the model. CONFIRMED**, and the shape is
better than asked: `supervision` carries locus / child / err /
ops, with `provenance.supervision` spans alongside. Putting it in
the *hashed* half is the right call — a supervision policy change
moving the model identity is precisely what makes an observer
able to say "this process predates the policy you are reading".

**P26 — model identity. CONFIRMED, exact.** proto 0.2,
`model_hash` at `0x80`, byte-equal to the artifact's `shape_hash`
on a live binary. iris shipped the consumer side the same day:
PROTOCOL §3.1 written, `protocol.h` bumped with a static_assert
at `0x80`, and the inspector now reports both identities as
independent axes — verified by editing a payload (topic hashes
move, model holds) and adding a locus (model moves, topic hashes
hold) against one running process.

Two details of yours that we adopted rather than worked around:
`0` as a real value meaning *no model* — our `synth` reference
emitter reads exactly that and now renders as "no model
(synthetic / non-Hale emitter)" — kept distinct from a proto 0.1
segment's *unknown*. `obs_model_hash()` returns a presence flag
rather than a value so the two cannot be conflated at a call
site, and guards on `header_len` as well as the version, since
`header_len` is what the emitter actually wrote.

**P20 — accepted closed as stale.** Your repro of the exact
conjunction is better evidence than our July measurement. Closed
on our board; we will re-file with a fresh local repro if it ever
reappears.

**P22 — cells are live; one caveat, no ask.** `send_block_ns`
accumulates on a real binding (324 ms over 200k sends on a
udp-connect topic), which settles the "written by no path" state
we reported. We did **not** independently confirm cells 3 and 5:
`queue_depth` and `retries` both read 0 in our probe, which is
expected for that transport (UDP does not queue; no reconnects
occurred), and our attempt at a stalled-unix-consumer shape
failed to connect for reasons on our end. Your overloaded-consumer
test is the right shape and we are taking it. Flagging only so
"iris verified 3–5" is not read into the record — we verified 4.

Consuming these is now iris-side work: fuse-hl fuses topic
counter lines and does not surface binding lines at all, so
nothing renders depth yet. That is M2's backpressure half and it
is ours.

## Board

Empty. No open asks.

Scoreboard: 26 findings across 12 handoffs; 26 resolved (19
upstream, 7 iris), 0 open, 2 retired, 1 retraction.
