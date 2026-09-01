# The model

Most compilers have an IR that describes *how the program runs* —
blocks, instructions, registers. Hale has one of those too. This is
about the other one: a typed description of what the program **is**,
derived once and read by everything downstream.

## Derive a fact once

A semantic fact about a program — who calls whom, which handlers a
publish reaches, which locus owns which instance — has exactly one
right answer. So it gets derived in exactly one place, and every
consumer projects or queries that value.

That is the whole principle. In-process, it means `hale check`'s
verdict on a law and the verdict recorded in the topology artifact
are the same computation — not two computations held in agreement
by a test. `DispatchPlan`, which decides how a bus subject
dispatches, is derived from the same model that answered the
verification questions.

Out of process it is necessarily weaker, and worth being precise
about. The fleet tier, reasoning across binaries, cannot hold an
`ApplicationModel` — it decodes a deliberately smaller
`ComponentModel` from admitted artifact JSON. What it shares with
the single-binary checker is the *reachability engine* and the
*admitted contract*, not the value. That's the boundary, and it's
where the wire form would earn its keep.

Eight concepts stay distinct: source, plan, model, lowered law,
evidence, artifact, lowering plan, execution evidence. Keeping them
un-conflated is what lets each one be simple.

## Facts, and an account of what is missing

The interesting design choice isn't the entity tables. It's that a
model records **what it could not determine**, as data:

```rust
Hole {
    at:    Function(cache_lookup),
    kind:  IndirectCall,
    hides: CALLS | EFFECTS | COSTS,
    reason: "call through `f`",
}
```

A graph that stores only what it found cannot distinguish *there is
no such edge* from *I could not see*. Those are opposite answers to
every question worth asking. "Does this handler reach a syscall?" —
if the walk met a function pointer and stopped, the honest answer
is not "no".

So Hale's verdicts have four values rather than two:

| | |
|---|---|
| `holds` | proved |
| `violated` | disproved, and here is the counterexample |
| `uncertified` | well-formed, not provable here — the graph has an unknown |
| `invalid` | the statement itself is malformed |

`uncertified` fails the build, exactly like `violated`. The
difference is what you do about it: one is a bug in your program,
the other is a limit of what the compiler can see, and those are
different repairs.

The dual of a hole is a **capability** — a positive statement that
some family of facts *is* complete. Positive on purpose: absence of
recorded unknowns is not evidence of completeness, so a model that
never noticed a gap does not get to claim exactness by default.
Whether the model is adequate for a given question is then
answerable directly, rather than inferred from the absence of
warnings.

## What that makes sayable

Holding the whole arrangement in one typed value makes properties
checkable that a per-function analysis cannot express.

**Effects that travel over the bus.** A call graph stops at a
publish. Hale's message graph is declared over a closed topic set,
so the compiler continues across it: *this handler, by publishing
`orders`, can transitively cause a filesystem write in the audit
subscriber.* You state the causal set, and omitting one is the
error. A system whose message graph isn't declared cannot ask the
question at all.

**Influence arriving.** The backward direction is the more useful
one. A locus that subscribes only to `SumLookup` looks isolated
from `Recalled` in every declaration it carries — but if some third
locus subscribes to `Recalled` and republishes onto `SumLookup`,
the influence arrives anyway, and nothing in the depending locus's
source mentions it. `@effects(depends: {…})` is the complete set of
subjects that can reach you, and the diagnostic names the path.

**Amplification.** `@budget(fanout = 8)` bounds the subscriber
deliveries one call can cause — transitively, per message, over
runtime registrations. Publishing once to a 200-subscriber subject
is a fan-out of 200. That is a property of the arrangement, and no
per-function count reveals it.

Each of these is a statement about the whole system. They are
checkable because something holds the whole system.

## Counting things correctly

Bounds like `fanout` are quantitative, and quantitative analysis
over a graph is easy to get *approximately* right in ways that
matter. Two distinctions do most of the work.

**A set is not a quantity.** Reachability asks *can this happen*;
a budget asks *how many times*. A handler that calls one publishing
helper twice publishes twice — the reachable set says once. One
`subscribe` declaration on a locus arranged with `replicas = 3` is
three runtime registrations. And a message carries one key, so a
keyed publish reaches the subscriptions that key selects, not the
union of every subscription it might ever reach.

This is why call and publish rows in the model are **site-grained**:
two calls to one callee are two rows, because the callee runs
twice. Collapsing them into a set computes reachability where the
language means executions.

**Aggregate last.** Fan-out takes a maximum — over the key values a
site can produce, and over the conformers of one interface
dispatch. Taking that maximum early and continuing gives a
different number:

```text
max(immediate) + union(downstream)    ≠   max(immediate + downstream)
Σ over sites of max over alternatives ≠   max over alternatives of Σ
```

The left-hand forms pick one key for the immediate count and then
follow *every* key's downstream branch, or let two conformers both
contribute when one dispatch runs one. So a whole **scenario** —
one key, one conformer — is costed end to end, and only then
compared.

Neither point is exotic. Both are the kind of thing that reads as
obviously correct in one form and is quietly wrong in another,
which is precisely why the semantics live in a written contract
rather than in whichever walk happened to be written first.

## What it doesn't do

The model lives **inside the compiler process**. There is one way
to build one — deriving it from checked source — and no way to load
one from anywhere else.

The topology artifact is a *projection*: it carries the model half
under a shape hash, plus typed law, capability and adequacy
sections, and it is what every out-of-process consumer reads. It is
lossy by construction, and a model cannot be rebuilt from it.
Giving the IR a reversible wire form is real work rather than a
weekend: every table would have to serialise, provenance would have
to survive, and admission would have to be impossible to trick into
producing a model that no program denotes.

For now: in-process consumers query the model; everyone else reads
the artifact and checks the digests that tie it to the compiler
that produced it.

---

The contract — sorts, relations, the hole and capability laws, the
identity rules, and the discipline a new judgment family follows —
is in
[`spec/model.md`](https://github.com/hale-lang/hale/blob/main/spec/model.md).

For what an individual row holds, field by field, the API reference
is at [`/api/hale_model`](api/hale_model/index.html).
