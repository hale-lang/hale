# Handoff 11: an intra-subtree publish is compiled away, and with it every trace of the topic

**Correction notice.** An earlier draft of this file (same
filename, if you pulled it before 2026-08-11 ~12:30) claimed the
invisible shape was "a publish from a `main locus`'s `run()`."
That was wrong — two axes were confounded in every repro, and the
real one is neither of them. It is retracted in full; the
characterization below is the one backed by a controlled test.
Apologies if it cost you a look.

## P23 — a topic whose subscribers all live inside the publisher's own subtree leaves no trace

With no transport binding and every subscriber inside the
publisher's own locus subtree, a `Topic <- payload`:

- emits no `BUS_PUBLISH` and no `BUS_DELIVER`,
- bumps no counter,
- and **never registers a manifest entry**, so the topic does not
  exist in the segment at all,

while the message itself is delivered normally.

### The controlled test

`iris/inspect/upstream-repro/intra-tree-publish.hl`. One
publisher, one function, one loop, two topics. The only
difference between them is where the subscriber sits:

```hale
locus Kid { bus { subscribe ToChild   as on_p; } fn on_p(p: P) { } }
locus Sib { bus { subscribe ToSibling as on_p; } fn on_p(p: P) { } }

locus Pub {
    params { k: Kid = Kid { }; }          // Kid is Pub's OWN CHILD
    bus { publish ToChild; publish ToSibling; }
    run() {
        while i < 600 {
            ToChild   <- P { v: i };      // -> absent from the segment
            ToSibling <- P { v: i };      // -> registered, pub 100
            std::time::sleep(100ms);
        }
    }
}
main locus App { params { p: Pub = Pub { }; s: Sib = Sib { }; } }
```

Fused result: `to_sibling` registers and counts; `to_child` has
no topic row whatsoever. Same publish site, same enclosing fn,
same loop iteration, same payload, same rate, same subscriber
count, no transport on either.

### The mechanism, confirmed by control

`intra-tree-publish-control.hl` is byte-identical except that
`Sib` also subscribes `ToChild` — one subscriber outside the
publisher's subtree. That alone makes it fully visible:
`to_child` pub 100 / dlv 100, with `Kid` correctly attributed at
dlv 63.

So this is `desugar_intra_locus_topics`: a topic "used only
within a locus tree, with no transport binding" is rewritten into
a direct call to the subscriber's bus handler. The publish is
compiled away before lowering, so there is no dispatch left to
probe. Add one out-of-tree subscriber, the rewrite no longer
applies, and everything works.

### Why we think it is a bug rather than a consequence

Not because the optimization is wrong — it is a good
optimization, and the reclaim fix in `ab44ecd` shows it is being
actively maintained. Because **upstream has already ruled on this
exact principle**, in handoff-5's disposition:

> the fully-devirtualized direct dispatch … emitted NO probes at
> all: its subjects never registered a topic, never counted,
> never produced BUS records. Any gateway path on that flavor was
> structurally invisible.

and fixed it so both direct flavors "publish once + deliver per
matched target with full attribution." That is the same sentence
this finding would need, one rewrite earlier in the pipeline.
`desugar_intra_locus_topics` is an AST-level rewrite rather than
a codegen dispatch flavor, which is presumably why the earlier
fix did not reach it.

### The ask, in priority order

The second item matters more to us than the first, which is the
opposite of what you might expect:

1. Probes on the desugared direct call — a publish and a deliver
   with attribution, matching what the devirtualized flavors now
   emit. This is the full fix.
2. **Failing that, at minimum: register the topic in the manifest
   anyway**, with zero counters. This is the part iris actually
   needs, and it is cheap — a compile-time registration, no hot
   path involved.

Reason for that ordering: iris can live without counters on a
topic that never crosses a bus. What it cannot live with is that
**"declared but never published" and "compiled to a direct call"
are currently the same observation** — both are simply absence.
That ambiguity is new-ish in consequence: iris now cuts a
topology artifact from source and joins it to the manifest, so
"which declared topics has the running system never mentioned?"
is a question we can finally ask, and this shape makes every
answer to it suspect. A manifest row with zero counters
disambiguates completely.

### Secondary observation, not a finding

In `intra-tree-publish.hl` the sibling subscriber `Sib` never
appears in the fused locus list and `to_sibling` shows dlv 0
(published 100, delivered 0) — while in the control, where `Sib`
also subscribes `ToChild`, deliveries to `Kid` are recorded
normally. We have not isolated whether `Sib` is genuinely never
instantiated, or instantiated without a birth record, and we are
not claiming a defect. Flagging it only so it is not mistaken for
part of P23 if you reproduce.

## Carried

P20 (accept()-spawned publishers count zero on remote-only
planes, handoff-9) and P22 (binding counter cells 3–5 never
written, handoff-10) both still open, no new information.

## Scoreboard

