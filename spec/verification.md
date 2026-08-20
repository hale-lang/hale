# Static verification surface

This page is the canonical catalog of the compile-time **checks** the
toolchain runs beyond ordinary type-checking — the structural and
semantic guarantees a program earns at `hale build` / `hale check`
time. It describes shipped behavior; the verification roadmap that drove
these checks — now delivered — is recorded in GitHub issue #18 (closed).

Two severity levels exist:

- **error** — fails the build (`Diag::is_error()` is true).
- **warning** — surfaced but non-fatal; the only non-error diagnostic
  Hale emits. Used where the flagged shape is a real smell but can be
  legitimate, so the call is left to the author.

Most checks run in the bundle-level passes of
`crates/hale-types/src/check.rs` (`check_bundle`); a few resolve-time
ones run in `crates/hale-types/src/resolve.rs`; cell slot-of-origin is
a codegen-time check. Each entry names the enforcing pass.

## Concurrency & placement safety

The bus + cooperative-pool model is the substrate; these checks keep a
program's placement coherent with how the runtime dispatches.

| Check | Catches | Severity | Enforced by |
|---|---|---|---|
| **Single-threaded-method invariant** | a *direct* cross-pool method call (`self.field.method()` where `field` is placed on a different pool) — it would run the callee's method on the wrong thread | error | `check_placement_single_thread` |
| **Dead bus receiver** | a non-`main` cooperative locus that subscribes to the bus *and* makes a blocking call in `run()` — the blocking call monopolizes the pool thread so the dispatch never delivers and its handlers never fire | error | `check_cooperative_pool_blocking` |
| **Blocking call on a cooperative pool** | a blocking `run()` (`recv`/`accept`, `process::run`) on a pool that isn't `where async_io` — it holds the pool's OS thread and stalls co-scheduled loci. Follows the call graph: blocking reached through a helper fn or `self.method` is flagged too | warning | `check_cooperative_pool_blocking` |
| **Cooperative pool starvation** | two or more loci on one cooperative pool (not `where async_io`) whose `run()` bodies statically never return (terminal `while` with no exit — `while true`, `while !self.draining`, or a never-assigned Bool flag) — the pool runs each `run()` to completion in birth order, so the later `run()` bodies never start. Covers fields with no placement entry (they default to pool `main`) and the main locus's own `run()`, which begins only after params-init | warning | `check_cooperative_pool_blocking` |
| **Nested long-running child** | a non-`main` locus holding a params field of a locus type whose `run()` doesn't return — the canonical fix is hoisting it to a `main` sibling with its own placement | error | `check_nested_long_running_child` |
| **Unowned subscriber locus** | a bus-subscribing locus instantiated *non-owned* inside another locus's method/handler body — it dissolves at that scope's exit, so its subscription can never fire (overridable with `--allow-unowned-subscriber`) | error | `check_unowned_subscriber_locus` |

The dead-receiver error is deliberately **direct-call-only** (its
call-graph surface is not widened), while the blocking *warning* is
interprocedural — the high-stakes diagnostic stays precise. See
`spec/semantics.md` type-check rules 7–8 and
`docs/src/services/concurrency.md`.

## Bus-graph property checks

