# dna — friction log

Gaps met while building the DNA Phase 0 domain proof (GH #526)
against today's Hale. Format follows iris / pond / brained: one entry
per gap — tag, severity, what happened, the minimized reproducer, the
workaround in `dna/core`, and the resolution once there is one.
Entries are never removed when a gap closes; they get a "Resolved"
line, because tomorrow's reader needs to know which workaround in the
source dates from which era.

A language REQUEST leaves this file only when the reproducer shows the
invariant cannot be expressed with loci, types, interfaces,
perspectives, topics and claims, and the proposed primitive's
semantics are specified (#521 "prove each before adding syntax").
Compiler BUGS found on the way are fixed upstream and noted here.

---

## F.1 — `match` on an enum declared in an imported seed

**Tag:** `import-enum-match`
**Severity:** blocking for any library that matches its own enum.
**Status:** open (compiler bug); workaround in `dna/core/types.hl`.

`dna/core/types.hl` declared `type Disposition = enum { Release,
Stage, Review, Escalate, Deny }` and a `match` over it. `hale check
dna/core` is clean. Any importer of the seed fails:

```
type error: match is not exhaustive; add a `_` arm or cover all
cases of `__lib_dna_core_types_Disposition`
```

so the importer's exhaustiveness pass does not see the mangled enum's
variants. Adding the `_` arm moves the failure to codegen:

```
codegen error: unsupported in codegen v0: constructor pattern:
unknown enum `Disposition`
```

**Reproducer:** `dna/friction/f1-import-enum-match/` — `lib/` +
`app/` (checker refusal), `lib2/` + `app2/` (with `_` arm: check
passes, `hale build` fails). `hale check lib` alone is clean.

**Workaround:** dispositions are `String` constants (`DISP_*`) in
`dna/core/types.hl`. The enum is the shape we want back; strings cross
the seed boundary today.

**Resolution:** pending. Two distinct bugs: (1) exhaustiveness over a
path-renamed enum; (2) codegen's constructor-pattern arm resolves the
enum by bare name after the import prefix was applied.

## F.2 — `@unbounded` did not acknowledge the hot-path advisory

**Tag:** `unbounded-not-honored-by-hot-path-lint`
**Severity:** blocks `hale verify` (the discipline gate) on any
param-bounded fan-out loop.
**Status:** FIXED upstream (hale, 2026-09-05, this track).

Every hot-path advisory ends with "or acknowledge an intentional
shape with `@unbounded` on the enclosing fn/hook". `Workflow::run`
births one `Step` per `steps` and `Work::run` one `Attempt` per try;
both carried `@unbounded` and the advisory still fired, so `hale
verify dna/core` stayed at 2 findings. `check_hot_path_alloc` in
`crates/hale-types/src/check.rs` walked lifecycle hooks with a
hard-coded `false` and fns with only `hot`; the flag the message named
was never read.

**Fix:** `HotPathCx` carries `unbounded`; `emit` skips the advisory
(never the `@hot` error) when set. Regression tests in
`crates/hale-types/tests/hot_path_alloc.rs`.

**Note:** `@hot` and `@unbounded` cannot stack (the parser admits only
`@budget` after `@hot`), which is fine — a hot fn that allocates
unboundedly is a contradiction, not an acknowledgement.

## F.3 — an interface-typed VALUE cannot flow into an interface-typed field

**Tag:** `interface-value-into-interface-field`
**Severity:** shapes the whole performer design.
**Status:** open (language gap, or a missing identity coercion).

`Work` holds `performer: Performer` (assembly-substitutable) and
wanted to hand it to each `Attempt { performer: self.performer }`.
Refused:

```
type error: type `Performer` cannot satisfy interface `Performer`
— only loci satisfy interfaces
```

Only a concrete locus literal coerces into an interface-typed field.
An interface-typed value is a fat pointer that already IS the field's
type, so the identity case is simply missing from the coercion table
(`spec/types.md` lists eight positions; "interface → same interface"
is not one). Even if it were admitted, the ownership question is
open: the field and the frame that produced the value would both
claim the impl (the same ambiguity the locus-typed-field guard
rejects, which returns early for interfaces — `check.rs:9540`).

**Reproducer:** `dna/friction/f3-interface-value-into-field/`.

**Consequence:** a dynamically created child cannot be given an
assembly-supplied implementation by its creator, because the creator
is library code and must name a concrete locus in the literal.
Assembly-time substitution reaches only statically-owned params.

## F.4 — a `perspective(P)` field must designate; a holder cannot just dispatch

**Tag:** `perspective-hold-without-designation`
**Severity:** closes the other route to F.3's problem.
**Status:** open — the first candidate language REQUEST from this
track (see "Requests" below).

The obvious answer to F.3 is the program-global slot: the assembly
designates `perspective(Performing)` once (`#525` item 2 made that
possible from a constructor), and every dynamic `Attempt` holds
`performer: perspective(Performing)` and dispatches through the slot.
But a perspective field with no initializer is a REQUIRED param:

```
type error: locus `Attempt`: missing field `performer`
```

and a default initializer re-designates the global slot at every
birth, clobbering the assembly's choice (last designation wins).
There is no way to say "hold the slot, dispatch through whatever the
program designated".

**Reproducer:** `dna/friction/f4-perspective-must-designate/`.

**Request shape:** `p: perspective(P);` with no initializer means
*hold, do not designate*; `hale check` errors if no locus in the
program designates `P`. Semantics are already those of the slot
(`spec/semantics.md` "One global slot"); only the designation rule
changes, and it is the constructor-shaped assembly of #521 comment 1
made to reach dynamic children.

## F.5 — a bus reply reaches a flow child at drain, after its `run()`

**Tag:** `bus-reply-delivered-at-drain`
**Severity:** informational; it fixes the routed-work shape.
**Status:** by design (cooperative queue), recorded so nobody fights it.

Routing work over the bus (`WorkRequested` out, `WorkDone` back keyed
by `work_id`) works, but the reply is delivered during the child's
DRAIN — after `run()` returned, before `release(c)` fires:

```
w1 out=-1        <- inside Work::run, after the publish
released 42      <- the parent, in release(w)
```

So a flow child can ask and its OWNER can read the answer, but the
child cannot act on it inside `run()`. The routed shape therefore
puts the retry/reassignment loop in the locus that owns the
performers (`WorkSystem`), not in `Work`; `Work` asks once and
settles with whatever came back.

**Reproducer:** `dna/friction/f5-bus-reply-at-drain/`.

## F.6 — a child type accepted by two parents: `release` runs the wrong parent's body

**Tag:** `release-dispatch-keyed-by-child-only`
**Severity:** memory corruption (SIGSEGV in the DNA core: `Work`'s
release body ran over `WorkSystem`'s layout).
**Status:** compiler bug; see resolution line.

`Attempt` was accepted by `Work` (in-tower attempts) AND by
`WorkSystem` (routed attempts). The routed program segfaulted inside
the first `Attempt::run` with a `String` field slot holding `0x1`.
Minimized: two plain parents that both `accept(c: Child)` and
`release(c: Child)`, each birthing one child in `run()`:

```
A accept from-A
A release ran:from-A
A run done n=1
B accept from-B
A release ran:from-B      <- A's release BODY, on B's self (b.n became 1)
B run done n=1
```

`accept` dispatches to the right parent; `release` dispatches by
child type alone, so the first parent type that declares
`release(c: Child)` wins program-wide and its body executes with the
actual owner's `self`. With different field layouts that is silent
corruption, which is what the core hit.

**Reproducer:** `dna/friction/f6-two-parents-release/`.

**Resolution:** FIXED upstream (hale, 2026-09-05, this track). Every
locus carries a synthetic `__owner_release` pointer, stored at accept
dispatch from the accept'ing parent TYPE's `release` fn (null when
that parent declares none), and the reclaim spine calls through it.
"Is this type a flow" stays a type-wide property (any parent releases
it → run-completion reclaims); WHICH body runs is now per owner.
Regression: `crates/hale-codegen/tests/release_two_parents.rs`. The
core keeps one `Attempt` concept accepted by both `Work` and
`WorkSystem`.

## F.7 — an interface method cannot be `fallible(E)`

**Tag:** `interface-method-fallible`
**Severity:** shapes every storage contract.
**Status:** open — second candidate language REQUEST.

`interface Journal { fn append(expected: Int, ...) -> Int fallible(JournalError); }`
is a parse error (`expected ;, got Ident("fallible")`): the interface
member grammar admits a return type but no error channel. A locus
method CAN be fallible, so an implementation can fail where its
contract cannot say so — the caller through the interface never
learns the error, and `or` addressing (the whole point of
`fallible`) is unavailable at the one place substitution happens.

**Reproducer:** `dna/friction/f7-interface-fallible/`.

**Workaround:** result structs (`AppendResult { ok, revision, error }`)
on every interface that can fail. That is exactly the shape
`fallible(E)` exists to replace.

**Request shape:** `interface_member` admits `fallible(E)`; a locus
satisfies the method only if its own signature declares the same
error type (or none — infallible satisfies fallible, not the
reverse). Call sites through the interface address the error with
`or` like any other.

## F.8 — `or` on an infallible stdlib call checks clean and fails at build

**Tag:** `or-on-infallible-stdlib-path`
**Severity:** minor; check/build disagreement.
**Status:** open (compiler bug; known class — multi-segment stdlib
paths type as `Unknown`, so fallibility is invisible to the checker).

`std::json::find_int_field(line, "seq") or 0` — the reader is
infallible (returns 0 on a missing field), so the `or` is meaningless;
`hale check` says nothing and `hale build` refuses with
`` `or` over unknown path call ``. The checker should reject the `or`
where the build does, with the fn named.

**Reproducer:** `dna/friction/f8-or-on-infallible-stdlib/`.
**Workaround:** no `or` on the flat-object readers.

## F.9 — `write_file_append` is `Int` to the spec and `Unit` to codegen

**Tag:** `write-file-append-unit-vs-int`
**Severity:** blocked the file-backed journal for an hour; misleading
diagnostics.
**Status:** open (compiler bug, three faces).

`spec/stdlib.md` and `stdlib_surface.rs` declare
`write_file_append(path, s) -> Int fallible(IoError)`. Codegen lowers
the call as Unit, so:

- `let n = std::io::fs::write_file_append(p, s) or 0;` fails at build
  with `expression statement other than locus literal or builtin
  call` — a message about something else entirely;
- `... or neg_one()` fails with `` `or` expression in value position
  has Unit success type `` — the honest message;
- `let n = std::io::fs::write_file_append(p, s);` with NO `or` is
  accepted by check AND build, an unaddressed fallible call, because
  the multi-segment path types as `Unknown` (the F.8 class).

**Reproducer:** `dna/friction/f9-write-append-unit/`.

**Workaround:** statement position with an error-check fn:
`write_file_append(p, s) or self.io_failed(err);` and read a counter.

## F.10 — a perspective declared in an imported seed does not resolve

**Tag:** `perspective-across-seeds`
**Severity:** blocks the routing-policy-as-perspective design in a
library.
**Status:** open (compiler bug: the import path-rename pass misses
`serves` lists and `perspective(P)` field types).

`perspective WorkRouting`, two `: serves WorkRouting` impls and a
`selection: perspective(WorkRouting) = CapabilityFirstSelection { }`
holder are clean in-seed. Through `import`:

```
locus `__lib_..._CapabilityFirstSelection` serves unknown perspective `WorkRouting`
param `selection`: declared `__lib_..._WorkRouting`, default is `__lib_..._CapabilityFirstSelection`
```

so a library cannot offer a live-rebindable policy slot at all; only
interfaces cross the seed boundary. This is the same class as F.1
(enums): a name family the mangler does not rewrite.

**Reproducer:** `dna/friction/f10-perspective-across-seeds/`.

**Workaround:** `WorkSelection` is an interface in the core;
substitution happens at construction only (#525 item 2 proved the
constructor-site designation in-seed, fixture
`65-perspective-ctor-override`).

## F.11 — `forbid reaches` follows the declaration default, not the constructor override

**Tag:** `reaches-through-interface-field-default`
**Severity:** SOUNDNESS — fail-open on the exact shape #521's assembly
is built from.
**Status:** open; filed upstream from this track.

`Dna { deployment: LocalApplyDeployment { } }` overrides the
`deployment: Deployment = NoDeployment { }` default. `Dna.stage`
calls `self.deployment.apply(...)`. The constitution
`forbid reaches(organism, effects(genome_apply))` passed although
`LocalApplyDeployment::apply` (the `genome_apply` carrier) is what
runs. Minimized in-seed:

- default `Noop`, override `Real` (carrier): check PASSES — fail-open;
- default `Real`, override `Noop`: check REFUSES through `Real::apply`,
  which never runs — false positive.

One-hop and two-hop, in-seed and cross-seed, all resolve the same way:
against the declaration-site default. Concrete-typed fields are
resolved correctly, which is why `apply_gate_fail` uses a concrete
bypass handle and `apply_gate_pass` holds partly for the wrong reason.

The rule the engine should follow is the one the spec states for
unknowns: an interface-typed field's callee set is every impl the
closed world stores into that field (every instantiation site's
literal, the default included), or, failing that analysis, every
locus that satisfies the interface — "a hole beats a false proof of
absence".

**Reproducer:** `dna/friction/f11-reaches-default-not-override/`.

---

## Requests and bugs, summarized (2026-09-05)

Eleven entries. What the fixtures established, against #521's prediction
that the first language request would be multiple `accept` clauses:

**The multi-accept request did not materialize.** A Step that must own
Work and delegated Tasks writes the `Task { }` literal anyway; interest-
based ownership bubbles it to `Metabolism`, lineage rides as data, and
settlement comes back over a keyed topic published by the mediating
supersystem. That is H9 in the letter, it compiles, it reclaims, and
`recursion_settlement_test.hl` proves it. Single-accept-type per parent
stands, for now, as a design that pushed the domain toward a better
shape than the one the issue drew.

**Candidate language requests** (each with a reproducer; none filed
yet — the fixtures should sit for a while first):

1. F.4 — `p: perspective(P);` with no initializer means *hold, do not
   designate*. Without it, a dynamic child cannot dispatch through the
   slot its assembly designated, and the constructor-shaped assembly
   of #521 stops at statically-owned params. This is the one that
   changes the DNA design most.
2. F.7 — `fallible(E)` on interface method signatures. Without it every
   storage contract is a result struct, which is the shape `fallible`
   exists to replace.
3. F.3 — an interface-typed value flowing into an interface-typed
   field (the identity coercion), WITH an ownership rule. Probably
   subsumed by F.4 for the performer case.

**Compiler bugs fixed in this track:** F.2 (`@unbounded` ignored by the
hot-path lint), F.6 (release dispatch by child type alone — memory
corruption).

**Compiler bugs open, reproducers under `dna/friction/`:** F.11
(SOUNDNESS: `forbid reaches` through an interface-typed field follows
the declaration default, not the constructor override — fail-open on
the assembly shape itself; fix this first), F.1 (enum match across
seeds: checker and codegen), F.10 (perspectives do not cross seeds),
F.8 (`or` on an infallible stdlib path accepted by check), F.9
(`write_file_append` Int vs Unit, misleading diagnostics). F.1 and
F.10 share a cause — name families the import mangler does not
rewrite: a library cannot use its own enums or perspectives until they
are.

**Recorded, by design:** F.5 (a bus reply reaches a flow child at
drain, so the retry loop lives with whoever owns the performers).

**Things the survey said to verify, now verified:** `adopt` of a
constitution declared in an imported seed was not needed — the app's
own `constitution DnaCore { ... }` plus groups naming `dna::*` loci
works and is what the law fixtures use; reassigning an interface-typed
field was never exercised because F.3 forbids the value flow before
reassignment is reached; String-keyed settlement topics carried every
delegated Task in the fixtures without visible cost, though nothing
here is at scale.
