# Building trellis-pair

This is the capstone. **trellis-pair** is the multi-binary
fitter/applier pair that the example ladder ends with — a
production-shaped Aperio program that exercises every substrate
primitive introduced over the previous eleven chapters.

The shape: a *fitter* binary fits parameters from an upstream
observation stream, ships stable parameters to an *applier* over
the bus, and each one runs in its own process under its own
scheduling regime. The two communicate only through declared bus
subjects, each subject typed against a shared schema compiled
into both binaries.

This chapter walks the program in full: the shared schema, the
fitter's source, the applier's source, the deployment
configuration, and how the substrate primitives compose into a
working production-shaped artifact.

## Repository layout

```text
examples/trellis-pair/
    shared.ap         // types compiled into both binaries
    fitter.ap         // the fitter binary's main
    applier.ap        // the applier binary's main
    deployment.yaml   // intended deployment-time config
    README.md
```

Each `.ap` file is built independently:

```bash
aperio build examples/trellis-pair/fitter.ap
aperio build examples/trellis-pair/applier.ap
```

Two ELF binaries land beside the source: `fitter` and
`applier`. They run as separate operating-system processes,
each opening its own
[lotus](../../reference/src/glossary.md#lotus).

## The shared schema (`shared.ap`)

The shared file declares the wire-level types that travel on
the bus, plus a perspective for the kernel:

```aperio
type Observation {
    value_low: Decimal;
    value_high: Decimal;
    timestamp: Time;
}

type Kernel {
    scale: Decimal;
    valid_after: Time;
    perspective_id: Int;
}

type Action {
    kind: String;
    magnitude: Decimal;
    quantity: Decimal;
    action_id: Int;
}

type Receipt {
    action_id: Int;
    receipt_value: Decimal;
    receipt_count: Decimal;
}

perspective KernelPerspective {
    params {
        kernel: Kernel;
        validation_count: Int = 0;
    }

    stable_when {
        return self.validation_count >= 3;
    }

    serialize_as Kernel;
}
```

Per [chapter 9](./09-cross-process.md), schema agreement is
*by compilation, not by runtime negotiation*. Both binaries
`import "trellis-pair/shared";`. Each compiles the same struct
layouts; the wire format is exactly that in-memory layout, so
deserialization is exact.

The `KernelPerspective` ([chapter 11](./11-perspectives.md))
wraps the wire-shaped `Kernel` with a `validation_count` and a
`stable_when` commit predicate ("ship only after at least three
perspectives agree"). The `serialize_as Kernel` annotation
declares that on the wire the perspective is a `Kernel` — the
`validation_count` is internal-to-the-fitter bookkeeping that
does not cross the bus.

## The fitter (`fitter.ap`)

The fitter's job: consume `Observation` messages, fit a kernel,
ship stable kernels.

```aperio
import "trellis-pair/shared";

locus FitterL {
    params {
        B: Int = 1000;
        c: Int = 10;
        sigma: Int = 1;
        phi: Float = 1.0;

        latest_kernel: Kernel = Kernel {
            scale: 1.0d,
            valid_after: `2026-01-01T00:00:00Z`,
            perspective_id: 0,
        };
        published_count: Int = 0;
        validation_count: Int = 0;
    }

    bus {
        subscribe "trellis.observation" as on_observation of type Observation;
        publish   "trellis.kernel"                        of type Kernel;
    }

    fn on_observation(obs: Observation) {
        // Update fitted kernel from the new observation.
        self.latest_kernel = Kernel {
            scale: self.latest_kernel.scale,
            valid_after: obs.timestamp,
            perspective_id: self.latest_kernel.perspective_id + 1,
        };
        self.validation_count = self.validation_count + 1;

        // Wrap as a perspective; ship if stable.
        let p = KernelPerspective {
            kernel: self.latest_kernel,
            validation_count: self.validation_count,
        };
        if p.is_stable() {
            "trellis.kernel" <- self.latest_kernel;
            self.published_count = self.published_count + 1;
        }
    }

    closure publish_keeps_pace {
        self.published_count ~~ self.validation_count - 2 within 1;
        epoch tick;
    }
}

fn main() {
    FitterL { };
}
```

The substrate primitives in play, all introduced earlier:

- **`params` block** with capacity parameters (`B`, `c`,
  `sigma`, `phi`) → `self.k_max` ([chapter
  5](./05-contracts-and-parents.md)).
- **`bus` block** with one subscription and one publication
  ([chapter 6](./06-the-bus.md)).
- **A bus handler** (`on_observation`) that mutates the locus's
  state and conditionally publishes ([chapter
  6](./06-the-bus.md)).
- **A perspective construction** wrapping a `Kernel` with a
  validation count, then `is_stable()` invoking the
  perspective's `stable_when` predicate ([chapter
  10](./11-perspectives.md)).
