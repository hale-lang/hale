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

**Resolution:** FIXED upstream (GH #534, hale PR #538, 2026-09-08):
the seed mangler now rewrites two-segment `Enum::Variant` paths in
expression and pattern position, and an importer can spell
`lib::Enum::Variant` in both. `Disposition` is an enum again in
`dna/core/types.hl`; `review_authority_test.hl` matches it on the
importer's side.

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

**Resolution:** FIXED upstream (GH #534, hale PR #538, 2026-09-08):
`serves` lists are renamed with their perspective. `WorkRouting` is a
perspective again in `dna/core/work_system.hl`; the assembly
designates it at construction (#525 item 2, now across a seed
boundary) and `WorkSystem` re-points it live with `reperspective` —
`assembly_test.hl` swaps policies mid-run. One thing the swap taught:
`reperspective` swaps CODE and preserves STATE (the footprint), so a
policy expressed as params (`default_order: String = ...`) does not
change when the slot is re-pointed — the new impl's methods ran over
the old impl's orders. Policies now live in their impls' methods and
the shared footprint holds only the decision counter, which the test
reads across both policies through the contract.

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

**Resolution:** FIXED upstream (GH #533, hale PR #538, 2026-09-08).
A declared interface keeps its own name in the field-type map, so
the call fans to every conformer in the closed world. Conservative
by construction, and it has a consequence for the core:
`PrivateModelRouter.private` had been interface-typed, so after the
fix the confinement claim saw the external backend through it and
`confine_pass` failed on merged main. An interface-typed slot admits
every conformer; a CONCRETE slot admits one; confinement by wiring
therefore needs the concrete type at the boundary, which is what
`PrivateModelRouter` now declares. Per-field narrowing (only the
impls the program actually stores into that field) would give the
precise answer without the concrete type; filed as a follow-up.

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

## F.12 — a keyed subscription never hears a wire delivery

**Where:** `dna/core/review.hl`, the membrane (GH #527 B6).

**What:** `ReviewVerdict` is `keyed_by review_id` so a verdict reaches
only the Review it names — the same idiom `Step` uses for `WorkDone`.
Bound on a unix socket, a `subscribe ReviewVerdict as on_verdict where
key == self.review_id` subscription receives nothing: remote fanout
is unkeyed at v0.1 (`spec/semantics.md`, "Remote fanout stays unkeyed")
and the listening side's wire dispatch skips every entry with a key
filter, because nobody re-derives the key from the decoded payload.
The publisher's counters say `sent=1`; the Review stays open.

**FIXED (GH #529 prep):** codegen synthesizes a per-keyed-topic
extractor (the publish site's exact computation over the deserialized
payload) and the runtime derives the key on every inbound path (unix
serve loop, boot-window flush, UDP reader, adapter inbound), so
`where key == …` means the same thing on both sides of a socket. The
Review subscribes keyed again. Test:
`crates/hale-codegen/tests/binding_keyed_over_wire.rs` (Int and
String keys).

**Was worked around:** the Review subscribed unkeyed and answered only
to its own `review_id` in the handler. Correct (every Review is a
separate locus, so a foreign verdict is a no-op), and cheap at this
scale, but it is the pattern the keyed topic exists to make
unnecessary, and it silently diverges from the in-process idiom: the
same source line means "mine only" locally and "everyone's" over a
binding.

**Wanted:** key derivation on the receive side of a binding — the
codec already decodes the payload and the topic declares which field
is the key — so `where key == …` means the same thing on both sides
of a socket. Until then the checker could at least warn when a keyed
subscription's topic is bound on a listen transport.

## F.13 — a listen binding serves one peer at a time

**Where:** the membrane (GH #528): `hale dna run` attaches iris to the
organism's `dna.intent.offered` / `dna.review.verdict` sockets, and
then `hale dna ask` cannot get in.

**What:** the unix listen transport's serve loop is
`accept → read until EOF → re-arm` (`lotus_bus_unix_serve`): one
peer holds the socket until it hangs up. iris's connect-role routes
are held for its whole run, so a second connector (the membrane
client) is left in the backlog and its message is never read. The
publisher's counters say `sent=1`; the organism journals nothing.

**FIXED (GH #529 prep):** the serve loop polls the listener beside
every accepted peer (up to 64): connections are admitted as they
arrive, each keeps its own framed seq space, a peer's EOF closes
that peer only, and the exit quiesce still drains every connected
peer to EOF. `hale dna ask` / `review` connect directly beside an
attached iris; the `iris.port` detour is gone. Test:
`crates/hale-codegen/tests/binding_multi_peer.rs`.

**Was worked around:** `hale dna run` wrote `.hale/dna/iris.port`
while iris was attached and `ask` / `review` published through
iris's `/ctl` endpoints.

## F.14 — an imported main's inline claims cannot see its seed's groups

**Where:** the acceptance application (GH #529 D7): the site's chat
server declares `group participants = { Participant };` and a claim
`forbid reaches(participants, effects(secret_use)) avoiding rooms`
inline in its `main locus`. Its own tests `import ".." as app`.

**What:** `hale test` on the importer fails with "claim
`guests_sign_only_via_rooms` names group `participants`, which is
never declared" — the imported main's inline claims are checked in
the importer's scope, where the seed's top-level groups are not
visible. The same claim in a `constitution Chat { … }` the main
adopts resolves its groups in the constitution's own scope and
passes. Reproducer: `dna/acceptance/chat-server` with the claim moved
back inline, then `hale test dna/acceptance/chat-server`.

**Worked around:** the acceptance copy carries the claim as an
adopted constitution (the page keeps it inline).

## F.15 — a flow child cannot outlive its `run()` to await a bus reply

**Where:** the intent path (GH #529 D7 steps 4–6): a Task's routed
Work publishes `WorkRequested` and the assembly answers with
`WorkDone` after seconds of toolchain work.

**What:** in-process (no bindings) the round trip is synchronous —
the request dispatches nested inside the publish and the reply is at
the Work by drain (F.5). In a bound organism (an off-thread bus) the
publish is queued: the Work, Step, Workflow and Task all finish their
`run()` before the assembly's handler runs, so the Task settled
`failed` with an empty history and the later `WorkDone` had no
subscriber. Waiting inside `run()` with sliced sleeps does not help:
the whole chain runs inside the assembly's `on_intent` handler, and
no other handler on the pool is dispatched while a handler sleeps —
the request never reaches its handler, the Work waits until its
timeout. A locus born in a handler that must survive an asynchronous
reply has no shape in the language today (it is either a flow child
that dissolves at drain or a static param).

**Recorded, by design (for now):** a routed Work settles `pending`,
the Step / Workflow / Task carry it up unchanged, the assembly
journals `task.pending` at the offer, and — when the routed Work is
done — settles the durable Task in the Journal (`task.done` /
`task.failed`, naming the Work and the performer). The live tree is
the expression of one synchronous pass; the Journal is the Task. In
process, where the reply arrives at drain, the Task still settles
`done` in its own pass and nothing is journaled twice.

**Compiler bugs fixed in this track:** F.2 (`@unbounded` ignored by the
hot-path lint), F.6 (release dispatch by child type alone — memory
corruption).

**Compiler bugs fixed since:** F.11 (SOUNDNESS: `forbid reaches`
through an interface-typed field followed the declaration default —
fixed in #538 by fanning to every conformer; per-field narrowing is
the open follow-up).

**Compiler bugs open, reproducers under `dna/friction/`:** F.1 (enum match across
seeds: checker and codegen), F.10 (perspectives do not cross seeds),
F.8 (`or` on an infallible stdlib path accepted by check), F.9
(`write_file_append` Int vs Unit, misleading diagnostics). F.1 and
F.10 share a cause — name families the import mangler does not
rewrite: a library cannot use its own enums or perspectives until they
are.

**Recorded, by design:** F.5 (a bus reply reaches a flow child at
drain, so the retry loop lives with whoever owns the performers),
F.15 (a flow child cannot await an asynchronous reply: routed Work
settles pending, the assembly settles the durable Task from the
Journal), F.14 (an imported main's inline claims cannot see its
seed's groups; an adopted constitution can).

**Runtime limitations, FIXED:** F.12 (keyed subscriptions now hear
wire deliveries — receive-side key derivation), F.13 (a listen
binding serves many peers).

**Things the survey said to verify, now verified:** `adopt` of a
constitution declared in an imported seed was not needed — the app's
own `constitution DnaCore { ... }` plus groups naming `dna::*` loci
works and is what the law fixtures use; reassigning an interface-typed
field was never exercised because F.3 forbids the value flow before
reassignment is reached; String-keyed settlement topics carried every
delegated Task in the fixtures without visible cost, though nothing
here is at scale.

## F.16 — a vec form's `set` with a value read from the same vec segfaults

Found writing the host in Hale (GH #566 F8). Swapping two items of a
`@form(vec)` the obvious way — read both with `get`, write them back
crossed with `set` — kills the process with SIGSEGV, and `hale run`
reports only `exit 1` with no diagnostic:

```hale
type Row { id: String = ""; state: String = "pending"; note: String = ""; }
@form(vec)
locus Rows { capacity { heap items of Row; } }
locus Keeper {
    params { rows: Rows = Rows { }; }
    fn go() {
        self.rows.push(Row { id: "purpose", note: "first" });
        self.rows.push(Row { id: "m1", note: "second" });
        let a = self.rows.get(0) or Row { };
        let b = self.rows.get(1) or Row { };
        self.rows.set(0, b) or discard;   // retires slot 0's strings, which `a` still aliases
        self.rows.set(1, a) or discard;   // copies from freed memory
        println("rows ", self.rows.len()); // never printed
    }
}
fn main() { let k = Keeper { }; k.go(); }
```

The read-modify-write of one slot (`get(i)`, change a field, `set(i, …)`)
appeared to work in the same program — luck, by the same reading.
Presumably the single-owner retire at a cell store (the
`vec.set` retire of GH handoff #263) frees the old value's strings
while a value read by `get` still aliases them; a `get` that returned
an owned copy, or a `set` that retired after the copy, would close
it. **Worked around** in `dna/host`: the projection never keeps
rows in a vec — it derives each row from the record on demand and
sorts an id list built by string insertion.

Two frictions in one: the aliasing, and the silence — a segfault in
a `hale run` program prints nothing, not even that it died by signal.

**FIXED** (GH #577): a vec form owns its elements — `get` returns the
caller's copy and `set` / `push` store the vec's copy, whatever arena
the value came from — and `hale run` reports a death by signal.