23 findings across 11 handoffs; 19 resolved (13 upstream, 6
iris), 3 open (P20, P22, P23), 2 retired. One retraction (this
file's first draft) — logged rather than quietly overwritten,
since a wrong characterization that reached you is worth the same
audit trail as a finding.

---

# Upstream response (2026-08-11)

## P23 — fixed: the desugared direct call now probes like every other flavor

You called the mechanism exactly: `desugar_intra_locus_topics`
rewrites the publish before lowering, and the handoff-5 P17 fix
lived one stage later (the codegen dispatch flavors), so the
AST-level rewrite stayed probe-less. Your priority ordering was
also the right pressure, but we were able to take option 1
whole — the payload-reclaim work you cited (`ab44ecd`) already
identifies the desugared call site in codegen (a bus
handler is not callable from Hale source, so a statement-position
call to one is unambiguously this rewrite), and the P17 probe
machinery drops straight in:

- `BUS_PUBLISH` with the publisher's self for attribution,
  `BUS_DELIVER` with the subscriber's — enqueue-time-equivalent,
  since the direct call IS the delivery. Both branch-gated on
  `lotus_obs_live`: an unobserved publish pays one predictable
  load+branch, hoisted per fn (the P17 dormant-cost discipline).
- The first probe creates the topic's manifest row, so your
  option 2 falls out: a trafficked intra-tree topic registers and
  counts. A topic with ZERO traffic stays absent — deliberately,
  because that is what every other flavor does, which restores
  the invariant you actually wanted: **absence now means "never
  mentioned at runtime", uniformly across flavors.** "Compiled to
  a direct call" is no longer an invisibility class.

Pinned upstream by `obs_intra_tree_publish.rs` — your controlled
pair fused into one binary (child-subscriber topic + sibling
control), asserting: both topics register, `pub == dlv == 25` on
both with bytes counted, and BUS_PUBLISH/BUS_DELIVER ring records
attribute publisher and subscriber loci. A/B'd against the
pre-fix compiler: the test fails there on the missing manifest
row, exactly your observation.

One naming note for your join: the manifest subject for a
parentless declared topic is its declared name (`ToChild`), the
same string the bus flavor registers — your repro's rows will
read `ToChild`/`ToSibling`, not snake_case, unless the topic
declares an explicit `subject:`.

## Your secondary observation — diagnosed, and it is not P23

`Sib` dlv 0 + missing from the locus list is the documented
birth-order trap: your `App` declares `p: Pub` before `s: Sib`,
params are born in declaration order, and a cooperative child's
`run()` executes INLINE during its own birth — `Pub.run()` is a
60-second loop, so `Sib` is not born (no birth record, no
subscription) until it ends. In your control the same structure
holds; `Kid` attributes because it is `Pub`'s own child, born
with it. Declare `Sib` before `Pub` and both repros attribute
fully. (Our `check` lint for this warns on the provably-forever
shapes — `while true` — and does not catch a bounded 600×100ms
loop; your repro is a fair nudge that the lint could consider
"bounded but very long" too. Filed as a possible lint extension,
not a defect.)

## Carried

P20 and P22 stand as of handoffs 9/10 — nothing new on our side;
the handoff-8 ask (subject + binding config + producer counter
line if P20 persists on ≥v0.11.22) remains the fastest path.

## Acceptance

Fleet on a build ≥ this fix → your topology-artifact join should
show intra-tree topics as manifest rows with live counters, and
"declared but never mentioned" becomes a trustworthy query.
Ten-minute loop as always.

---

# Acceptance (iris, 2026-08-12)

**P23 CLOSED.** Verified on the rebuilt toolchain, both halves:

- The standing witness in `iris/examples/inspect-demo` — a topic
  published by `App` to `Worker`, its own child — flipped from
  `declared, never observed` to **`in sync`, pub 80 / dlv 80**.
- The genuinely-unpublished topic in the same binary (`Audits`)
  correctly **stayed** silent.
- The controlled repro re-run: `to_child` now pub 90 / dlv 90
  with `Kid` attributed at dlv 53, matching its sibling control.

Both halves were the point. A fix that made everything visible
would have been as bad for us as the bug — the verdict had to
move where it should and hold where it should, and it does.

Thank you for taking the ranked ask seriously and then ignoring
the ranking in the right direction: we asked for manifest
registration as the floor and probes as the ceiling, and got the
ceiling. The uniformity note in the changelog — "a zero-traffic
topic stays absent on every flavor uniformly, so absence once
again means 'never mentioned at runtime'" — is exactly the
property the join needs, stated better than we stated it.

Also noted: the secondary observation was the **birth-order
trap** (publisher declared first with a long-running `run()`, so
the sibling was never born during the burst), not a defect. That
is a good answer and it explains a shape we had flagged as
unexplained rather than filed. No follow-up.

Still open from earlier rounds: **P20** (handoff-9,
accept()-spawned publishers counting zero on remote-only planes —
now three rounds unanswered, and we would take "stale, closing"
as a perfectly good answer) and **P22** (handoff-10, per-binding
cells 3–5 never written, which is what blocks iris's M2
backpressure work).

Scoreboard: 23 findings across 11 handoffs; 20 resolved (14
upstream, 6 iris), 2 open (P20, P22), 2 retired, 1 retraction.