The bus topology is a typed directed graph in the source; these walk
it. (GitHub issue #18 item 4.)

| Check | Catches | Severity | Enforced by |
|---|---|---|---|
| **Orphan topic / subject** | a declared `topic` or literal subject wired to only one end — published with no subscriber, subscribed with no publisher, or used by neither | warning | `check_bus_graph` |
| **Cross-locus bus cycle** | a publish→subscribe→publish loop spanning ≥2 loci — the cell hops via the cooperative queue and can spin / livelock | warning | `check_bus_cycles` |
| **Intra-locus re-entrant cycle** | an *unconditional* self-republish loop within one locus — intra-locus self-dispatch is a direct synchronous call, so it recurses on one thread without bound (stack overflow) | error | `check_bus_cycles` |
| **Bus backpressure** | a publish inside an unbounded `while true` loop with no flow-control or exit point (`yield` / `sleep`/`tick` / input-pacing `recv` / `break`/`return`) — floods the bus without bound | warning | `check_bus_backpressure` |
| **Subject type-mismatch** | two sites on the same literal subject string declaring different `of type` payloads — a subscriber would decode the wrong type | error | `check_bus_subject_types` |
| **Routing-key fallback rules** | an `on_unmatched: fallback` topic with no `where key == _` subscriber, or a `where key == _` filter on a non-fallback topic | error | `check_phase3_fallback_subscribers` |
| **Topic parent-chain cycle** | a topic hierarchy that loops (`topic A : B; topic B : A`) | error | `finalize_topic_chain` (resolve) |

Orphan detection is **closed-world gated** (it runs only when a `main`
locus is present), and suppressed by transport bindings, `**` wildcard
coverage, cross-seed (`alias::Topic`) references, and self-pub/sub —
so library seeds and external peers aren't falsely flagged. The
intra-locus cycle error counts only *unconditional* sends as edges: a
self-republish guarded by `if`/`match`/loop is a terminating state
machine, not unbounded recursion, and is left alone. See
`spec/semantics.md` type-check rules 9–10.

## Claims — domain requirements as checked sentences (GH #382, phase 1)

Bundle-level, **named** sentences over the program graph, declared in
the main locus and evaluated in `hale check` as **errors** — never
advisories (an advisory claim reads as law and doesn't bind, the #354
fail-open shape). The motivating property is multi-tenant isolation:
"no path from domain A to domain B" as one declaration with a name a
contract can cite, instead of per-fn `@effects` contracts scattered
across every position with completeness by hope.

```
group delta_wing = { delta::*, DeltaStore };
group gamma_wing = { gamma::Research };

main locus Org {
    params { ... }
    claims {
        iso_dg: forbid reaches(delta_wing, gamma_wing);
        iso_calls: forbid reaches(delta_wing, gamma_wing) via { calls };
        no_spend: forbid reaches(gamma_wing, effects(money));
    }
}
```

- **Groups are declared vocabulary, never patterns.** A `group` names
  a set of declared program elements (loci, free fns, imported
  decls). An unknown member is an **error, not an empty set** — the
  misspelt-effect-class lesson applied at the group layer — with a
  did-you-mean. An empty group is a **vacuity error** unless it opts
  out with `may_be_empty`: a `forbid` trivially satisfied by an empty
  quantification domain is a fail-open wearing formal clothing. The
  only glob is `alias::*` — trailing-only enumeration of an imported
  seed's decl set via the same rename table codegen resolves
  `alias::Name` through, deliberately mirroring the trailing-`**`
  rule for bus subjects. Qualified members (`alias::Name`)
  canonicalize at the mangle stage (#334's path), never by
  name-suffix matching.
- **`forbid reaches(SRC, DST)`** — absence under closure: no path
  from any element of SRC to any element of DST. Evaluation is
  fn-grained: a locus member projects to all of its methods,
  lifecycle hooks, and modes, which only ever *adds* sources and
  sinks (the conservative direction). Edges are the resolved call
  graph (stdlib bodies merged, handle-method calls resolved) and the
  declared bus graph (a publish site composes with every subscriber
  of its subject, wildcard subscribers included). `via { calls }` /
  `via { bus }` restricts which relations compose; the default is
  the **full composition** — more edges, conservative. `bus`
  composes publish sites in the visited fn's own body; the default
  (with `calls`) is the sound transitive closure.
- **`effects(<class>)`** in target position: the declared carriers
  of an effect class (an `@effects(is: {…})` frontier entry or a
  classified leaf), composed-class masks included. An undeclared
  class in claim position is an error with a did-you-mean.
- **Witnesses.** A violation renders a minimal countermodel path in
  author spelling — `` `delta::Triage::on_task` -(publishes
  "org.metrics")-> `gamma::Research::on_metric` `` — cross-seed
  symbols demangled. One witness per claim (the minimal
  countermodel, not an enumeration). **Provenance** (#392): the
  witness also says where to edit, as secondary diagnostics in the
  effect system's root + leaf shape — the callsite that crosses the
  boundary (or the publish site and the subscription decl for a bus
  hop) and the forbidden destination's declaration. Spans are
  emitted only for bundle decls: stdlib bodies parse in their own
  offset space, and a span from there attributed to a bundle file
  would name the wrong source, so a stdlib-interior hop renders by
  name alone.
- **Unknown ⇒ violation.** An indirect call (function-typed
  parameter, #353) or a computed publish subject on a path from a
  `forbid` source cannot be certified and is reported as a
  violation, exactly as `@no_syscall` treats the same shapes. And
  the **unresolved-callee backstop**: EVERY method call on a
  receiver the summarizer cannot type (a struct-literal receiver,
  a chained `self.a.b` field, a call result, a branch value) fails
  closed in any judgment that traverses calls — without it,
  `forbid reaches(A, B)` certified while the forbidden path
  executed (found by the #382 soundness audit), and a name-keyed
  version was still blind to WRAPPERS reaching the target
  transitively (found by the follow-up review). No name comparison
  is sound; the edge itself is the uncertainty, and the artifact
  records it (`untyped_receiver_call:<callee>`) inside the hashed
  model half. Synthesized form/builtin methods (`counts.set`)
  carry a known receiver type and are exempt. The rule is one
  shared predicate applied by every judgment that traverses calls
  — claims, effect inference, effect certificates, `@budget`, and
  the quantitative dims — so fn-level certificates and
  bundle-level claims always agree (#392 closed the `@budget`
  guard, which had the message but not the test).
- **Interface dispatch fans out.** A method call on an
  interface-typed value (`route.handler.handle(ctx)` — the stdlib
  router's own shape) is resolved by closed-world enumeration
  (#392): the summarizer fans the one written edge out to every
  conforming locus. Conformance here is structural name + arity
  over the declarations — a superset of the checker's typed
  conformance, safe because over-approximation only adds edges.
  Reachability and effect judgments walk every alternative;
  counting judgments (`bound`, `@budget`, the quantitative dims)
  take the **max** over the alternatives of one dispatch site,
  because one invocation dispatches to exactly one target — a sum
  would count phantom calls no execution performs. An interface NO
  locus conforms to has no values in a closed world (an interface
  value only arises by coercing a conformer), so its call sites
  are dead: they contribute nothing to any judgment, and the
  artifact records each (`uninhabited_interface_call:<iface>.<callee>`)
  inside the hashed model half so a conformer appearing later
  changes `shape_hash`.
- **Placement.** `claims { }` is only legal inside `main locus`
  (parse error elsewhere): main is the closed-world gate, so
  bundle-wide claims cannot be evaluated anywhere earlier, and
  one-main-per-bundle makes the claims root unique. Claim names are
  the contract-of-record and must be unique.

The remaining verbs (#382 phases 2–5):

- **`only edges A -> B { publish T; subscribe T; }`** — isolation
  with an exhaustive grant enumeration: every DIRECT edge from A to
  B must match a granted line, and every un-granted edge is
  reported (the grant list is the review surface, so the full diff
  matters). `publish T` and `subscribe T` admit the same bus edge —
  the verb names which end's declaration is the reviewable line.
  Call edges are never grantable: a direct call across the boundary
  is always an un-granted edge. Transitive paths through third
  parties are `forbid reaches` territory; a subscriber outside B
  (the `log.**` sink shape) is not an A→B edge and needs no grant.
- **`bound C <= N on paths from G`** — `@budget`'s per-call
  semiring behind a claims surface: total sites of user class C
  reachable per invocation (a call-tree SUM, exactly
  `@budget(C = N)`'s semantics — two calls to a carrier are two
  sites). A recursion cycle, loop-nested carrier, indirect call, or
  computed publish subject is unbounded and violates. The witness
  carries the count and a representative chain. Built-ins keep
  their `@budget` spellings.
- **`require subscribes|publishes(some G, topic T)`** — existence
  over the DECLARED bus ends (the `bus { }` blocks — "wired" is a
  declaration property).
- **`require attributed(all C)`** (GH #436) — every user fn that
  **directly** performs an operation of built-in class `C` carries at
  least one **user-declared** effect class. `require attributed(all
  syscall)` says every place the program touches the OS names a
  purpose.

  **Orthogonal to interposition.** `forbid reaches(app,
  effects(syscall)) avoiding gate` constrains WHERE a boundary is
  crossed and says nothing about what any crossing is FOR; this
  constrains attribution and says nothing about location. Neither
  implies the other: all I/O can funnel through one
  `write(path, bytes)` everyone calls for everything (interposed,
  unattributed), or forty loci can each touch the OS while every one
  names its purpose (attributed, un-interposed). A hybrid wants both.

  It is a universal over the whole closed world, where `avoiding` is
  scoped to a group — so a locus outside the group, including one
  added later, is covered without editing the claim.

  A **direct site** is: a classified frontier path call, an `@ffi`
  declaration (the declaration itself, not its caller — it is the
  application-owned boundary and can carry the purpose), a resolved
  callee the author does not own whose effects include the class, a
  syntactic site (publish, allocation), or the fn's own
  `@effects(is: {C})`. That last covers a method that declares itself
  a carrier and calls nothing classified — the shape a privileged
  operation takes. A built-in in `is:` establishes that the operation
  exists; it is not its purpose, so a user class is still required.

  An **indirect or opaque call** the checker cannot resolve leaves
  the claim `uncertified` unless the enclosing fn already names a
  purpose — the same refusal-to-certify posture as the rest of the
  stack.

  **DIRECT, not transitive**, and that is load-bearing: transitively,
  every caller downstream of one attributed fn inherits the label and
  passes, which would make the claim nearly vacuous. The attribution
  point is the first **application-owned** fn crossing out — a
  frontier path call, an `@ffi` declaration, or a Hale-source stdlib
  body whose own effects include the class. An ordinary application
  callee is judged on its own row instead, which is what keeps this
  direct. Attaching only to frontier path calls would have made
  coverage depend on whether an API happens to be a path call or a
  stdlib locus method — not a stable boundary to hang a security
  claim on. A built-in in
  `is:` does not count — it restates what the compiler already
  infers, and the claim asks for a purpose the author supplied. The
  class must be a built-in; a user class there would be trivially
  true while reading like a contract.

- **`require sealed(all G)`** (GH #436) — every locus in `G` is
  declared `@sealed`. A **universal** over the group's members, which
  is why the quantifier is `all` rather than the `some` the other
  `require` forms take: those ask whether an endpoint exists
  anywhere, this asks whether every member holds. Reports every
  unsealed member in one diagnostic — a baseline is adopted once and
  the reader wants the whole list. Without it, sealing is per-locus
  discipline, and one unsealed member of a vault group is the whole
  hole.
- **`cover topic in seed(a): subscribed_by(some G)`** — bounded
  universal: every topic the seed imported as `a` declares has a
  subscriber in G. Every uncovered topic is named. A seed with no
  topics is an error at the claim (an empty coverage domain holds
  vacuously).
- **`count publishers|subscribers(topic T) ==|<=|>= N`** — the
  cardinality family over distinct loci; `== 1` is the invariant
  behind every single-writer pattern, and a violation names the
  competing writers.
- **`during P`** on `forbid` — restricts sources to the named
  phase of each source locus (`during birth` is the quiet-boot
  claim), evaluated against the model's **phase relation** (#392):
  lifecycle hooks (`birth`, `accept`, `release`, `run`, `drain`,
  `dissolve`) and modes (`bulk`, `harmonic`, `resolution`) are
  hook-phases the runtime drives; an ordinary method is its own
  source-slice phase. The relation is exported in the topology
  artifact, which is what makes a `during` row independently
  re-derivable. A phase naming nothing in the group is an error,
  not a vacuously-holding claim.
- **`avoiding G`** on `forbid` — masks G's vertices out of the
  walk, which makes it the interposition form: "every path from A
  to B passes through the gate" is `forbid reaches(A, B) avoiding
  gate`.

**Indexed effect families** (#382 phase 3): `domain wing = { delta,
gamma };` declares a closed index domain; `effect knowledge(wing);`
declares a family. Every instantiation `knowledge(delta)` interns
as an ordinary declared class and `knowledge(*)` as an
auto-populated composed class over all of them — the whole feature
is a reduction onto shipped machinery (masks, `only:` complements,
cross-seed remap, did-you-mean), so a misspelt index is an
undeclared-class error and a domain member added later lands
OUTSIDE every existing `only:` contract (#354's fail-closed,
inherited rather than re-derived). The domain must be declared
earlier in the same file as the family. Companion:
`@budget(<user class> = N)` bounds calls to declared carriers of a
class along any path, with the same loop/indirect unboundedness
rules as every per-call dimension.

**The topology artifact** (#382 phase 2; schema 1.11):
`hale check <t> --dump-topology` emits the serialized model —
sorts (loci, fns, topics), relations (calls with **weights**: loop
nesting, unbounded-loop membership, interface-dispatch tags;
publishes; subscribes), the through-stdlib **contracted** edges
(`calls_via_stdlib`: user→user paths whose interior is stdlib
bodies, collapsed to their endpoints with a conservative loop
flag, so reachability over the artifact matches reachability as
evaluated), the declared **groups**, the effect **labels**
(declared carriers), the **phase relation**, the **seed sort**,
the compiler-**derived** per-fn effect sets, the **supervision**
relation (schema 1.10, downstream handoff — one row per
`on_failure` handler: supervising locus, supervised child + error
types, the recovery ops the body invokes, and a literal retry
bound when one is written; a policy change moves `shape_hash`,
and `provenance.supervision` carries the spans, so the observer's
live RESTART/SUPERV_TRANS stream finally has declared policy to
anchor to), and the **unknowns**
(fns with indirect calls, untyped-receiver calls, dead
uninhabited-interface dispatch, or computed publish subjects), all
in author spelling — plus every named claim's normalized form and
result, under a schema version and a `shape_hash` (FNV-1a/64 over
the canonical model half; claim RESULTS are excluded, so one
topology under different law keeps one shape). Schema 1.11
(GH #476 Change 6) adds three unhashed, digest-covered typed
sections: **`law`** — every lowered `ClaimIr` row with its
ordinal, origin (`main` / `constitution:<name>` / `library` /
`annotation`), judgment **family** (`reachability` / `boundary` /
`endpoint` / `bound` / `certificate` / `unmigrated` / `fleet`),
machine **verdict**, a TYPED **`law` payload** (one tagged object
per `ClaimIr` variant carrying the law's operands — each reference
as `{"name": <raw canonical identity>, "display": <author
spelling>, "resolved": bool}`; the raw half is the machine join
key, the same deliberate raw/display duality the `topics` section
carries), per-certificate **`certs`** evidence keyed by
certificate ordinal for the certificate family, and source
provenance, plus `law_digest` (the law table's semantic digest)
and `inputs_digest` (the analysis-inputs digest) — the ties a
consumer checks before trusting external evidence against this
artifact; **`capabilities`** — the model's positive completeness
account, typed (`exact_cardinality` is derived: endpoint counts
come from a closed-world enumeration and are exact unless a hole
says otherwise); and **`adequacy`** — per migrated judgment
family, `exact` when capabilities vouch every relation family that
judgment consumes, else `degraded` (the certificate family
consumes PUBLISHES — a publish-set contract cannot prove a
computed subject in-set). The UNMIGRATED families (`causes:`,
`depends:`, `@budget`) carry the old engines' authoritative
results in their law rows, so no non-passing law row can coexist
with a `clean` document verdict. The emitted model half — and
`shape_hash` — come from the `ApplicationModel` projection
(`project_model_half`), and so do the unhashed `sources`,
`provenance`, and `topics` sections (`project_unhashed_tail`);
the legacy gathering survives only as the corpus differential's
comparison arm until Change 9. Bus selectors in the `law` payload
serialize their resolved CANDIDATE sets (the topic identities and
wire patterns the selector matched) plus the selector's own
source location; `capabilities` carries `exact_publishes` and
`exact_subscribes` as INDEPENDENT flags, and per-family
`adequacy` reads the positive account — a family is `exact` only
when capabilities vouch every relation family its projection
consumes (holes are the validation cross-check, never a source of
positive knowledge). A payload contract carries a structural
`opaque` discriminant — `opaque` is not a reserved word, so a
struct field literally named `opaque` keeps its structural shape.
The `law` section also carries the CANONICAL catalogs:
**`fn_universe`**, **`loci`**, **`groups`**, and **`topics`** as
`{name: <raw canonical identity>, display}` pairs (topics add
their wire `subject`), **`subjects`** (the wire-subject pattern
universe), and **`effect_classes`** (each class with its
`declared` / `cyclic` status). `fn_universe` covers every
function the model knows — wider than the legacy `sorts.fns`, so
module-scoped annotation subjects resolve. A resolved reference
must match one exact `(name, display)` pair — the raw half is
the machine join key and is anchored, not merely
cross-row-consistent — and the catalogs are cross-tied to the
`topics`/`groups`/`sorts` sections other consumers join on. Bus
selectors keep their candidate sets, and admission RECOMPUTES
each set from the catalogs with the compiler's own matching rule
(`bus_ref_matches`) — a candidate swap under an unchanged
selector name is refused. Legacy `claims` rows carry the law `ordinal`
they project from. **One shared admission**
(`validate_law_account`) runs for Track A rendering AND fleet
composition: it decodes every payload against the closed
per-kind shape (unknown fields refused), resolves every
`resolved: true` reference against the artifact's own catalogs
(loci, groups, topics, phases, seeds, effect classes,
`fn_universe`), refuses a `holds` verdict over any unresolved
operand, re-renders each certificate's expected form from the
typed operands and requires the row's `certs` to match it (an
operand swap cannot keep the old certificates), requires the
`lowered` section to project ONE-TO-ONE from the typed account —
every lowered row carries its law `ordinal` (and certificate
`cert` ordinal where applicable) and must match the form, result,
and subject re-rendered from the typed operands, and every
generated expectation must be met, so deleting law rows orphans
their evidence even in an annotation-only artifact — recomputes
per-row and document verdicts from the evidence, checks the
one-to-one claims-to-law projection in both directions, requires
the `law.legacy` report (the old engines' verdicts for the
unmigrated `causes:` / `depends:` rows, keyed by ordinal and by a
form FINGERPRINT re-rendered from the typed operands; an operand
mutation orphans the entry, and a `causes:` row naming an
undeclared or cyclic class cannot hold), refuses fleet-family
rows outright (an application artifact does not own a fleet
account — that is Change 7's), and recomputes `adequacy` from
`capabilities`. Static invalidity DOMINATES (round 7): an
unresolved operand or an undeclared/cyclic effect class makes
`invalid` the ONLY admissible verdict — a replayed engine result
is never an alternative — and otherwise a certificate row's
verdict is EXACTLY its recomputed evidence severity. A subject
outside the legacy analyzable universe carries no certificates
and judges `uncertified` with its residue on the row (round 8):
`sorts.fns` for fn subjects, the `analyzable` flag on `law.loci`
rows for phase contracts — module-scoped bodies are residue, not
invalidity. An implicit lifecycle phase with no hook body gets a
SYNTHETIC `Holds` certificate from the evidence layer (no hook
performs no effects), so `@phase_effects(birth: {})` on a
hook-less locus is `holds`, not a missing report. Beyond the
statically decodable invalidities, the claims evaluator's other
legitimate `Invalid` outcomes (an operand outside a verb's
domain, projection vacuity, an empty `during` slice, `avoiding`
overlap, …) admit by RETAINING the judgment's explanation — an
`invalid` row must carry either a decodable invalidity or its
judgment's evidence, and an unanalyzed `uncertified` its
residue. Machine-`invalid` rows may still
PRESERVE the old engines' reports (`law.legacy`, keyed budget
`lowered` rows) — bound by fingerprint as optional evidence,
never demanded and never overriding the machine verdict, so the
compiler's own cyclic-class artifacts admit. The catalogs are
CLOSED: unique in both halves and in exact bijection with the
`topics` / `groups` / `sorts.loci` sections (`fn_universe` covers
`sorts.fns`), so selector recomputation cannot be widened
underneath a certificate. The artifact carries a typed
**`endpoints`** section (round 8) — every bus endpoint at
wire-subject grain with its verb and `via: site | declaration`,
including a DECLARED publisher with no send site, which the V1
site-grained relations never show — and `law.subjects` must equal
exactly the subjects the endpoint and topic sections carry, in
both directions. The endpoint section is NOT a second authority
(round 9), and endpoint identity is TYPED (round 10): every row
carries its wire subject and, when a declared topic covers the
end, the topic identity (`declared_topic` is the model's
syntactic fact — a literal address whose text collides with a
topic display stays a literal, never inferred from strings; a
topic-typed row's subject must equal the topic's wire subject).
Site rows are LOSSLESS (rounds 11–12): each carries its
owning fn/handler, authored site ordinal, and AUTHORED SPAN, with
exactly one occupant per (verb, owner, site). They project onto
the V1 relations at (owner, name) grain under their OWN declared
identity — a topic-covered end by the topic display, a literal
end by its text — and their per-owner SPAN MULTISETS must equal
the span-grained provenance section one-to-one (the V1 rows
dedup; the provenance spans do not, so a typed site row cannot
disappear behind a collision). Declaration rows come from the
typed **`declares_publish`** relation section and keep the OWNING
LOCUS in the compared identity — a declaration cannot move
between loci while a `require publishes` verdict rides on the
original owner. The relation's canonical identity is
`(locus, subject, declared_topic)` (round 13): a literal
declaration whose text equals a topic's wire subject and the
typed topic declaration are distinct endpoint facts, and BOTH
survive regardless of declaration order — the model schema
represents them separately, exactly as `BusSubject::canonical`
and the endpoint judgment distinguish them. Closures never
participate in locus analyzability: they are invisible to the
certificate machinery at every scope (a top-level closure-only
locus already certifies synthetically), so the builder and
admission share one membership rule — only members that produce
function entities the engines could walk count. Known residual, by design: the CONTENT of a site
row (its wire/topic identity) at an unchanged span is not
independently verifiable without the source — the wire identity
of a display-colliding literal lives only in unhashed sections,
so substituting it at the same site is a self-consistent edit;
closing it requires hashing endpoint wire identity (a shape-
identity change).
Analysis coverage is FUNCTION-grained with THREE typed
states (rounds 10–11): `analyzed` (the body was walked),
`summarized` (a behavior-summary row exists — this set IS the
legacy `sorts.fns`, the hashed anchor), and whether a certificate
engine emitted a report (the evidence account). Failure handlers
carry the typed `FunctionKind::FailureHandler` — no consumer
infers handler-ness from a display prefix. The coverage laws — `analyzed ⇔ summarized` (both
directions: a summarized body was walked and a walked body is
summarized, anchoring `analyzed` to the hashed `sorts.fns`);
failure handlers never analyzed; the legacy fn sort equals the
summarized set; an unanalyzed body RETAINS its `UnanalyzedBody`
residue and an analyzed body carries none; and (rounds 14–15)
`LocusDecl::analyzable` DERIVES from the CLOSED ownership
account: every function carries its canonical `owner`
(`Some` for methods, hooks, modes, and failure handlers; `None`
for free functions), `member_of` must be a total, exclusive
partition agreeing with it exactly (a free fn appears in no row;
every other kind in exactly one, at its owner), and a locus is
analyzable iff every relevant owned member (all kinds except
failure handlers) is analyzed, an empty set vacuously so — so a
membership row can be neither deleted nor moved to launder
coverage or group projection. Ownership is coverage-bearing and
folded into the evidence coverage digest. All of these laws live
in ONE shared validator (`ApplicationModel::validate_coverage`),
called by `ApplicationModel::validate` and by
`EvidenceTable::validate` alike: the same coverage state is
lawful (or refused) at the model, the sidecar, and the artifact.
Coverage is BINDING in the sidecar API too:
`EvidenceTable::validate` categorically refuses a certificate
payload for a subject or phase whose typed coverage says no
report exists — fn eligibility requires BOTH `analyzed` and the
hashed `summarized` anchor, and locus eligibility recomputes the
member coverage from the typed `member_of` relation rather than
trusting the flag (round 14) — a matching digest proves the
sidecar repeated the model's bits; this proves its evidence obeys
them. `law.fn_universe` rows carry
all three facts, and the SUMMARIZED subset must equal `sorts.fns`
exactly. `law.loci`'s
`analyzable` flag recomputes from the member coverage: a locus
with executable members is analyzable iff all its non-`on_failure`
members are analyzed, and a MEMBERLESS locus is vacuously
analyzable (no body to walk — every phase contract holds by
absence), so the flag is recomputable in both directions. An
implicit lifecycle phase's certificate must be exactly the
synthetic `holds` with no diagnostics. Coverage also participates
in EVIDENCE IDENTITY: the sidecar carries a coverage digest
(`TopologyShapeV1` deliberately excludes coverage for recording
compatibility), and a judgment refuses a sidecar derived beside
different coverage as stale.
The **`law.issues`** account (round 9) serializes every
table-level law-selection failure — lowering issues (unknown or
cyclic constitutions, illegal adoption, collisions) and the
judgment pre-pass (duplicate claim names) — with source
locations; it participates in `law_digest` (the canonical
fingerprint covers `{issues, rows}`) and in the document verdict
(a non-empty account is `law_failed`), admission validates each
entry like any diagnostic, and the duplicate-name case is
recomputed from the rows themselves — no claim error disappears
between checking and artifact projection. The evidence engine's
`ANALYSIS_SEMANTICS_VERSION` is 3: round 8's producer/judgment
changes (synthetic implicit-phase certificates, report-less
subjects judging `uncertified`) are result-affecting, so pre- and
post-round-8 evidence cannot share an `inputs_digest`. Both digests are RECOMPUTED at
admission: `law_digest` is the canonical-JSON fingerprint over
the law rows (serde-canonical rendering, fnv1a64 — a row edit
under a stale digest refuses), and `inputs_digest` must equal the
consuming binary's analysis-inputs digest (evidence produced
under a different stdlib/analysis snapshot is refused — re-dump).
Evidence is VALIDATED, not presence-checked: every diagnostic
carries a non-empty message and paired provenance resolving to a
known source, a migrated `violated` / `uncertified` row must
retain its judgment's evidence (the countermodel / the residue),
and a violated certificate must retain its diagnostics.
Evidence locations are source-space-honest: a diagnostic whose
span lives in a FOREIGN offset space (stdlib parse space, another
seed) is never re-resolved against bundle sources — the
projection carries a per-diagnostic discriminator, so numeric
overlap with a bundle file cannot misfile stdlib evidence as
application code. Unmigrated rows bridge to the old engines
only where the old walk demonstrably enumerated the row
(module-scoped annotations and ambiguous multi-assert anchors
stay `uncertified`). The legacy `claims` / `lowered` string rows
remain, now PROJECTED from the canonical model path; `semantics`
bumps to 2 because the machine verdicts are stricter in two
documented places (a certificate naming a cyclically-defined or
undeclared effect class is `invalid`, never a vacuous `holds`;
`require attributed` over an unanalyzable body is `uncertified`,
never a fail-open `holds`), and the document `verdict` follows
the machine. A **provenance**
section carries per-edge and per-decl source spans as
bundle-global byte offsets; it is excluded from the hash on
purpose — moving code must not change the shape identity.
A **`topics`** section (unhashed, #399) carries the per-topic
OBSERVATION identity: the wire subject, the canonical payload
shape, and `payload_hash` = FNV-1a/64 over
`wire_subject ++ ':' ++ shape` — byte-identical to the value the
native emitter registers in the runtime observation manifest
(`lotus_obs.c`; the shared implementation is
`hale_types::topic_identity`, which codegen also calls, so binary
and artifact cannot drift). This is the reconciliation ruling for
the compiler-side vs runtime-side identities: they stay separate
(payload field shape does not affect claim evaluation, so it is
not part of the model `shape_hash`), and the artifact is the JOIN
document — a recording/WAL segment carrying `(name, shape_hash)`
matches a row here and thereby names the exact checked topology
it ran under. The `subject` field stays raw (it is the join key);
a subject-less topic registers under its declared — possibly
mangled — local name, so shared topics should declare `subject:`
to fuse across binaries. The exact definition and test vectors
live in iris PROTOCOL.md §4.
`--check-topology-shape <path>` gates the model identity alone —
`shape_hash` — so claim renames and source motion pass while a
changed graph fails. `--check-topology <path>` diffs against a
committed baseline and
fails with a regenerate hint — the `.hale.effects` precedent: an
unreviewed topology or law change fails CI the way an API break
does. v2 scope: every claim verb replays independently over the
exported relations — `forbid`/`only edges` including
through-stdlib reachability, `require`/`count` cardinality,
`cover` via the seed sort, `during` via the phase relation,
`bound` over user classes via labels + weights (dispatch
alternatives group by (from, interface, method) and fold with
max). Remaining compiler-certified: `bound` over built-in classes
(site counting through the stdlib interior, deliberately not
serialized) and any walk past the step ceiling. The derivation
(source → model) remains the trust root.

**§8 — one schema of record** (#392): every fn-grained certificate
— each `@effects` assert, each `@phase_effects` phase contract,
each `@budget` in both families — is REPORTED as the claim form it
is pointwise sugar for, with its verdict, in the artifact's
`lowered` array (`forbid reaches({F}, effects(money))`,
`bound alloc <= N on paths from {F}`,
`only effects {…} on {L} during birth`, …). Rows come from the
same evaluations that gate the build, so the report and the build
cannot disagree. The traversal substrate is already one engine
(the shared call-graph walker plus the shared summary and the
shared fail-closed predicate); the annotation voices keep their
diagnostics.

**Library-tier claims** (#392 thread 2): a TOP-LEVEL `claims { }`
block — legal outside any locus — is the LIBRARY tier: a seed
swears about **itself and its own boundary**, the block travels
with the import, and it re-evaluates in every closing build over
the merged world. Checked standalone, a library's claims evaluate
over its own world; at close, the same sentences quantify over
everything the merge added — a second subscriber the app wires
onto a library topic violates the library's own
`count subscribers(topic T) <= 1`, reported with seed attribution
(`pay::single_settle`, never a mangled symbol) and pointing at the
library's own claim line. World-quantification stays main's: a
seed that declares `main locus` states its law inside that locus,
and its writing the top-level form is a check error. That tier
split is the enforcement surface for "a dependency may not brick
downstream builds with world-claims" — a library can only NAME
what it can see (its own decls and its own imports), and its
traveling block is marked at the mangle stage, which only ever
touches imported seeds. Group and topic references inside a
traveling block canonicalize to mangled decls exactly as group
decls do (#334); claim names are never mangled.

## Constitutions — one authored claimset, many closed worlds (GH #409)

A **constitution** is a named claimset declared outside any main and
adopted by entrypoints with `adopt NAME;` inside a main-locus
`claims { }` block. It is not a new quantification horizon: every
clause is evaluated in the adopting main's own closed world, exactly
as if written there. Authoring is shared, evaluation is not.

- **Placement.** `constitution` is top-level only. `adopt` is legal
  only in a MAIN-LOCUS `claims { }` block — adoption is what fixes
  which world the clauses are evaluated against, and a library seed
  closes none. `adopt` inside a `constitution` is a parse error;
  claimsets compose with `extends`.
- **Composition is union.** `extends A, B` contributes A's and B's
  clauses. A derived constitution may ADD and may not REPLACE: a
  claim name declared by two constitutions in one adopted set is an
  error naming both origins, as is a local main clause shadowing an
  adopted one. This is what makes weakening *unexpressible* — a
  stricter bound is a second named claim that coexists with the
  inherited one, and both are checked. Deciding whether a replacement
  strengthens or weakens would require proving implication between
  claims, which fails open when it is wrong.
- **Duplicates within one constitution are an error.** Diamond
  duplication is resolved by constitution-level traversal, so a
  repeated `(origin, claim name)` can only be two clauses declared
  under one name — silently keeping the first would leave law the
  author wrote unchecked.
- **Diamonds dedup by ORIGIN.** Two constitutions extending a common
  base contribute the base once. Dedup is by originating
  constitution, never by claim name: deduping by name would swallow
  the genuine two-origin collision the union rule depends on.
- **Cycles** in `extends`, unknown constitution names (with a
  did-you-mean), and duplicate constitution declarations are errors.
- **Groups are not implied.** A constitution names group vocabulary,
  and every adopting entrypoint must DECLARE those groups. An
  undeclared name is an unknown-name error, not an empty set;
  `may_be_empty` applies only to a group that is declared and
  resolves to zero members. An entrypoint lacking a component
  therefore writes `group thing = { } may_be_empty;` rather than
  omitting the declaration.

### Identity

A constitution's **display name** is flat and unmangled so
diagnostics cite it as written. Its **identity** is the digest of its
normalized closure — its own `(name, rendered form)` pairs, sorted,
plus its **deduplicated** bases' digests, recursively. `extends A, A`
and `extends A` evaluate identically, so they must digest identically.
Two declarations are the same constitution iff their closures agree.

Identities come from the adoption **traversal**, not from inspecting
which claims were emitted: a constitution that contributes no clause
of its own (`constitution Dev extends Base { }`) has a meaningful
closure and must still be compared. An evaluation reports both its
`roots` — the constitutions named directly, by source `adopt` or by
environment binding — and the `closure` those roots reach.

The distinction is load-bearing across entrypoints: two seeds may
each declare `Core` with different clauses, so a binding by bare name
proves only that each entrypoint had *some* constitution called
`Core`, not that all were evaluated against one claimset.

### Deployment environments

`hale.toml` binds constitutions to deployment targets:

```toml
[claims]
base = "Core"                # every environment carries this

[environments.prod]
constitution = "Prod"        # …and prod adds this
entrypoints  = ["apps/riskgw"]

[environments.dev]
source_only  = true          # explicit: this environment adds none
entrypoints  = ["apps/riskgw"]
```

`--env <name>` injects the base and the environment's constitution as
if the source had written `adopt`. Binding here rather than in source
is what lets one entrypoint satisfy different claimsets in different
environments: it cannot write two conflicting `adopt` lines, but it
can be checked twice.

`[claims] base` is what makes "an environment may add law, never drop
it" true of the mechanism rather than only of `extends` — every
evaluation carries the base by construction. A workspace declaring
environments must decide about a base explicitly: either `base = "…"`
or `no_base = true`. An absent `[claims]` section is indistinguishable
from a misspelled one, and the intended baseline would vanish while
every environment still looked valid.

An environment naming no constitution must say `source_only = true`
for the same reason, and unknown keys in an environment section are
rejected.

The base is compared **workspace-wide** (`base::<name>`); an
environment's own addition is compared within that environment
(`env::<env>::<name>`). Keying the base per-environment meant two
environments with disjoint entrypoint sets never shared a comparison
key, so a base resolving to different closures in dev and prod went
undetected — the mechanism proved consistency *within* each
environment and nothing about the base being shared.

`--env` requires its target to be an entrypoint, whether or not the
environment contributes any constitution: an environment binds law to
a deployment target, and a `source_only` environment with no base
would otherwise check a library and report success.

Combinations that cannot be honoured are rejected rather than
ignored. `--matrix` runs many evaluations, so a per-evaluation
artifact flag (`--dump-topology`, the `--check-*` baselines) has no
single artifact to emit or gate against; and `--env` / `--workspace`
alongside it would select nothing the matrix does not already
enumerate. `--env` with `--workspace` is likewise rejected: a
workspace sweep includes libraries, and an environment binds law to
an entrypoint.

A constitution required by the manifest has no source `adopt` line,
so a diagnostic about it names the environment and the manifest
rather than pointing at a main locus that mentions it nowhere.

A constitution's identity does not depend on the import alias
through which its seed was reached: the same policy seed imported as
`pol` in one entrypoint and `plc` in another is one identity.

`--matrix` checks every declared (entrypoint, environment) pair. It
does not short-circuit; the exit status reflects every pair. An
entrypoint in no environment is an error, and a seed that fails to
PARSE is an unknown entrypoint rather than a non-entrypoint —
otherwise a syntax error would erase a seed from coverage. Within one
environment, all entrypoints must resolve each constitution to the
same closure digest.

The artifact records three things: per-claim `source` (where this
clause came from), and an `evaluation` section carrying the
`environment` label plus `roots` and `closure` with their digests
(which deployment this run certified, and under what law). All are
inside the integrity-covered body — an evaluation context editable
after the fact would certify nothing.

### Source maps and model semantics (GH #408 Phase 0)

Spans in the artifact are **file-local**, resolved through a
`sources` table:

```json
"sources": [{"id": 0, "path": "apps/api/main.hl", "digest": "…"}],
"provenance": { "decls": { "App": {"source": 0, "span": [36, 39]} } }
```

They were bundle-global byte offsets — an artifact of concatenating
the seed's files, meaningful only inside the process that produced
them. A consumer composing artifacts from separately compiled
applications cannot turn `[1204, 1231]` into a location, so no
cross-artifact witness could say where to look.

Paths are relative to the **workspace** (the nearest ancestor holding
a `hale.toml`, else the deepest common ancestor of every source) and
are canonicalized before being made relative. Both matter: an
absolute path makes the artifact machine-specific, and rooting at the
target alone leaves an imported seed — which usually lives outside it
— absolute anyway. The artifact must be byte-identical for the same
sources regardless of the working directory it was produced from,
because comparing two of them is the point.

Each source carries a content digest, so a consumer can tell whether
two artifacts were built from the same text, and can catch a stale
artifact paired with edited source, without the source being shipped.

A span the map cannot place reports `"source": -1` rather than being
attributed to the nearest file: an unplaceable location is more
useful than a confidently wrong one.

**`semantics`** is a version distinct from `schema`. Schema says a row
has these fields; it cannot say that "an interface dispatch fans out
to every conformer" or "unknown implies violation" were the rules in
force when the rows were produced. Two compilers agreeing on the
schema and disagreeing on the semantics would compose artifacts into
a model neither would certify, with nothing in the document revealing
it. A consumer that does not recognise the value must refuse rather
than assume equivalence.

## Fleet composition (GH #408 Phase 1)

A **fleet** is a named deployed system of application *instances* —
not "every main in a repository". `hale fleet check|dump <plan.json>`
composes **artifacts**, never source.

That distinction is the design. A source-merged "super-main" would be
unsound in both directions: an unbound topic is in-process by
default, so merging two binaries makes matching publishers and
subscribers look locally connected when no deployed route joins them;
deploy-time routes that exist only in configuration would not appear
at all; and calls, which cannot cross a process boundary, would
become ordinary reachability. So **matching wire identities establish
compatibility; only an explicit route creates a fleet edge.**

A plan names exact, finite instances and routes. Autoscaling ranges
and wildcard discovery are elaborator inputs, not sealed-plan
contents: a bounded range is not one truth value for a cardinality
claim.

```json
{
  "schema": "1.1", "name": "prod",
  "instances": [{"id": "oms-0", "artifact": "artifacts/oms.json", "labels": ["oms"]}],
  "routes": [{"id": "request", "transport": "unix",
              "publishers":  [{"instance": "oms-0", "topic": "t::OrderRequest"}],
              "subscribers": [{"instance": "gw-0",  "topic": "t::OrderRequest"}]}]
}
```

Composition:

1. **Validate every component.** Integrity before meaning: the
   whole-body `artifact_digest` is checked first, because
   `shape_hash` covers the model half only and cannot vouch for the
   `topics` rows the join reads. Then the `semantics` version, then
   the component's own `verdict` — local law is a precondition of
   fleet admission, and an artifact that fails it is not admissible.
2. **Namespace under the instance id.** `oms-0::Oms::on_intent`. An
   application type is not a deployed instance, and cardinality,
   witnesses and multiple instances of one artifact all need that.
3. **Retain interiors.** Calls stay strictly within one instance;
   there is no cross-process call to invent.
4. **Join on the wire identity** — `(subject, payload_hash)`, never
   the local topic name, which can mean different shapes in different
   applications. Endpoints that disagree are a plan that cannot be
   formed. A topic with no declared `subject:` is not portable and is
   rejected on a route.
5. **Insert route edges explicitly**, so a cross-process hop keeps
   its boundary rather than collapsing into a direct call.
6. **Propagate unknowns.** Uncertainty in a component stays
   uncertainty in the fleet: it may add paths, never delete one.

`fleet_shape_hash` covers the model half — instance identities and
cardinalities, routes and their wire identities, component shape
hashes, the composed relations. Provenance and unknowns stay outside
it, mirroring the application artifact's split: moving source must not
change the identity of a deployment, but a changed transport or a
second instance must.

Unknown keys in a plan are rejected, for the reason they are rejected
in the environment manifest: a misspelled field and an omitted one
look identical to a verifier.

### The workspace's deployments

```toml
[fleets]
production = "ops/fleet/prod.plan.json"
staging    = "ops/fleet/staging.plan.json"
```

`hale fleet check` with no plan checks every declared deployment.
A repository usually has more than one, and checking whichever one
somebody remembered to name is the same partial-coverage problem
`--matrix` solves for entrypoints. Every fleet runs even when an
earlier one fails, and the exit status is the worst of them.

`[fleets]` and `[environments]` are **separate axes** and a workspace
may declare both. An environment binds law to an ENTRYPOINT at the
application tier; a fleet is an ARRANGEMENT of deployed instances.
`production` in one need not mean `production` in the other, and
collapsing them would force every entrypoint's law to be a function
of some deployment it may not even appear in.

There is deliberately no coverage check over plans — unlike
entrypoints, which are discoverable seeds, a plan is an arbitrary
file path, so "every plan in the repository is declared" is not a
question that can be asked without guessing.

### Fleet claims (GH #408 Phase 2)

Claims over the composed model, carried in the plan as normalized
rows rather than source grammar — the plan is an IR, so a generator
can produce one without Hale syntax committing to a deployment
format.

```json
"groups": { "strategies": {"labels": ["strategy"]}, "oms": {"instances": ["oms-0"]} },
"claims": [
  {"name": "orders_pass_oms",
   "forbid_reaches": {"from": "strategies", "to": "gateways", "avoiding": "oms"}},
  {"name": "one_order_authority",
   "count_publisher_instances": {"subject": "svc.order.request", "eq": 1}},
  {"name": "gw_receives_orders",
   "require_subscribes": {"group": "gateways", "subject": "svc.order.request"}}
]
```

A fleet group quantifies over **instances**, by id or by label; its
vertices are every vertex of those instances — the same projection an
application group makes from a locus to its methods, one altitude up.
An unknown name or an empty resolution is an error, never an empty
set.

- **`forbid_reaches`** walks the composed edge set: interior calls
  **including the through-stdlib contracted edges**
  (`calls_via_stdlib`), interior bus hops, and explicit routes.
  Reading only `calls` loses a path the component's own claims can
  see — a handler reaching its publisher through `std::http::Router`
  contributes no direct edge at all. `avoiding` masks a group's
  vertices out, which makes it the interposition form — any surviving
  path is a bypass, and `avoiding` may not name an endpoint of its
  own claim, since masking one deletes the domain being quantified
  over. An instance in **both** the source and target groups is a
  zero-length violation: the source already is the forbidden
  destination.
- **`only_edges`** grants by wire **subject**, not by transport
  address. There are no cross-process calls to grant.
- **`require_subscribes` / `require_publishes`** are structural
  deployment statements: some instance in the group exposes the
  endpoint **and the plan connects it**. Both halves are load-bearing
  — checking only that the endpoint exists lets a plan where nothing
  publishes a subject satisfy "the ledger receives fills", which is
  the one thing that claim is for. "Ledger listens for fills" is
  provable; "every fill is durably booked" is not implied by it.
- **`count_publisher_instances` / `count_subscriber_instances`**
  count instance-qualified **endpoints**, which is a different sort
  from the application tier's declaration count — hence the different
  spelling. Two components can each be individually legal while the
  deployment has two publishers. A count must name at least one of
  `eq`, `min`, `max`: with none of them a claim compares nothing and
  holds against every fleet while reading like law.

A claim names **exactly one** verb. Several verbs under one name
would be judged one at a time and recorded under that name as though
the whole sentence held, so the shape is refused: split it, and each
half gets its own name and verdict.

Route admission validates **roles**, not just topic identity. A
publisher endpoint must publish the subject and a subscriber endpoint
must subscribe it, as its own artifact records — declaring a topic is
not using it, and any component importing a topic module declares
every topic in it. Without this a plan could name a producer that
does not exist and satisfy a law about the consumer side with nothing
feeding it. A plan that misdescribes its components is an invalid
model, refused before any claim is evaluated rather than reported as
a law failure.

**Uncertainty propagates.** A component's `unknowns` are part of the
composed model, not a footnote beside it. A prohibition whose source
can reach a vertex with an incomplete outgoing edge set answers
`uncertified` — that vertex's missing edges could lead to the target,
so the absence is not proved. `uncertified` fails like `violated`;
the distinction is recorded because the repair differs (resolve the
unknown edge, versus fix the program). Two rules keep it usable: a
concrete counterexample wins over a hole, and only a hole the claim's
source can actually reach counts, so an unrelated unknown elsewhere
in the deployment does not poison every law. `uninhabited_interface_call`
is not such a hole — in a closed world an interface with no
conformers has no values, so the site is dead rather than unknown.

Endpoints resolve through each component's `topics` table by wire
subject, never by local name.

A violation renders a **cross-artifact witness**: the instance-
qualified vertices, the route carrying **each** hop, and the source
file each vertex lives in — which is what Phase 0's source maps exist
to make renderable.

```
fleet claim `orders_pass_oms` violated — witness:
  prober-0::Probe::submit  [rogue/main.hl]
  -(route `bypass`)->
  gw-0::Gateway::on_order  [gw/main.hl]
```

### Signed components & attestation (GH #408 Phase 7)

A composition proves a world against artifacts it can read; a
signature proves those artifacts are the ones a key-holder meant.
It certifies **provenance and integrity, never behavior** — the
artifact never claims a message will arrive, and a signature never
claims the code is good. Out of scope, deliberately: a compromised
builder, a malicious compiler, runtime memory tampering.

The scheme is **ES256** (ECDSA P-256 over SHA-256), because the
system already speaks it: it is `std::crypto`'s signature suite,
OpenSSL-backed in the runtime, PEM keys, raw `r‖s` 64-byte
signatures. One algorithm end to end means a Hale program — a
supervisor, a deploy gate — verifies the same sidecar with the
language's own stdlib.

Signatures cover the artifact's **exact bytes**. That is sound
because artifacts are byte-reproducible (schema 1.8), and it is
necessary because the in-band `artifact_digest` is FNV-1a — an
integrity tripwire, not a trust anchor. Nothing signs a digest.
The sidecar is `<artifact>.sig`, one line, `es256:<128 hex>`; an
unknown prefix is a refusal, not a skip.

```sh
hale fleet keygen ops                      # ops.pem (0600) + ops.pub.pem
hale fleet sign app.topo.json --key ops.pem
hale fleet check prod.plan.json --trust ops.pub.pem
```

**Trust is strict when declared.** Passing `--trust` (repeatable),
or declaring keys in the manifest, makes an unsigned or
unverifiable component a composition **error**:

```toml
[fleet_trust]
keys = ["keys/ops.pub.pem"]    # SPKI PEM, manifest-relative
```

There is no `require = true` knob, for the reason `no_base` exists
in `[claims]`: a trust set that quietly admits unsigned artifacts
is law that looks bound and binds nothing. An absent section means
signatures are not checked — the pre-Phase-7 meaning of a
composition, unchanged. Signature verification runs **before** the
integrity digest: provenance before integrity before meaning, each
check covering the bytes the next one reads. The all-fleets form
(`hale fleet check` with no plan) takes trust from the manifest
only; `--trust` there is an error, so one flag cannot quietly
rebind every declared deployment.

The fleet artifact records what admitted each component — unhashed
provenance, like the rest of the `components` section:

```json
{"id": "app-0", "artifact": "artifacts/app.json",
 "sha256": "59ad65d4…", "signed_by": "a88ca61a96ffa055"}
```

`sha256` is the admitted bytes; `signed_by` is the key's identity
(first 8 bytes of SHA-256 over the SPKI DER) or `null` when trust
was not declared — a fact, not an omission, so a reader can tell
"unsigned admission" from "verified under this key".

**Attestation** answers the remaining question: are the executables
this plan deploys the ones the operator hashed? Plan schema 1.1
adds two optional rows per instance, and `hale fleet attest` is
all-or-nothing over them:

```json
{"id": "app-0", "artifact": "artifacts/app.json",
 "binary": "bin/app", "binary_sha256": "de55ee70…"}
```

A missing row is a refusal, not a skip — a partial attestation
would report coverage it does not have. `binary_sha256` is
cryptographic where `artifact_digest` deliberately is not: this
hash is the thing an operator asserts across a trust boundary.
Attestation checks bytes at rest at deploy time; whether a
*running* process is still that binary is runtime territory
(sent/delivered observation, 7b), and the artifact never claims
otherwise.

## Secrets — confine, classify, claim (GH #436)

Hale's answer to secrets is not an analysis. It is the ownership
primitive doing its job, plus one classified operation, plus law.

**1. `@sealed locus L` — confinement.** A sealed locus's `params` are
readable only from inside its own methods. Others may still CALL it;
they may not read its state:

```hale
@sealed locus Signer {
    params { key: Bytes; }
    @effects(is: { secret_use })
    fn sign(m: Bytes) -> Signature { … }
}
```

This exists because loci are otherwise **not** field-encapsulated —
`self.child.key` typechecks from anywhere holding the locus, so
without sealing "the key never leaves the locus that owns it" is a
property you check rather than one that is true. The annotation is
opt-in and breaks no existing program. There is currently **no**
claim form for requiring an annotation, so a constitution cannot yet
demand `@sealed` across a group — a `require sealed(all G)` form is
the obvious follow-up, and the same gap applies to `@supervised`.

Only `params` are confined. Capacity slots and methods are untouched
— sealing confines state, it does not make a locus uncallable, which
is the entire point.

**`@sealed` and `contract { expose … }` cannot be combined.** They are
contradictory claims about the same boundary and sealing wins, so an
`expose` on a sealed locus reads as a permission it cannot grant — the
contract consistency check passes, a matching `consume` binds, and
every use of the field is then rejected. The pair is a check error.

`expose` cannot serve as the sealed allowlist without redefining it:
it is the coordinator/coordinatee surface, so honouring it would grant
reads to an `accept`ing parent while still denying them to a parent
holding the same child as a param — one field, public to one kind of
holder and not the other.

Param **initialization** is deliberately not restricted. A parent
writing `Signer { key: … }` already holds the value it passes, so
sealing the initializer would cost ordinary configuration and buy
nothing. Real secret material should be loaded inside `birth` from a
vault, environment, or file rather than passed in.

A constitution can then demand confinement rather than trusting that
every author remembered:

```hale
constitution SecretBaseline {
    vault_confined: require sealed(all vaults);
    no_plugin_secrets: forbid reaches(plugins, effects(secret_use));
}
```

**2. One classified operation.** The privileged method carries a user
effect class, so every path that can touch the secret is visible on
the call graph — `frontier::infer_effects` propagates it transitively
with no further annotation.

**3. Law.** The domain states who may reach it and how often, using
claim forms that already exist:

`secret_use` is a **compiler built-in** — the stdlib owns the
mechanism, the compiler owns the class identity, the application
states the law. Declaring `effect secret_use;` is an error: user
classes intern per-`Program`, so a stdlib-declared class had no
identity an application's claims could name, and the law over
`std::secret` was silently unenforceable and order-dependent.

```hale
claims {
    no_plugin_secrets: forbid reaches(plugins, effects(secret_use));
    one_op_per_request: bound secret_use <= 1 on paths from handlers;
}
```

A violation names the crossing call:

```text
claim `no_plugin_secrets` violated: `plugins` reaches
  `effects(secret_use)` — witness: `PluginHost::sneaky` -> `Signer::sign`
```

**`std::secret` closes the initialization gap.** Sealing protects the
read side; a parent writing `Signer { key: … }` still holds what it
passes. The stdlib loci therefore take the **name of a source**, never
the bytes:

```hale
locus Gateway {
    params {
        s: std::secret::Signer =
            std::secret::Signer { env_var: "SIGNING_KEY" };
    }
    fn go(m: Bytes) -> Bytes { return self.s.sign(m); }
}
```

`self.s.key` from `Gateway` is a compile error. The key is read during
`birth`, so it exists only inside a sealed locus from the moment it
enters the program and there is no construction site at which the
caller held it. `std::secret::Credential` is the same discipline for a
token or password, with a `fingerprint()` that is safe to log.

Sealing keys off the receiver's resolved type. Since GH #470 every
qualified path naming a Hale-source stdlib declaration resolves to
the mangled name that source declares — the whole surface, not only
sealed loci — so field-existence, method arity, and
interface-satisfaction checking apply to stdlib-typed values exactly
as to user types (the old `Ty::Unknown` tolerance was fail-open:
a wrong-arity method coerced to a stdlib interface unchecked and
corrupted memory at the fat-pointer call). Rust-implemented builtin
handles keep the permissive typing; their path-call names are
validated by the stdlib surface registry.

**The recommended shape**, a pattern rather than an enforced contract:
an ordinary function prepares a request from public data and returns a
closed plan; the sealed locus interprets the plan and performs the one
privileged step. The planner never receives the secret, a handle, or
any secret capability. Prefer a closed operation enum over a callback
— a finite vocabulary is reviewable, a function parameter is a
programmable oracle. Worked end to end in
`crates/hale-codegen/tests/fixtures/examples/secrets-sealed-handler.hl`.

**In the artifact.** `sealed` is a hashed model row (schema 1.9), so a
locus gaining or losing `@sealed` moves `shape_hash` and a
`--check-topology` gate sees it. Confinement is a structural property
rather than only a claim input: a seal changing with no topology diff
would be exactly the invisible security change the artifact exists to
surface.

`require sealed` replays from that row. `require attributed` does not:
it turns on DIRECT effect sites, and the artifact exports inferred
per-fn effect sets rather than the direct/transitive distinction, so
that form is **compiler-certified** — the artifact carries its verdict
and not the facts to recompute it.

### What this guarantees, and what it does not

> The secret lives in a locus that owns it, the domain cannot obtain
> it, the only operations on it are classified, and the domain's
> claims constrain who may reach those operations and how often.

This is **not** information flow and **not** noninterference. Two
residual assumptions, both deliberate:

1. **The sealed locus's own body is trusted.** Sealing stops others
   reading the field; it does not stop the owner returning it. Keep
   that locus small enough to review.
2. **Primitive classifications are compiler-asserted facts.** That
   `crypto::hmac` behaves as claimed is a declaration in the stdlib
   frontier, not a proof — the same trust every effect claim rests on.

Deliberately out of scope, each a separate and stronger theorem:
derivation tracking (nothing derived from the key influences a public
output), control dependence (a constant-time compare still lets the
*verdict* be published), resource uniqueness (nonce counters, DRBG
state), and zeroization.

## Structural & design rules

| Check | Catches | Severity | Enforced by |
|---|---|---|---|
| **CQRS / no-locus-return** | a locus `fn` member whose return type (or `fallible(T)` payload) names a user-declared locus type — returning a managed entity from a method is a Law-of-Demeter / CQRS / Dependency-Inversion violation that also leaks via payload-arena routing | error | `check_no_locus_return` |
| **Stdlib error-type shadow** | a user-declared `type IoError` / `ParseError` / `CryptoError` / `IndexError` / `KeyError` / `EmptyError` whose shape doesn't match the stdlib's, when that error type is reached by a fallible stdlib call | error | `check_stdlib_error_shadowing` (resolve) |
| **Codec purity** | a bus codec whose `encode` / `decode` method isn't pure (codecs may be dispatched off-thread) | error | `check_main_and_bindings` + `purity::infer_purity_for_bundle` |
| **`ring_layout` contract** | a foreign-ring layout declaration that's internally ill-formed — unknown scalar/`len_prefix` repr, missing `framing` (or `byte_records` without a `len_prefix` / `buffer_size`, or `slots` without `slot_size` / `slot_count`), no cursor / a cursor without an `at`, unknown cursor ordering or unit, a missing `magic` / `data_at`, or a `shm_ring(..., layout: N)` whose `N` doesn't resolve to a declared `ring_layout` | error | `check_ring_layout` + `check_main_and_bindings` |
| **`ring_layout` geometry** | a *cross-field* inconsistency that would let a record header land out of bounds or silently corrupt the reader: a header scalar or the cursor overrunning `data_at`, two fields overlapping, a non-power-of-two `align`, a `pad_sentinel` too wide for the `len_prefix`, a `len_prefix` width `> align`, a non-8-aligned `atomic_u64` cursor, or (producer side) a `buffer_size:` that isn't a multiple of `align` | error | `check_ring_layout` + `check_main_and_bindings` |
| **Foreign-ring payload shape** | a `layout:`-bound topic whose payload is neither flat-shapeable (typed mode — read by direct cast, needs a fixed byte layout) nor `BytesView` (raw-frame mode — a bounded view per record, for heterogeneous rings); e.g. a struct with `String` / `Bytes` / variable-size fields. Enforced regardless of `where zero_copy` | error | `check_main_and_bindings` |
| **Cell slot-of-origin** | releasing a `Cell<T>` into a different `(locus, slot)` than it was acquired from | error | codegen |

CQRS is GitHub issue #18 item 6; its three sanctioned remedies
(parent-child + contract, bus mediator, delegation) are named in the
diagnostic. See `spec/semantics.md § Locus method dispatch`.

## Default-on & opt-in analyses

One GitHub issue #18 analysis runs **by default**: item 4 (bus-graph property
checks — *errors*, fail the build). The rest, including item 1 (memory-bound),
are **opt-in** (behind a flag) or deferred. Only item 4 is a build gate; don't
assume the others in a build:

- **Memory-bound proofs (item 1)** — **opt-in**, two ways. The proof is
  opt-in by design: "bounded per epoch" only means something for a
  long-lived process (a daemon, a bus handler, a persistent locus), so a
  script that allocates and exits owes it nothing and pays nothing by
  default — the same descent-curve stance as the `@locality` cache-tier
  budgets (annotation/flag-gated, never automatic). The two opt-in surfaces:
  - **`@bounded locus L { … }`** — the in-source opt-in. A locus annotated
    `@bounded` is checked on every `hale check` (no flag), and a
    `@unbounded fn`/`@unbounded run { … }` inside it is the greppable
    carve-out that silences one body's sites. This is the descent marker:
    the locus that took on long-lived state asks for the proof on itself.
    *(Currently advisory warnings; the intended end state is a hard
    **error** contract once the precision refinements — store-latest vs.
    append, `@form(cap)` composition — drive in-scope false positives to
    zero.)*
  - **The whole-program advisory survey — DEFAULT-ON since 2026-07-02**
    (the M3 stage-5 flip, after a full-corpus audit triaged all 402
    warnings: every true positive preserved, every residual false positive
    in a documented accepted class — see
    notes/unbounded-alloc-audit-2026-07-02.md). Flags every site
    regardless of `@bounded` (a `@unbounded` fn is still suppressed);
    run-to-exit programs (a `main` with no `run` loop and no bus handler)
    warn nothing — a script owes the proof nothing. Warnings print but
    never fail the build. **`--no-warn-unbounded-alloc`** is the opt-out;
    `--warn-unbounded-alloc` is accepted-and-ignored (the former opt-in
    spelling).
  `--dump-alloc-summary` prints the raw per-fn summary. A per-method allocation summary + call-graph
  escape/loop dataflow — with **escape-awareness** (a non-escaping local in
  a per-message handler is reclaimed at the per-delivery method-scratch
  destroy, so it isn't flagged), call-result escape tagging, and
  **loop-ranking** (a `while v < N` const counter is proven bounded) — flags
  a value allocated in a per-message handler / unbounded loop that escapes
  and **accumulates until the locus dissolves**. A whole-value replace
  (`self.f = Struct{…}`) genuinely leaks (the arena bump-allocates a fresh
  value each time); the fix is **in-place mutation** (`self.f.x = v` /
  `self.a[i] = v`), a capacity-bounded `@form` (`ring_buffer` / `lru_cache`
  / a `capacity` slot), the bus (reclaims per dispatch), or a per-iteration
  child locus. It also flags an **insert into a growing collection** —
  `v.push(x)` / `m.set(x)` where the receiver's declared type is a
  `@form(vec)` / `@form(hashmap)` locus — in an unbounded context; the
  backing buffer grows with population and frees only at dissolve. A
  `@form(ring_buffer)` / `@form(lru_cache)` is cap-bounded and excluded.
  (Detection reads *declared* receiver types — params, typed `let`s, locus
  param fields — not inferred ones.) Zero corpus false positives. Type-aware
  String-concat sites and untyped-receiver collection inserts remain
  deferred. See `notes/memory-bound-proofs.md`.
- **Hot-path allocation contract — `@budget(alloc_per_call = N)`** (2026-07-16).
  The dual of `@unbounded`: where `@unbounded` acknowledges intentional
  unbounded allocation, `@budget` declares an *opt-in per-call ceiling* and
  the compiler **enforces it as a hard error**. On a `fn` (free or method),
  `@budget(alloc_per_call = N)` asserts the fn performs at most `N` arena
  allocations per call. The check reuses the item-1 allocation summary +
  call graph: it counts the arena-allocating literals / `@form` inserts it
  can see, **transitively through resolved (bundle-local) callees**, plus
  the known-allocating `recv` family (`recv` / `recv_bytes` /
  `recv_with_source` — the same set the hot-path lint flags); a
  loop-nested allocation, or a call to an allocating fn inside a loop, or
  recursion, is **unbounded per call**. `N = 0` is the zero-alloc
  certificate — the strongest form, for a per-datagram handler or decode
  helper the runtime calls on the hot path with a guarantee it touches no
  arena. Opaque calls other than the `recv` family are outside what the
  budget sees (the same boundary the escape analysis draws); pair the
  contract with `recv_into` + a reused `BytesBuilder`. fn-only; mutually
  exclusive with `@unbounded`. A violation reports the measured count and
  points at every offending allocation with the fast-path fix.
- **Effect assertions — `@effects(...)` and its `@no_*` sugar**
  (GH #265, 2026-07-29). `@budget`'s discipline generalized from
  allocation *count* to effect *classes*: an opt-in contract at a root
  the author cares about, inferred everywhere else, enforced as a hard
  error. **One surface, one engine, one classified frontier** — not a
  family of independent flags.

  The general form is `@effects(...)`:

  ```hale
  @effects(none: {syscall, block})   fn decode(b: Bytes) -> Msg { ... }
  @effects(none: {time})             fn backoff(n: Int) -> Int { ... }
  @effects(publish: {OrderFill})     fn route(o: Order) { ... }
  ```

  `none: {…}` forbids effect classes; `publish: {…}` declares the
  allowed publish set (exact, because the topic set is closed). The
  classes are `syscall`, `block`, `time`, `entropy`, `env`, `ffi`,
  `publish`, `spawn`, `recursion`.

  The `@no_*` family is **documented sugar**, desugared at parse time
  so the checker has exactly one shape to interpret (a flag can never
  drift from the general form):

  | sugar | means |
  |---|---|
  | `@no_syscall` | `@effects(none: {syscall})` |
  | `@no_block` | `@effects(none: {block})` |
  | `@no_ffi` | `@effects(none: {ffi})` |
  | `@no_publish` | `@effects(none: {publish})` |
  | `@no_spawn` | `@effects(none: {spawn})` |
  | `@no_recursion` | `@effects(none: {recursion})` |
  | `@deterministic` | `@effects(none: {time, entropy, env})` |

  All stack with each other and with `@hot` / `@budget(...)`, so the
  full hot-path certificate is one line:
  `@no_block @no_syscall @deterministic @no_recursion @hot
  @budget(alloc_per_call = 0)`.

  **Where each class's truth comes from.** `syscall` / `block` /
  `time` / `entropy` / `env` are queries against the **fully
  classified stdlib registry** — all 327 surface entries carry an
  `EffectSet` in `hale-types::stdlib_surface`, with zero unclassified
  residue (pinned by a test). The classification distinguishes
  *reading* an effect source from operating on a *supplied value*:
  `time_from_unix(n)` is deterministic while `monotonic_ns()` is not;
  `http::parse_request` is pure while `http::get` is blocking I/O.

  **Incompleteness fails closed, in both of its forms.** An entry
  present but *unclassified* is treated as may-do-anything and
  violates every assertion. So is a `std::` path with **no registry
  row at all** — the two used to be asymmetric, and that asymmetry
  was a soundness hole: an unclassified row failed closed while a
  whole unregistered namespace read as pure, silently certifying
  calls into it. "Absent" and "unknown" are the same claim, and
  neither can be certified.

  The **language builtins that write to a stream** (`println`,
  `print`, `eprintln`, `eprint`) are syscall-class. They are not
  `std::` paths, so they once sat outside the frontier entirely and
  a `@no_syscall` fn could print freely — while the diagnostic for
  `std::io::fs::*` described the syscall class as covering "stdio".
  Writing to a stream is a `write(2)`; it can block, and a hot-path
  certificate that permits it is not certifying what it claims.

  `ffi` matches
  bundle-local `@ffi` declarations. `publish` and `spawn` are
  **syntactic** — `Topic <- v` and `Child { … }` are effects the
  language expresses directly, recorded as effect *sites* on the
  summary rather than as call edges. `recursion` is a graph property.

  **Reachability follows handles, not just paths.** A call made on a
  value — `reader.slurp()`, `resolver.get(…)` — is a real edge, and
  the analysis resolves the receiver's declared type to walk into
  the method body. This includes the part of the standard library
  that is **written in Hale** (`hale-stdlib`): those bodies are fed
  to the callgraph, so their effects are *inferred from the
  implementation* rather than declared in a table that can drift.
  Witness paths through a stdlib locus are rendered in the public
  spelling (`std::cli::Resolver::get`), never the internal mangled
  name.

  This matters more than it sounds: the locus-with-methods shape is
  the idiomatic way to do I/O in Hale — the same shape a violation
  diagnostic recommends as the fix. Moving an effect behind a locus
  the asserting fn still calls does not make it unreachable, and the
  checker must not confuse the two.

  **Diagnostics carry the witness path** — the call chain from the
  asserting root to the offending leaf, which `@budget`'s fixpoint
  structurally could not produce:

  ```
  effect assertion violated: `on_tick` must not reach `block`, but reaches
    on_tick -> helper -> nap [std::time::sleep — a blocking operation …].
  ```

  plus a second diagnostic at the leaf itself.

  **Boundaries:** opaque callees outside the classified frontier are
  not seen (the same soundness boundary the escape analysis and
  `@budget` draw); `@ffi` labels are trusted, not verified; a computed
  publish subject cannot be proven in-set and is reported. `@no_panic`
  remains a separate track — disposition coverage + index-op
  selection, not leaf reachability.
- **Quantitative budgets — `@budget(<dim> = N)`** (GH #265 step 5,
  2026-07-29). `@budget` counts more than allocations now; dimensions
  compose in one clause, comma-separated:
  - `stack_bytes` — worst-case stack depth as a **DAG longest path**
    over estimated frame sizes. Acyclicity is the precondition, which
    is why this pairs with `@no_recursion`: a cycle reports
    *unbounded*.

    **What the estimate rests on** (#326, examined 2026-08-03). Frames
    are estimated from declared shapes: 32 bytes of call overhead, 8
    per parameter, 8 per local. That unit is close to right *because
    of Hale's memory model, not by luck* — fixed arrays, structs and
    string/bytes buffers are arena-allocated, so a local is a pointer
    and almost nothing but scalars is ever on the stack. The same
    estimator in C would be wrong by orders of magnitude. The premise
    is pinned by tests (`hale-codegen/tests/stack_budget_premises.rs`)
    precisely because it is load-bearing: if a shape ever became
    stack-allocated, the estimate would silently under-count by the
    size of that shape.

    Inlining, the other obvious worry, cuts the safe way: the model
    charges `CALL_OVERHEAD` per level of call depth and inlining
    removes levels, so it strictly reduces the term the model spends
    most of its budget on.

    **What the estimate does NOT cover.** Storage the optimizer
    introduces is invisible to any source-level model — register
    spills above all. We build `-O3` with `target-cpu=native`, so on
    an AVX-512 host a spilled vector register is **64 bytes** against
    a model whose unit is 8, and a vectorized loop can consume
    hundreds of bytes with no source local to explain it. Inlining
    amplifies this by merging live ranges, which is what produces
    spills — the risk is merged *pressure*, not merged locals.

    So: the bound is a **structural** estimate over declared shapes,
    not a machine-level guarantee and not WCET. It is sound against
    everything the frontend can see and silent about everything the
    backend adds. Treat it as a bound on *program shape* — call depth
    and declared locals — and not as a promise about bytes of hardware
    stack. Settling the latter needs post-codegen measurement
    (LLVM's `.stack_sizes` section), which is not wired up; `hale
    check`'s diagnostic already says "estimated from declared shapes",
    and this entry now matches that honesty rather than exceeding it.
  - `block_points` — how many blocking operations one call may reach
    (`0` is `@no_block`; `1` is "may wait once, on its own socket").
  - `publish` — publishes per call. `@budget(publish = 1)` **is** the
    exactly-once-reply contract the issue sketched as `@replies`,
    falling out as a count rather than a bespoke analysis.
  - `fanout` — transitive subscriber **deliveries** one call causes,
    read off the bus graph. This is the amplification/backpressure
    property no per-fn count reveals: a handler publishing to a
    200-subscriber subject amplifies 200×.

  A contributor inside a loop saturates to unbounded, matching
  `alloc_per_call`'s per-call semantics.
- **Phase-indexed effects — `@phase_effects(...)` on a locus**
  (GH #265 step 6, 2026-07-29). The lifecycle model expresses what a
  function-level effect system cannot:
  `@phase_effects(birth: {alloc}, run: {})` **is** the DO-178 "no
  dynamic memory after initialization" discipline, stated directly
  rather than assembled from two unrelated flags. Each phase names
  the classes it may perform (`alloc`, plus the `@effects` classes,
  **including declared user classes** — #392 closed the contract
  over the live class universe like `only:`, with the same
  atomic-only complement; the hardcoded built-in list was the
  documented deficiency); a phase omitted is unconstrained, a phase
  with `{}` forbids everything. Phases resolve to lifecycle hooks
  (`birth`, `run`, `drain`, `dissolve`, `accept`, `release`) or to
  any member fn / handler by name.
- **`@no_panic` — disposition coverage** (GH #265, 2026-07-29).
  Deliberately *not* an effect class: this is a syntactic property of
  a body, not a query over the classified frontier. A fn asserting
  `@no_panic` must have no reachable trap — no explicit `violate`, no
  fallible expression dispositioned `or raise` (which propagates
  rather than handles), no trapping index form. `or discard`, a
  substitute value, and `or handler(err)` all satisfy it.
- **The conformance loop — checking the checker** (GH #265 step 7,
  2026-07-29). Static classification is a *claim*; the running
  binary is the oracle. `crates/hale-codegen/tests/
  effects_conformance.rs` compiles programs carrying assertions,
  runs them, and samples the runtime's own counters
  (`std::diag::syscall_count` / `heap_alloc_count`) around the
  certified call: a fn certified `@no_syscall` that performs a
  syscall, or `@budget(alloc_per_call = 0)` that allocates, is **a
  caught soundness bug in the analysis itself** — the one class of
  defect that "the checker says what I expect" testing cannot find.
  A negative control asserts the oracle detects the effect when it
  genuinely happens, so the conformance checks can never pass
  vacuously. Same philosophy as GenMC-in-CI, applied to effects.
- **The `.hale.effects` manifest + CI gate** (GH #265 step 7). The
  manifest is a **behavioural fingerprint**: every fn's declared
  contracts *and* its INFERRED effect set (`does={…}`), stable-sorted
  for diffs. `hale check <target> --dump-effects-manifest` writes it;
  `--check-effects-manifest <path>` diffs against a committed
  baseline and **fails the build** when the program's effects change,
  printing which fn gained or lost what. That catches the case
  annotations cannot: a handler that quietly starts doing filesystem
  I/O shows up as `+ Api::emit … does={syscall,publish,alloc}` in
  review even though no annotation changed. Regenerate deliberately
  with the dump flag when the change is intended. Rows recurse
  through `module` declarations (GH #296 review: a module-contained
  fn absent from the rows was invisible to both this gate and
  `hale replay`'s safety admission); an inline-module fn the
  callgraph summarizer cannot yet resolve renders `does={unclassified}`
  — fail-closed, and scoped to modules so non-module manifests are
  unchanged.
- **Corpus-wide conformance** (GH #265 step 7). Beyond the per-test
  runtime oracle, a sweep over the whole in-tree `.hl` corpus asserts
  three properties everywhere real code lives: no reachable stdlib
  call lands on an **unclassified** registry row (an unclassified
  leaf silently weakens every assertion over it, so frontier
  completeness must not rot as the corpus grows); inference is
  **deterministic** (the same program inferred twice yields identical
  sets — otherwise manifests diff spuriously); and every corpus
  program carrying an assertion **satisfies** it.
- **The `.hale.effects` manifest** (GH #265 step 7). `effect_manifest`
  / `render_effect_manifest` emit the whole program's declared
  contracts in a stable, sorted line format alongside `.hale.topo`.
  Declaration-only at v1 (inferring a full effect set per fn is the
  effect-rows-on-function-types slippery slope the issue defers);
  what it buys today is that an effect **regression** — a handler
  that quietly lost a contract — shows up as a one-line diff in
  review, the way an API break shows in a `.d.ts` diff.
- **Cross-actor causality — `@effects(causes: {…})`** (GH #265,
  2026-07-29). The call graph stops at a publish; the **bus graph
  continues**. Because Hale's message graph is declared over a closed
  topic set, "this handler, by publishing `orders`, can transitively
  cause a filesystem write in the audit subscriber" is a checkable
  property — the compiler walks publish sites → subject → each
  subscriber's inferred effect set. The diagnostic names the causal
  path (`Api::handle -> subject Orders -> Audit::on_order`). Only
  effects reached THROUGH the bus are reported; direct effects are
  the `none:` form's job. Actor systems without a declared message
  graph structurally cannot offer this.
- **Backward causality — `@effects(depends: {…})`** (#330,
  2026-07-31). The dual of `causes:`. `causes:` walks the bus graph
  forward; nothing walked it backward, so an independence claim
  between two parts of a bus graph was unenforceable — a dependence
  routed through one republishing intermediary is invisible in every
  declaration on the depending locus, whose `bus {}` block names only
  the innocent subject it directly subscribes to. `depends:` is a
  COMPLETE declaration of the subjects that may transitively reach any
  of the locus's handlers, and the diagnostic names the path
  (`subject SumLookup -> Launderer -> subject Recalled ->
  StatedCarry`). **Locus-level**: dependence enters through
  subscriptions, which are declared per-locus, so a fn-level
  `depends:` is a parse error rather than a silent no-op. Opt-in on
  measured grounds — over a real application (428 topics, 114 loci)
  transitivity adds nothing beyond the `bus {}` block for 87% of loci,
  so a mandatory form would be redundant far more often than
  informative. The closure is over the **bus graph**: influence
  travelling outside it (see shared state below) is not part of it.
- **User-declared effect classes — `effect NAME;` and
  `@effects(is: {…})`** (#345, 2026-08-01). A program may name its own
  effect classes and have them propagated by the same engine, with the
  same witness paths:

  ```
  effect money;

  @effects(is: {money})
  fn charge(cents: Int) -> Int { … }

  @effects(none: {money})
  fn price(n: Int) -> Decimal { … }   // violates if it reaches charge
  ```

  Grounded exactly like a built-in. The objection to user effects is
  that they have no frontier — that `@no_money` could only mean "no fn
  somebody remembered to annotate". But effects worth checking are
  about interaction with the outside, and that IS the frontier: money
  moves when the processor is called, the ledger row is written, the
  settlement is published. So `is:` adds rows to the classification
  that already exists rather than introducing a second kind. **The
  compiler owns propagation; the program owns classification** — the
  same split the stdlib registry has, with a different owner.

  Classes are interned as indices and occupy the free bits above the
  ten built-ins — `EffectSet` is a u64, so 54 are available.
  Overflow is an **error at the declaration**, not a saturating
  no-op: a class with no bit unions as PURE, so `none: {…}` on it
  would certify a fn that reaches a declared source. The analysis
  fails closed everywhere else and must here too.
  Classes cross seed boundaries. Each seed interns its own names from
  zero, so the merge unions the tables and rewrites each seed's
  indices before concatenating items — without that, two seeds' class
  0 share a bit and a `none:` on one is checked against the other.
- **An indirect call fails closed** (#353, 2026-08-03). A call
  through a function-typed parameter reaches the call graph as an
  unresolved callee, indistinguishable from a call to an unknown free
  fn — which contributed nothing. So `@no_syscall` on a fn whose body
  is `return f(v);` typechecked while the program performed the
  syscall, and `@budget(alloc_per_call = 0)` leaked identically. Every
  certificate the language offers ran through one hole, function
  pointers being the first genuinely open-world construct in the
  language. An indirect call is now treated as **may do anything**:
  unclassified for the effect classes, unbounded for the quantitative
  dimensions. Deliberately conservative rather than exact — Hale is
  whole-program and closed-world, so the target set IS enumerable and
  exact resolution is possible, but a certificate wrong in the safe
  direction beats one wrong in the other, so the conservative form
  lands first and precision becomes an improvement rather than a
  correctness fix.
- **Closed effect contracts — `@effects(only: {…})`** (#354,
  2026-08-03). The dual of `none:`. `none:` forbids a listed set and
  permits everything else, which makes it **rot**: expressing "this
  handler only allocates" requires enumerating every other class, and
  adding a class to the language silently widens every such contract —
  the annotation still reads "only alloc" and no longer means it.
  Nothing fails; the certificate quietly weakens. `only:` states the
  permitted set and is checked against the **complement computed at
  check time** from the live class universe (the ten built-ins plus
  every declared user class). Nothing is written down that can go
  stale, so a class declared after the contract was written is outside
  it automatically. Rendered separately from `none=` in the manifest —
  a reader must be able to tell a closed contract from an open one,
  because they age differently.
- **Composed effect classes — `effect NAME = { A, B };`** (#354,
  2026-08-03). A class may be DEFINED as the union of others. A
  composed class owns **no bit**; its mask is its members', which
  yields both useful directions with no additional analysis:
  forbidding `io` tests against `syscall|block` and catches either,
  and a fn that reaches a syscall carries `io`. This also repairs a
  fail-open in `@deterministic`, which desugars to a hardcoded
  `none: {time, entropy, env}` and therefore could not see a
  user-declared class: `effect wallclock = { time };` puts the time
  bit in the class's mask, so the existing contract catches it with no
  new mechanism — and it stays opt-in, since an atomic class like
  `money` is correctly *not* swept into determinism. A definition
  cycle resolves to no effect at all, so every contract naming it
  would hold vacuously; cycles are rejected.
- **Synchronized access is blocking and non-deterministic** (#340/#341,
  2026-08-01). A `sync`-bearing form takes a lock, so a call reaching
  one contributes `block`, and a read through one defeats
  `@deterministic` — another pool can change the value between two
  calls with identical arguments. Attributed because **placement is
  not static**: once placement can be swapped at runtime, whether a
  mutex ever contends is undecidable at compile time, so a certificate
  reading "never blocks, we are single-pool today" would be
  invalidated by a later swap. A form with no `sync` discipline takes
  no lock and stays certifiable.
- **Supervision coverage — `@supervised`** (GH #265, 2026-07-29). A
  locus marked `@supervised` asserts that every locus in its subtree
  has a failure policy in scope — an `on_failure` on itself or an
  ancestor. A locus with children and no policy above it is reported
  by name: a failure there has nowhere to go. The declared ownership
  tree makes this a tree walk; it is the static supervision-coverage
  property the actor world has wanted for decades.
- **`@secret` params — a LINT, not a certificate**. A parameter
  declared `@secret name: T` reaching a bus publish or a log / file
  sink in the same fn body is reported as a **warning**.

  It is not a proof. "Must not reach a sink" is a whole-world
  property; this is a local walker over one body that follows no
  calls and tracks no aliases, and the fragment it walks is narrower
  still — `then` branches but not `else`, no `match`, no `let`, no
  assignment. Anything outside that fragment is absent from the
  result rather than surfacing as `uncertified`.

  The default traversal stays narrow deliberately: widening it fails
  programs that compile today, and a lint that grows teeth in a point
  release is a userspace break even when every new finding is a real
  bug. **`hale check --strict-secret`** runs the
  widened walk: every branch (including `else`, `else if`, and both
  block and EXPRESSION `match` arms), alias propagation through `let`
  and tuple destructuring, and `uncertified` for anything it cannot
  follow — an unfollowed call, a field store, a return. It is loud,
  which is the honest signal that one body's reasoning is not a
  containment proof.

  The expression walk is **exhaustive by construction**: it has no
  catch-all arm, so adding an `Expr` variant fails the build rather
  than opening a laundering route. Branch taint is shared rather than
  merged per-branch — imprecise in the over-tainting direction.

  For a guarantee rather than a lint, see § Secrets below.
- **Inferred effect sets + symbolic cost** (GH #265, 2026-07-29).
  `frontier::infer_effects` computes the transitive effect set of ANY
  fn — no declaration required — which is what feeds the causality
  check and lets the `.hale.effects` manifest report inferred sets
  alongside declared contracts. (Effect rows on function *types*
  remain the deferred slippery slope; a manifest is a report, not a
  type.) `cost_expression` renders a structural cost —
  `O(n^k)` in nesting depth with a step estimate — explicitly **not
  WCET**: the first-filter triage shape, meaningful only for a fn
  already proven bounded.
- **Placement-implied contracts — the assertion you don't write**
  (GH #265, 2026-07-29). A locus placed
  `cooperative(pool = X) where async_io` shares that pool's single
  worker; a blocking operation reachable from one of its handlers
  holds the worker and stalls every other locus on the pool. Since
  the placement already declares the intent, the compiler enforces it
  **with no annotation at all**: an unannotated handler on an
  async_io pool that reaches a blocking call gets a warning naming
  the chain and both fixes (move the work to its own pool, or assert
  `@no_block` to have it enforced as an error). Advisory rather than
  hard, because a lone locus owning its pool may block deliberately;
  writing an explicit assertion suppresses the advisory (the author
  is engaged, and the enforced error replaces it). This is the class
  of bug that shipped as a downstream latency mystery — a sleeping
  handler holding an engine pool — now visible at compile time.
- **Hot-path allocation lint — default-on advisory** (2026-07-16). Two
  loop-scoped anti-patterns get a **warning** (never a build failure), so
  the allocation-free shape is the path of least resistance rather than
  expert folklore: (1) a **locus** (its own arena / heap buffer) or a
  `std::bytes::BytesBuilder` instantiated inside a loop — hoist it to a
  reused field; (2) an **allocating `recv`** (the `recv` family) in a loop
  — use `recv_into` with a reused `BytesBuilder`. Both accumulate in the
  method scratch until the enclosing method returns, and a `run()` read
  loop never returns. Loop-scoped keeps the signal clean (per-iteration is
  the unambiguous case); a plain value struct/type literal is not flagged
  (only loci and heap-bearing builders), and a per-invocation
  instantiation outside a loop reclaims at method exit. This is the
  conservative default advisory; `@budget` is the strict opt-in contract
  built on the same intent.

  Gap D extensions (2026-07-17): (3) a locus / `BytesBuilder`
  instantiated **anywhere in a bus handler** (not just a loop) — a
  handler runs per message, so a per-call instance is the
  ~4.5 KB/frame class; hoist it to a reused field. (4) **`accept`
  without `release` on a daemon-shaped locus** — declaring
  `release(c: C)` marks `C` a flow child (reclaimed when its `run()`
  completes); without it every accepted child is RESIDENT until the
  parent dissolves, so a parent whose `run()` loops forever (literal
  `while true` — the deliberately narrow daemon signal) grows
  O(accepted children). Run-to-exit accept examples stay silent.
- **`@hot` — hot-path certification** (Gap D, 2026-07-17). The layered
  escalation between the default advisory and `@budget`'s counted
  ceiling: `@hot fn` certifies "this is a 10k/s-class path" and (a)
  **promotes the hot-path lint's findings inside that fn to hard
  errors** (prefixed `@hot:`), and (b) enables two stricter,
  perf-only hints that would nag as defaults: `.snapshot()` /
  `.finish()` in a loop or handler (each call copies the builder's
  full contents — prefer the zero-copy `.view()` / `.text_view()`),
  and a whole-struct replace of a direct self-field (post-Gap-A the
  replaced String clones retire, so this is no longer a leak — but
  each store still pays a clone + retire per heap field where
  in-place scalar mutation is allocation-free). fn-only; stacks with
  a following `@budget(...)`:
  `@hot @budget(alloc_per_call = 0) fn send(...)`.
- **Anchor-retirement verdict flip** (Gap D, 2026-07-17). The item-1
  survey's model learned what Gap A's runtime now does: a whole-field
  `self.<f> = Struct { ... }` replace of a struct whose fields are all
  scalar / `String` reclaims at the enclosing method's activation
  boundary (the struct bytes memcpy in place; replaced String clones
  retire and recycle — RSS-validated flat over 1M replaces), so such a
  site invoked unboundedly is no longer reported. The conservative
  verdict stays for: structs with `Bytes` / nested compound / array
  fields (those leaves don't retire yet), stores directly inside a
  `run()`-loop (no activation boundary — pending retires never
  flush), and scratchless owners.
- **Resource-budget tracking (item 5)** — fully shipped, opt-in. A static
  **count** of pinned threads / cooperative pools / bus subjects /
  fd-acquisition sites (fd-opening calls *and* held-fd `Listener` /
  `Stream` instantiations) via `hale check --dump-resource-budget`; a **CI
  ceiling gate** `--check-resource-budget <file.toml>` (fails the build
  when a count exceeds a declared ceiling); and **fd-leak detection**
  `--warn-resource-leak` (an fd-acquiring call whose result is stored
  resident in an unbounded context). See `notes/resource-budgets.md`.

  The ceiling file is TOML; every key is optional (an absent key leaves
  that resource unconstrained, an unknown key is an error):

  ```toml
  pinned_threads    = 4
  cooperative_pools = 2
  bus_subjects      = 16
  fd_open_sites     = 8
  ```
- **Closure-assertion lifting (item 3)** — scoped, deliberately parked.
  The tractable case (constant assertions) is already handled: typecheck
  rejects any closure whose assertion observes no runtime-varying value
  (pure literals *or* const arithmetic), so there are no constant closures
  to lift. The only liftable closures are ones provable from producer
  arithmetic (symbolic execution) — low-leverage for a niche feature, not
  built. Closures still verify their (runtime-observing) invariants at
  *runtime*. See `notes/closure-lifting.md`.

The item-1 whole-program survey and the hot-path lint are advisory
(warnings). The one build-failing *allocation* gate is opt-in:
`@budget(alloc_per_call = N)` on a fn — you ask for the ceiling, and a
violation is a hard error. fd and thread bounds remain advisory / CI-gated
(item 5), not automatic build failures.

Item 2 (race-completeness for substrate primitives) is a *substrate*
quality bar, not a user-facing check: it model-checks the runtime's own
concurrent primitives under all C11 interleavings with GenMC, run as a
standing CI gate (the `genmc` job). Every substrate primitive with a
cross-thread synchronization surface is now modeled: the lockfree
hashmap's enter/drain/grow protocol, the pinned-locus mailbox monitor,
the cooperative-pool bus queue's conditional lock, and the arena
subregion-slot freelist lock. (The per-thread chunk pool needs no model
— it is `__thread`, with no cross-thread access.) See `verification/`.
