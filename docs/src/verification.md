# Verification

Most languages ask you to *write* correct concurrent code and hope you
did. Hale takes a different bet: make incorrect *designs* fail to
compile, and model-check the runtime everything executes on. This page
is the honest account of what that buys you — and what it deliberately
doesn't.

## The substrate is model-checked

Hale's runtime, **lotus**, is C: pthreads and C11 atomics. Every
primitive in it with a cross-thread surface is transcribed into a model
and checked **exhaustively, under every legal interleaving**, with
[GenMC](https://github.com/MPI-SWS/genmc) — as a standing CI gate. A
race, use-after-free, or assertion failure in any model fails the build.

| Primitive | What's verified |
|---|---|
| Lock-free hashmap | the enter / drain / grow protocol |
| Mailbox monitor | the pinned-locus mutex hand-off |
| Bus queue | the cooperative-pool conditional lock |
| Arena subregion lock | the parent's child-slot freelist |

Each model carries a **negative control**: delete the synchronization
and GenMC reports the exact bug the real code prevents — proof the
check has teeth. (The per-thread chunk pool needs no model: it is
`__thread`, with no cross-thread surface.) Sanitizers catch races on
the paths your tests happen to hit; model checking catches the ones no
test reliably triggers — grow-during-drain, compact-then-grow. For a
language whose whole concurrency story is the bus, trusting the
substrate is the foundation everything else rests on.

### What the models do not establish

Note the word *transcribed*. Each model is a hand-written rendering of
one protocol's atomics, not the production C, and **nothing checks the
two against each other**. A model that still passes proves a property
of the runtime as it stood when the model was written. Correspondence
is maintained by hand and is part of the trusted base.

That has consequences worth stating rather than burying:

- Individual atomics and code paths added since a model was written are
  not in it until someone transcribes them.
- One model (the hashmap iterator) verifies clean under sequential
  consistency and reports a use-after-free under GenMC's default
  release-acquire model — the one that corresponds to the runtime's
  actual orderings. That is a gap in the model-level justification, not
  a demonstrated bug in the runtime, and it is open.
- The CI gate is conditional: a prose-only diff skips the model
  checker, on the reasoning that no sentence in a `.md` alters a memory
  ordering.

[`verification/README.md`](https://github.com/hale-lang/hale/blob/main/verification/README.md)
in the compiler repo is the audit of record, including a dated
inventory of exactly which surfaces have drifted. It is kept honest
because it is the document we read before trusting a result.

## Your programs are data-race-free by design

Above the substrate, the language is shaped so application code can't
introduce a data race in the first place:

- **A typed bus instead of shared state.** Loci talk by publishing
  typed values to topics; the payload is *copied* into the receiver's
  region. There is no shared mutable cell to race on.
- **The single-threaded-method invariant.** Calling a locus's method
  from the wrong pool's thread is a compile error.
- **Vertical-only failure.** No lateral references between siblings; a
  failure travels up to a parent's `on_failure`, never sideways.

## Checked at build time

These run during `hale check` / `hale build`, on top of ordinary
type-checking.

**Bus-graph properties.** The bus topology is a typed graph, and the
compiler walks it. This is the analysis that is **on by default** and
fails the build:

- *orphan* topics (wired to only one end) — warning
- *cross-locus cycles* that can spin — warning
- *intra-locus re-entrant* self-publish (unbounded recursion) — **error**
- *backpressure* — an unthrottled publish in an unbounded loop — warning
- *subject type-mismatch* — two sites disagreeing on a payload type — **error**

**Design rules**, enforced as errors:

- **No locus-return** — a method may not hand back a managed locus (a
  Law-of-Demeter / CQRS / dependency-inversion violation caught in one
  rule).
- **Codec purity** — a bus codec's `encode` / `decode` must be pure;
  they may run off-thread.
- **`ring_layout` conformance** — a foreign shared-memory ring layout
  is checked for internal and cross-field consistency before a torn
  read is possible.

**Concurrency & placement**, keeping a program's placement coherent
with how the runtime dispatches:

- **Dead bus receiver** — a cooperative locus that subscribes to the
  bus *and* blocks in `run()`, so the blocking call monopolizes the
  pool thread and its handlers never fire — **error**.
- **Blocking call on a cooperative pool** — a blocking `run()`
  (`recv` / `accept` / `process::run`) on a pool that isn't
  `where async_io`; it holds the pool's thread and stalls
  co-scheduled loci — warning.
- **Cooperative pool starvation** — two or more loci on one
  cooperative pool whose `run()` bodies statically never return
  (`while true`, `while !self.draining`, a never-assigned flag): the
  pool runs each `run()` to completion in birth order, so the later
  ones never start — including the main locus's own `run()` when a
  forever-looping locus shares pool `main`. Bus handlers keep firing
  at sleep/yield drains, which makes the hang look like a healthy
  idle; the warning names every offender — warning.
- **Nested long-running child** — a non-`main` locus holding a
  params field of a locus type whose `run()` never returns; the fix
  is hoisting it to a `main` sibling with its own placement —
  **error**.
- **Unowned subscriber locus** — a bus-subscribing locus
  instantiated *non-owned* in another locus's method body, so it
  dissolves at scope exit before its subscription can fire —
  **error**.

**Memory-bound proofs** *(on by default).* Every `hale check` /
`hale build` runs the whole-program survey: the compiler's
escape/loop dataflow flags allocations that escape a per-message
handler or unbounded loop and **accumulate until the locus
dissolves** — with loop-ranking that *proves* a `while v < N`
counter bounded. Run-to-exit programs (a `main` with no `run` loop
and no bus handler) warn nothing — a script owes no bound proof.
`@unbounded fn` is the in-source carve-out for an acknowledged
site; `--no-warn-unbounded-alloc` opts a run out. Advisory today; a
hard error contract is the intended end state once the remaining
documented false-positive classes get their annotations. A separate
advisory also flags two **loop-scoped hot-path allocations** — a locus
or `BytesBuilder` instantiated per iteration, and an allocating `recv`
in a loop — steering toward a hoisted field / `recv_into`.

**Hot-path allocation budget** *(opt-in, hard error).* `@budget(alloc_per_call
= N)` on a fn is the dual of `@unbounded`: an explicit per-call
allocation ceiling the compiler enforces. It counts the arena
allocations it can see — literals, `@form` inserts, transitively
through resolved callees, plus the known-allocating `recv` family — and
**fails the build** if the fn allocates more than `N` per call (a
loop-nested allocation is unbounded per call). `N = 0` is the zero-alloc
certificate for a hot-path handler. The one allocation check that gates
the build, because you opted into it.

**Resource budgets** *(opt-in).* Static counts of file descriptors, OS
threads, cooperative pools, and bus subjects, with a
`--check-resource-budget budget.toml` ceiling gate for CI and fd-leak
detection.

## Effects you forbid, proven absent

The checks above run whether you ask for them or not. **Effect
assertions** are the opt-in half: you name something a function must
never do — reach a syscall, block, read the clock, allocate — and the
compiler proves it over the call graph, naming the path to any
violation.

```hale,fragment
@no_syscall @deterministic
fn price(book: OrderBook, qty: Int) -> Decimal { ... }
```

Three properties make them worth relying on rather than decorating
with:

- **The proof follows the call graph**, including calls made on a
  handle (`reader.slurp()`), across seed boundaries into imported
  libraries, and through interface-typed slots into whatever concrete
  implementation is bound.
- **Incompleteness fails closed.** A stdlib call the registry cannot
  classify — or does not know at all — is treated as may-do-anything
  and violates the assertion, rather than being quietly assumed pure.
- **The compiler reports what it infers**, not only what you declared,
  as a diffable artifact — so a handler that silently starts doing
  filesystem I/O is a one-line review diff even though nothing
  annotated changed.

They are taught in full in **[Effects & contracts](./effects.md)**.


## Claims: the law of the assembled system

Effect assertions attach to one function. A **claim** quantifies over
the whole assembled program: a named sentence, declared on the main
locus, checked at every `hale check`. The canonical use is isolation
— "nothing in wing A reaches wing B" — stated once, with a name a
code review or a compliance document can cite:

```hale
type Task { id: Int; }
type Metric { n: Int; }
topic Tasks   { payload: Task; }
topic Metrics { payload: Metric; }

locus DeltaTriage {
    params { seen: Int = 0; }
    bus { subscribe Tasks as on_task; publish Metrics; }
    fn on_task(t: Task) {
        self.seen = self.seen + 1;
        Metrics <- Metric { n: t.id };
    }
}

locus GammaResearch {
    params { total: Int = 0; }
    bus { subscribe Metrics as on_metric; }
    fn on_metric(m: Metric) { self.total = self.total + m.n; }
}

group delta_wing = { DeltaTriage };
group gamma_wing = { GammaResearch };

main locus Org {
    params {
        triage: DeltaTriage = DeltaTriage { };
        research: GammaResearch = GammaResearch { };
    }
    claims {
        iso_dg: forbid reaches(delta_wing, gamma_wing);
    }
}

fn main() { Org { }; }
```

This program **fails to check**: the metrics publish crosses the
boundary, and the violation names the claim and the full path —

```text
claim `iso_dg` violated: `delta_wing` reaches `gamma_wing` —
witness: `DeltaTriage::on_task` -(publishes "Metrics")-> `GammaResearch::on_metric`
```

A `group` is declared vocabulary: it names loci and functions
(including imported ones — `delta::*` enumerates a whole imported
seed). An unknown name in a group is an error, never a silently
empty set, and an empty group is an error unless it says
`may_be_empty` — a rule that exists because a `forbid` over an empty
set holds vacuously, which is a fail-open wearing formal clothing.

`forbid reaches(A, B)` follows both call edges and bus edges by
default; `via { calls }` or `via { bus }` restricts the relation.
The target may also be an effect class — `forbid
reaches(quote_api, effects(money))` says the quote path can never
reach anything classified as moving money.

Claims are **errors**, not warnings, and deleting or weakening one
is a visible source diff — which is exactly the review event the
feature exists to create.

Beyond `forbid reaches`, the claim family covers the properties
real topologies ask for:

```hale,fragment
claims {
    // The boundary itself: every direct edge must match a grant.
    grant: only edges gamma_wing -> delta_wing {
        publish t::ResearchDigest;
    };
    // Cost law: at most one model call per task, on any path.
    one_call: bound llm <= 1 on paths from planners;
    // Wiring the org must actually have.
    wired: require subscribes(some delta_wing, topic t::Tasks);
    no_orphans: cover topic in seed(t): subscribed_by(some staff);
    single: count publishers(topic t::Tasks) == 1;
    // The quiet-boot and interposition forms.
    quiet: forbid reaches(delta_wing, gamma_wing) during birth;
    gated: forbid reaches(intake, execute) avoiding permission_gate;
}
```

Effect classes can also be **indexed families** — `domain wing = {
delta, gamma }; effect knowledge(wing);` — so per-tenant capability
contracts (`@effects(only: {knowledge(delta), llm})`) don't need
N² boilerplate, and `knowledge(*)` covers every index. And the
whole checked model exports as the **topology artifact**: `hale
check <t> --dump-topology` emits the sorts, relations, and every
claim's result under a `shape_hash`, and `--check-topology
<baseline>` fails CI when the topology or the law changes without
review.

## Secrets: confine, classify, claim

A signing key is not a problem for a taint checker to solve. It's a
problem the ownership model already solves, once you close one gap.

`@sealed` on a locus makes its `params` readable **only from inside its
own methods**. Others can still call it — that's the point — they just
can't read its state:

```hale,fragment
@sealed locus Signer {
    params { key: Bytes; }

    @effects(is: { secret_use })
    fn sign(m: Bytes) -> Signature { … }
}
```

Loci are not otherwise field-encapsulated: `self.signer.key` typechecks
from anywhere holding one. `@sealed` makes the only way in a method
call — and that method carries an effect class, so every path that can
touch the key is visible on the call graph.

Now the law is two ordinary claims:

```hale,fragment
claims {
    no_plugin_secrets: forbid reaches(plugins, effects(secret_use));
    one_op_per_request: bound secret_use <= 1 on paths from handlers;
}
```

Wire a `Signer` into a plugin and the build stops with the crossing
call named:

```text
claim `no_plugin_secrets` violated: `plugins` reaches
  `effects(secret_use)` — witness: `PluginHost::sneaky` -> `Signer::sign`
```

The shape that falls out: an ordinary function prepares a request from
public data and hands back a plan; the sealed locus interprets the plan
and performs the one privileged step. The planner never receives the
key, a handle, or anything that could produce one — so there is nothing
for it to leak, whatever it does.

**You usually don't have to write the sealed locus.** `std::secret`
ships one. Sealing protects reads and writes but not construction — a
parent writing `Signer { key: … }` holds the key to pass it — so these
take the *name of a source* instead:

```hale,fragment
locus Gateway {
    params {
        s: std::secret::Signer =
            std::secret::Signer { env_var: "SIGNING_KEY" };
    }
    fn go(m: Bytes) -> Bytes { return self.s.sign(m); }
}
```

The key is read during `birth`, so it exists only inside a sealed locus
from the moment it enters the program — there is no line anywhere in
your code where you hold it. `self.s.key` is a compile error naming the
methods you can call instead. `std::secret::Credential` is the same for
a token or password, plus a `fingerprint()` for logs.

When the source is encoded — many venues issue an HMAC secret in
base64 — name the encoding with `decode:` (`Signer { env_var: "…",
decode: "base64" }`), so the key is the *decoded* bytes rather than the
text of the base64. Undecodable input, or an unrecognized transform,
fails closed: an empty key and `ready() == false`, never a key that
isn't the one the source names.

Two more claims worth knowing, because they answer different questions
about the same boundary. **Where** may the program touch the OS?

```hale,fragment
claims {
    all_io_is_gated: forbid reaches(app, effects(syscall)) avoiding safe_io;
}
```

Every syscall path must pass through the vetted component. And,
separately, **what is each crossing for?**

```hale,fragment
effect audit;

claims {
    io_attributed: require attributed(all syscall);
}
```

(`secret_use` above needs no `effect` declaration — it is a compiler
built-in, so every program means the same class by it.)

Every fn that actually touches the OS must name a purpose with a user
effect class. Where the checker cannot see — a fn making an indirect
or opaque call, naming no purpose of its own — the claim is
**uncertified** rather than quietly satisfied: unknown is not the
same as clean. Classify the caller with `@effects(is: {...})`, or
bind the callee so it resolves. These are independent: I/O can be perfectly gated and
still tell you nothing about why any given write happened, or be
scattered across forty loci while each one says exactly what it's for.
The second also covers code nobody has written yet — it quantifies over
the whole program rather than a named group.

If a whole group of loci should be confined, say so once and let the
compiler watch for the one someone forgets:

```hale,fragment
group vaults = { Signer, TokenStore };

claims {
    vault_confined: require sealed(all vaults);
}
```

**What this is and isn't.** The secret lives in a locus that owns it,
your code cannot obtain it, the operations on it are classified, and
your claims constrain who reaches them. That is confinement, not
information flow: a signature *derived* from the key is not tracked, a
constant-time compare still lets the verdict be published, and the
sealed locus's own body is trusted — keep it small enough to read.

`@secret` on a parameter is a **lint**, not a proof. It flags a secret
reaching a publish or log in the same body, follows no calls, and
tracks no aliases. `hale check --strict-secret` widens the walk and
reports `uncertified` wherever it can't follow, which is loud by
design.

**Adopting `@sealed` on an existing codebase** is a question you don't
have to answer by reading. `hale check --sealable` tells you:

```text
sealability: 4 of 5 loci can be `@sealed` today

  free to seal (nothing outside touches their params):
    Already, App, Holder, Private

  would break callers:
    Exposed — 1 external access(es): Exposed.k
```

Empty means sealing that locus is a no-op. In practice most loci
already qualify: across this repo's own corpus, 148 of 151. The ones
that don't share a shape — a parent reading a child's *result* field
instead of calling a method, which the no-locus-return rule already
discourages.

## Invariants you declare, checked as it runs

The checks above are the compiler's. You can add your own with a
**closure** — a property a locus promises to keep, written as a
first-class block and audited by the runtime while the program runs:

```hale,fragment
closure balanced {
    self.debits ~~ self.credits within 0.01d;
}
```

`~~` is "approximately equal, within tolerance." When the invariant
breaks, it routes to the owner's failure handler (or, unhandled, stops
the program) — a declared property enforced by the substrate, not a
comment you hope holds. Closures are taught in full in
[When things fail](./services/failure.md#declaring-an-invariant-closure);
they're the runtime half of "verified by construction."

## What Hale does *not* claim

Hale is **not** a whole-program functional-correctness prover — that is
the world of CakeML and F\*. The guarantee here is narrower and
deliberately so: the **coordination** (the bus graph), the **substrate**
(the concurrent primitives), and **bounded resource use** are verified,
because those are the properties that must hold no matter what executes
the design — native, wasm, or a future target. Verification that
survives a change of substrate is the kind worth building on.

Nor does it prove arbitrary application correctness, make a lossy
transport reliable, or remove the need to choose sensible bounds and a
recovery policy. It makes those choices visible, checkable, and
attributable to the architecture that owns them.

### The analysis boundary

Three properties of the analysis are worth stating precisely, because
each one shapes how a result should be read.

**The checks are sound, not complete.** Every analysis here is a
conservative over-approximation. When the compiler cannot resolve
something it treats it as may-do-anything and the contract fails, rather
than assuming the best and passing. That direction is deliberate, and it
has a cost worth naming: a rejected program is sometimes a limit of the
analysis rather than a defect in the code.

**Closed-world reasoning has an edge.** The guarantees derive from a
closed call graph, a closed topic set, and a classified standard
library. Influence that leaves that world — through `@ffi`, through a
transport to another binary, through the operating system — is outside
what the compiler can follow, and the contracts say so rather than
pretending otherwise.

**Cost models are source-level bounds.** Effect classes, publish sets,
and causal sets are checked facts. The quantitative budgets are measured
against what the compiler can see before the backend has finished
deciding layout and inlining, so read those numbers as bounds on the
program you wrote rather than on the instructions that ultimately run.

> The authoritative, exhaustive catalog of every compile-time check is
> [`spec/verification.md`](https://github.com/hale-lang/hale/blob/main/spec/verification.md).
> The verification roadmap that drove this work — now delivered — is
> [GitHub issue #18](https://github.com/hale-lang/hale/issues/18).
