# Iris: the embedded observer

`LOTUS_OBS=1` makes any Hale binary publish an observation
segment (see [Operations & debugging](./operations.md)). **Iris**
is the program that reads it: it attaches to every publishing
process on the machine, fuses them into one system graph, and
serves it over HTTP with a live browser view. It ships inside
the `hale` binary, so there is nothing to install:

```sh
hale run --observe path/to/prog.hl      # program + observer, one command
# … in another terminal, or a browser:
open http://127.0.0.1:8787
```

`--observe` sets `LOTUS_OBS=1` on the program and launches
`hale iris` beside it for the program's lifetime. Nothing reaches
the program's own argv; the flag is consumed by `hale run`.

## The commands

```
hale iris [port] [artifact.json]        attach + fuse, serve :port (default 8787)
  --diff <a.topology> <b.topology>      … with the review view over the pair
  --diff <diff.json>                    … over a ready `hale model diff` document
  --membrane <dir>                      … publishing verdicts and intent to an organism's sockets
hale iris inspect <artifact.json> [url] artifact-side inspector
hale iris --where                       print the cache directory
hale iris --build-only                  materialize + build, print the binary
```

**`hale iris`** attaches to every `LOTUS_OBS=1` process it can
see (the registration files under `$XDG_RUNTIME_DIR/hale/` or
`/tmp/hale-obs/`), fuses their rings into one graph, and serves:

- `/` — the live view (locus tower, message topology, transport
  edges with throughput / depth / latency / loss, supervision
  events as they fire).
- `/snapshot` — the fused graph as JSON, one document.
- `/events` — the same as a server-sent-event stream.

Pass a `--dump-topology` artifact as the second argument and
iris overlays what the compiler *declared* on what the runtime
*does*: declared-but-silent topics, undeclared traffic, drift.

**The review view.** Give iris a change instead of a snapshot and
it renders the change over the running fleet (key `4`):

```sh
hale check . --dump-topology=before.topology      # before the edit
hale check . --dump-topology=after.topology       # after
hale iris --diff before.topology after.topology   # or: --diff diff.json
```

`--diff a b` runs `hale model diff` in-process and hands the
document to the observer; `--diff doc.json` hands over one you
already have. The view lists the semantic rows (`+ locus
EmailIntake`, `! fn Intake::on_mail gains publish`, `! locus
Support subscribes: +Tickets`, `! claim iso: holds → violated`),
and, because every observed process reports the model hash it was
built from, it says which live processes are still expressing the
old artifact: they are listed as stale and ringed on the canvas.
With a pair and no artifact argument, the `after` side is also the
law artifact.

**The membrane.** An organism built on the DNA core binds two typed
control topics on unix sockets (`dna/organism/main.hl` is the
reference): `dna.review.verdict` and `dna.intent.offered`. Point
iris at them and the browser can answer a pending Review or offer
intent (key `m`):

```sh
hale build dna/organism && ./dna/organism/organism &   # listens under /tmp
hale iris --membrane /tmp
```

`--membrane <dir>` routes the observer's publishes of those two
topics to `<dir>/hale-dna.review.verdict.sock` and
`<dir>/hale-dna.intent.offered.sock` (an env-configured connect
route; the organism must already be listening). The panel posts to
`/ctl/review` and `/ctl/intent`, which publish the DNA core's own
`Verdict` and `IntentOffer` declarations. Nothing is decided in
iris: the organism's Review checks the candidate digest, the
reviewer's authority and independence exactly as it does for a
local verdict, and the intent goes through the membrane gate and
the Metabolism. The snapshot reports how many of each were
published; the organism reports what it decided.

**The organism.** `hale iris --organism <status.json>` adds a third
source beside the segment and the artifact: a DNA organism's status
projection (see [DNA](../dna/run.md)). Perspective [5] renders it and
tints the lineage tower on the canvas. `hale dna run` wires all of
this up for you.

**`hale iris inspect`** is the artifact-side half of that
comparison: point it at an artifact (and optionally a running
`hale iris` URL) and it reports drift, declared-but-silent
surface, and the law rows, without a browser.

## How it ships

Iris is itself a Hale program (a `fuse-hl` consumer, a C attach
shim over the shared-memory segment, and a vanilla-JS renderer).
Rather than rewrite it in Rust, the `hale` binary carries its
*sources* the way it carries the standard library: embedded at
compile time, materialized on first use into

```
$XDG_CACHE_HOME/hale/iris/<toolchain-hash>/     (or ~/.cache/…)
```

and built there with the very compiler you are running. The
hash covers the compiler version and every embedded byte, so a
compiler upgrade rebuilds the observer once and a second launch
is exec-only (`hale iris --where` prints the directory; delete
it to force a rebuild). The observer is compiled by the same
codegen that compiled the observed program, so the wire layout
(`crates/hale-codegen/runtime/obs_protocol.h`, pinned by
`_Static_assert`s in the runtime and by a decoder test) can
never drift between the two halves of one installation.

The observer needs the same C toolchain `hale build` needs.
`hale iris --build-only` is the way to pay that cost ahead of
time (in a container image, say).

## What iris does *not* do

It does not schedule, replay, or edit. Attaching is
read-only: the segment's `observer_count` goes up, the emitting
process starts writing records instead of just counters, and
that is the whole footprint. Record a run with
`LOTUS_OBS_RECORD=<path>` and [replay](./replay.md) it later
under the same observer.