- **A closure** auditing that `published_count` keeps pace
  with `validation_count` (within a small tolerance for
  in-flight perspectives that have not yet hit the stability
  threshold) ([chapter 7](./07-closures.md)).

## The applier (`applier.ap`)

The applier's job: consume `Observation` messages, apply the
current kernel, emit `Action` messages, track `Receipt`
responses.

```aperio
import "trellis-pair/shared";

locus ApplierL {
    params {
        B: Int = 10000;
        c: Int = 1;
        sigma: Int = 1;
        phi: Float = 1.0;

        current_kernel: Kernel = Kernel {
            scale: 1.0d,
            valid_after: `2026-01-01T00:00:00Z`,
            perspective_id: 0,
        };
        actions_emitted: Int = 0;
        receipts_received: Int = 0;
        next_action_id: Int = 1;
        kernels_received: Int = 0;
    }

    bus {
        subscribe "trellis.observation" as on_observation of type Observation;
        subscribe "trellis.kernel"      as on_kernel      of type Kernel;
        subscribe "trellis.receipt"     as on_receipt     of type Receipt;
        publish   "trellis.action"                        of type Action;
    }

    fn on_observation(obs: Observation) {
        let a = Action {
            kind: "primary",
            magnitude: obs.value_low * self.current_kernel.scale,
            quantity: 1.0d,
            action_id: self.next_action_id,
        };
        self.next_action_id = self.next_action_id + 1;
        self.actions_emitted = self.actions_emitted + 1;
        "trellis.action" <- a;
    }

    fn on_kernel(k: Kernel) {
        self.current_kernel = k;
        self.kernels_received = self.kernels_received + 1;
    }

    fn on_receipt(r: Receipt) {
        self.receipts_received = self.receipts_received + 1;
    }

    closure action_receipt_balance {
        self.actions_emitted ~~ self.receipts_received within 5;
        epoch dissolve;
    }
}

fn main() {
    ApplierL { };
}
```

A larger surface than the fitter, exercising:

- **Three subscriptions and one publication.** The applier
  consumes `Observation` (upstream stream), `Kernel` (fitter
  output), and `Receipt` (downstream response); it produces
  `Action` (its own outputs).
- **Hot-loading kernels.** `on_kernel` replaces
  `self.current_kernel` atomically. The next `on_observation`
  invocation reads the new kernel; the swap is torn-read-free
  because bus dispatch in v0 is cooperatively scheduled (per
  the runtime spec).
- **An at-dissolve closure** auditing that every emitted
  `Action` eventually became a `Receipt` (within a tolerance of
  5 for any in-flight actions).

## The deployment

The trellis-pair's `deployment.yaml` shows the *intended*
production transport binding:

```yaml
channels:
  "trellis.observation":
    transport: udp_multicast
    group: "239.7.7.7"
    port: 9000

  "trellis.kernel":
    transport: nats
    url: "nats://nats-control:4222"

  "trellis.action":
    transport: nats
    url: "nats://nats-control:4222"

  "trellis.receipt":
    transport: nats
    url: "nats://nats-control:4222"
```

Each subject is bound to a transport appropriate to its
traffic shape:

- **`trellis.observation`** — UDP multicast. High-frequency,
  lossy-acceptable, broadcast to many subscribers (every
  applier on the same group receives the same observation
  stream).
- **`trellis.kernel`** — NATS. Slow cadence, reliable
  delivery, ordered per fitter. Kernels need to arrive,
  and they need to arrive in order.
- **`trellis.action`** / **`trellis.receipt`** — NATS.
  Control-plane messaging with the downstream sink.

For local testing, the YAML supports a wildcard swap:

```yaml
channels:
  "trellis.*":
    transport: in_memory
```

This binds every `trellis.*` subject to the in-memory
transport — the same source code, bound differently for a
single-process integration test.

