# claims-demo — witnessed law on the flower

Two seeds that turn every state of iris's **witnessed-law** view
into a live picture (INSPECTOR.md § *Witnessed law*). The app
declares four claims that all HOLD statically; the point is what
observation adds.

```
# 1. build both, cut the app's artifact (the law document)
(cd app  && hale build . && hale check . --dump-topology=/tmp/cd.topo.json)
(cd rogue && hale build .)

# 2. run the app under observation, attach fuse-hl WITH the artifact
(cd app && LOTUS_OBS=1 ./app &)
../../consumer/fuse-hl/fuse-hl 8787 ../../render/web /tmp/cd.topo.json &

# 3. open http://127.0.0.1:8787 and press [3] for the law view.
#    Then start the rogue and watch single_writer flip:
(cd rogue && LOTUS_OBS=1 ./rogue &)
```

## What each claim shows

| claim | static | with the app alone | with the rogue too |
|---|---|---|---|
| `single_writer` (count == 1) | holds | **consistent** — 1 writer seen | **CONTRADICTED** — 2 writers fused |
| `consumed` (require subscribes) | holds | **consistent** — delivery seen | consistent |
| `audited` (require subscribes) | holds | **not exercised** — wired, never sent | not exercised |
| `iso` (forbid reaches) | holds | **not exercised** until both wings run, then **consistent** (one-hop basis) | consistent |

The headline is `single_writer`. It counts **declared** publishers
**in one seed**, and hale refuses on principle to look across a
deployment ("nothing about a deployment is visible from source").
`rogue/` is a separate binary publishing the same wire subject +
payload shape. Both check green; the fused fleet has two writers.
iris joins on `(subject, shape)`, sees both, and reports **static
holds / witnessed contradicted** — with both writers ringed red on
the canvas. Only an observer under the running fleet can produce
that verdict.

`audited` is the quiet one: the wiring is complete (a real
publisher declaration and a real subscriber) but nothing ever
sends an `Audit`, so it reads **not exercised** — law that no
traffic has pressured. That is *coverage for claims*, the analogue
of an assertion that never ran.

## The honest asymmetry

iris can **falsify** a claim (a second writer exists; a forbidden
route fired) and can report **not exercised**. It can never
*prove* a universal — a `forbid` holding is only ever "no
counterexample observed on the paths taken", which is why its
consistent verdict always names its basis (one-hop; multi-hop
chains wait for M3). Observation counts **live instances**; the
compiler counts **declared loci**. The ledger always says which.