> **v0 caveat.** The YAML form is the *intended* future
> surface. v0's actual cross-process bus consumes the simpler
> `LOTUS_BUS_CONFIG` line format from chapter 9.
> `deployment.yaml` parsing and richer transport selection
> are v1.x roadmap items.

## What the program does

Putting the parts together, the trellis-pair pipeline:

1. **Observations flow in.** A separate process (an upstream
   observation source) publishes `Observation` messages on
   `trellis.observation` over UDP multicast. Both the fitter
   and the applier subscribe.
2. **The fitter fits.** Each `Observation` arrival updates the
   fitter's `latest_kernel` and increments `validation_count`.
   When `validation_count >= 3`, the fitter publishes the
   current `Kernel` on `trellis.kernel` over NATS.
3. **The applier applies.** Each `Observation` arrival
   multiplies `value_low` by `current_kernel.scale`, packages
   an `Action`, and publishes on `trellis.action`. Each
   `Kernel` arrival hot-loads `self.current_kernel`. Each
   `Receipt` arrival increments `receipts_received`.
4. **The closures audit.** The fitter's `publish_keeps_pace`
   fires at every tick and complains if publishes drift beyond
   expected lag from validations. The applier's
   `action_receipt_balance` fires at dissolve and complains if
   actions and receipts diverge beyond an in-flight tolerance
   of 5.

Each binary's lotus is independently lifecycle-managed:
`birth` runs once when each process starts; `run` is implicit
(the bus subscriptions keep the locus alive); `drain` and
`dissolve` fire when the process receives a shutdown signal
(SIGINT in the v0 substrate).

## What the substrate enforces

The trellis-pair is small in lines but exercises the full
substrate-up stance the language was built for. A reader
familiar with the previous eleven chapters can verify the
following at the source level:

- **No leaked allocations.** Every `Action`, every
  intermediate string, every `Observation` copy lives in its
  locus's arena and is freed when the locus dissolves. There
  is no available concept of escape across the boundary.
- **No lateral failure routing.** A `ClosureViolation` on the
  applier's `action_receipt_balance` reaches `ApplierL`'s
  `on_failure` (which is unhandled in this version, so the
  process exits non-zero with the violation report).
  Sibling-to-sibling absorption is structurally impossible.
- **Schema agreement by compilation.** Both binaries compile
  the same `shared.ap`. There is no schema document to
  maintain separately; if the schema changes, both binaries
  recompile from the same source, and the deployment is a
  single rolling update.
- **Hot-loaded perspectives, not patched code.** When the
  fitter's understanding of the upstream stream changes, it
  ships a new `Kernel`. The applier swaps it in atomically.
  No code reload, no applier restart, no special "config
  refresh" mechanism — the kernel is the value the system
  was built around.

## What v0 does not yet do

A few production-relevant pieces are roadmap, not v0:

- **`p.is_stable()` as a method.** The fitter calls
  `p.is_stable()` on the perspective; for v0 the substrate
  treats the `stable_when` block as the body of an
  `is_stable()` method. Generalizing perspective methods
  beyond `stable_when` is post-v1.
- **Multi-perspective fitting.** Holding several candidate
  perspectives in flight, deduplicating equivalent ones,
  applying `stable_when` across the population — this is
  *application-level* code today; substrate helpers will
  land in v1.x.
- **`serialize_as TypeV1` rolling deployments.** Schema
  evolution with mixed-version producer/consumer pairs
  during a deployment window. Open-question #13;
  implemented when a workload demands it.

## Where to go next

Beyond this chapter:

- **The reference.** The
  [Aperio Reference](../../reference/book/index.html) covers
  every construct in the language with formal grammar and
  semantics. Reach for it when a question this book left
  imprecise comes up.
- **The standard library.** The [Aperio Standard
  Library](../../std/book/index.html) catalogs the
  batteries — I/O, HTTP, text processing, the test
  framework — that overlay the substrate. (Phases 1–5 of
  the v1.x roadmap; many libraries are in active
  development.)
- **The example ladder.** `examples/` in the repository is
  the fifty-rung ladder this book has drawn from. Each rung
  is a runnable Aperio program with an annotated `main.ap`
  and a `README.md` walk-through.

You have read the substrate-up tour. Aperio's promise is
that programs of any shape, written against this substrate,
behave the way the substrate's invariants guarantee — by
construction, with the compiler enforcing the rules and
the runtime upholding them.

Open the wand. Cast.
