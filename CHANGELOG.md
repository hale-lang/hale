# Changelog

Behavior changes by release. The canonical spec lives in
[`spec/`](./spec/) — each file there represents *current*
behavior.

---

## Unreleased

### A second `accept` clause is an error, not a silent overwrite (GH #525)

A locus accepts exactly one child type (`spec/types.md` F.11,
single-accept-type per parent). The checker stored that type in a
singular slot and a second `accept(c: U)` clause simply replaced
the first — no diagnostic, and the parent written to own two child
types quietly owned one. It is now a typecheck error naming both
clauses (the second carries the first as a related span), and the
first clause stays the locus's accept type. Surfaced by the DNA
Phase 0 design (#521), whose first fixture needs a Step to own
both Work and delegated Tasks and must fail loudly there.

## v0.19.2 — helpers get their speed back (2026-09-04)

### The caller-arena publish is gated on allocation, not on syntax (GH #522)

`fn_modular` — two-deep calls through opaque function pointers —
has been **29% slower since v0.14.0**, and the bench suite never
said so: its tolerance band is 30%, so a 29% regression passed
with a point to spare, at 1% measurement noise.

Bisected across released binaries on one machine, one session:

| build | fn_modular |
|---|---|
| v0.11.3 – v0.13.0 | ~18 ms (flat) |
| **v0.14.0** | **23.25 ms** |
| v0.18.0 – v0.19.1 | 23.24 ms (flat) |

Disassembling `outer()` from identical source built by each
compiler showed 5 instructions becoming 15, the difference being
`call <lotus_set_caller_arena>` — #375's caller-arena publish,
landing inside a 10M-iteration loop.

#375 fixed a real use-after-free and that trade was right. The
problem was its gate, which asked whether the body *could* read
the TLS by substring-searching `{:?}` of the AST for `Call {` or
`Struct {`. Any call at all armed it, including a call through a
function pointer that allocates nothing.

The gate now also consults `non_allocating` — the same fixed-point
classifier that already lets such a fn skip its m49 subregion, and
one that reasons about function-pointer params instead of giving
up on them. The TLS exists for allocation sites to read, so a body
that provably never allocates has no reader to heal. `fn_modular`
returns to **17.3 ms**, at or below every measurement since
v0.11.3; no other bench moves.

The safety direction is pinned in both places it can break:
`caller_arena_tls_unwind.rs` (the #375 reproducer, green under
`LOTUS_ASAN=1`) and a new `caller_arena_publish_gate.rs` that
asserts an allocating body still publishes **on entry** — which
required getting the test right twice, since the first version
asked about functions LLVM had inlined away and the second counted
call-site publishes that happen regardless of the prologue.

## v0.19.1 — print what you meant (2026-09-04)

### Logging ergonomics (GH #469)

f-strings, composite rendering, format specs, and `std::log`'s
structured half. The discovery that framed the issue: f-strings
shipped in v1.x-10 and were documented nowhere, so in practice
everyone wrote `println("x=", x)` and the ones who guessed
`"x={x}"` got the braces printed back at them.

**Interpolation renders composites.** `f"{point}"` was a type
error advising you to "render struct/locus fields individually" —
which is a fair description of the workaround and not of anything
anyone wants to do mid-debugging. Structs, tuples, fixed arrays
and `bounded` now render recursively:

    Reading { sensor: "t-1", at: Point { x: 3, y: 4 } }

A String **inside** a rendered value is quoted, so a value
containing a comma still reads as one value; a String on its own is
not (`to_string(s)` stays identity). `bounded` renders its live
count, not its capacity. Three exclusions are deliberate: a
**locus** never renders (it is flow, not shape — and rendering one
would read back the `params` that `@sealed` exists to confine),
`Bytes` never renders (the useful form is a choice), and neither
does an unsized `[T]`.

`println` had a *second* copy of the printable rule, on its own
printf-building path. It moved too — the corpus check/build
agreement gate (#512) caught the half-landed version, which is
what it is for.

**Format specs.** `f"{x:>8.2}"`, with
`[[fill]align][width][.precision][kind]`:

    println(f"[{n:6}]");      // [    42]  numbers pad left
    println(f"[{name:6}]");   // [ada   ]  text pads right
    println(f"[{n:0>6}]");    // [000042]
    println(f"{ratio:.2}");   // 3.14
    println(f"{n:x}");        // 2a

An absent alignment resolves from the value's type, so a column of
figures lines up on the ones place without being asked. Width never
truncates: a silently shortened number in a log is worse than a
ragged table. `Decimal` precision goes through the exact
fixed-point formatter rather than an `f64`, so rendering a money
value does not reintroduce the rounding `Decimal` exists to avoid.
Ungrammatical specs are parse errors; specs that do not apply to
the value's type are type errors. Neither reaches codegen.

**Diagnostics inside an interpolation have spans.** They used to
report at `1:1`: the sub-parse ran on a private string whose
offsets began at zero, so the caret landed on whatever declaration
was at the top of the file, and the author was told their type
declaration was wrong. Token spans are now shifted into the
enclosing source before parsing, which fixes every consumer at
once. Making them real also exposed that some errors were reported
twice; byte-identical diagnostics are now deduplicated.

**A lint for the silent case.** A plain string containing `{x}`
passed to `print`/`println`, where `x` names something in scope,
warns and suggests the f-string. It fires only when the braces
name a real binding, so `println("{}")`, `println("{\"a\": 1}")`
and prose about a template stay quiet.

**`std::log` gains its structured half.** The module already was
the "logs as a topic" design — typed events on hierarchical
`log.<path>` subjects, sinks as ordinary bus subscribers. What it
lacked:

- **fields** — `LogEvent.fields` carries logfmt text, built with
  `std::log::kv(k, v)` (which quotes values containing a space, a
  quote or an `=`). Every level gains a `_kv` variant. Text rather
  than a map because a map in Hale is a *locus* and a locus cannot
  be a payload; the flat record keeps crossing every transport the
  bus supports and keeps the field order you wrote.
- **`ts`** — unix seconds stamped at the **publish** site. The
  console sink previously printed its own clock, which is a
  different time under a queued or bridged sink and an unrelated
  one under `hale replay`.
- **level filtering** — `HALE_LOG=error|warn|info|debug`, or
  `min_severity` in code. Filtering is at the *publisher*, so a
  suppressed `log.trace(...)` in a hot loop costs one integer
  compare and publishes nothing at all. Sinks accept the same knob,
  for when two sinks want different levels.

Locus attribution — stamping `locus=…` on every record from the
observability publisher TLS — is **not** in this change, and the
issue's "near-free" estimate for it does not survive contact: the
instance table holds `{self, id, type_id, parent}` with no name
(names resolve consumer-side from the model), the table only fills
when observability is on, and the publish happens inside
`Logger.info` so the attributed locus would be the *logger*, not
its caller. The cascading `parent_path` is the attribution that
works today.

### `match { cond -> … }` — first-match-wins without a scrutinee

First-match-wins over guards was always expressible: a match arm
carries a guard, so `match true { _ if cond -> … }` worked. You just
had to invent a scrutinee you then ignored and write `_ if` on every
arm.

That shape is a `cond`, and it appears wherever dispatch is a ladder
of tests rather than a shape match — HTTP routing, protocol
dispatch, tiered fallbacks:

    return match {
        std::http::is_route(ctx, "GET", "/users")     -> self.list(),
        std::http::is_route(ctx, "GET", "/users/:id") -> self.show(id),
        else                                          -> not_found(),
    };

`else` is the catch-all. Arms are tried in order and the first true
one wins.

Parser-only sugar: it desugars into exactly the ignored-scrutinee
form, so typecheck, codegen, the model and every judgment see the
match they already saw, and nothing downstream learns a new shape. A
test pins that the two spellings produce identical programs.

Notably it needs no method references — the arms are ordinary calls
with `self` in scope, which is why this and not a `routes` block was
the answer for one-locus-many-endpoints (GH #509 §2).

## v0.19.0 — check earlier, run faster (2026-09-02)

### `on_failure` and literal `subscribe` are checked, not just built

Two shapes in the language's core vocabulary passed `hale check` and
failed `hale build`, with no source location:

- `on_failure` with the wrong arity, or a second param that is not
  `ClosureViolation`. Codegen enforced both; the checker looked at
  neither.
- `subscribe "some.subject" as h;` with no `of type T`. A declared
  topic supplies the payload, but a literal subject has no
  declaration to take one from, so codegen needs the clause and said
  so far too late.

Both now report at check time, with a span, and the subscribe
diagnostic shows the spelling that works. Two test fixtures turned
out to be invalid Hale that only ever passed because those tests
check and dump an artifact without building.

This is the first pass against the check/build divergence baseline:
47 down to 42.

### Locus param defaults are typechecked

They were not. `check_locus_member` skipped the `params` block with a
comment claiming defaults "are checked against declared types
implicitly when the param is referenced" — they were not, so this
passed `hale check`:

    locus L { params { n: Int = "nope"; } }

and failed in codegen, which the checker was better placed to
report. Same for a default naming something that does not exist, and
for a locus literal omitting a required param.

Found writing an HTTP example: `handler: u` referencing a sibling
param checked clean and then failed to build with "unknown
identifier `u`" and no source location.

A bare name in a default resolves to top-level CONST scope, not to
sibling params — with a `const n` and a param `n`, a default written
`n` takes the const. So an unresolved bare name is a genuine unknown
identifier, and the diagnostic now says the spelling that reaches a
sibling:

    param `b`: unknown identifier `a` in its default — `a` is another
    param of this locus; a default reaches one through `self.a`

The coercions a default may rely on are preserved: Int literal into
a Float param, String into StringView, Bytes into BytesView, a locus
into a param typed as the perspective it serves, and a template
literal into a param typed as its generic monomorph (`b: Box<Int> =
Box { value: 0 }`). Each of those is a corpus program that a naive
check rejects; each has a test.

Seven harvested corpus fragments stop being check-clean. All are
single-seed slices of multi-seed tests, referencing types their
sibling seed declares, so they were never buildable standalone —
they were admitted only because defaults went unchecked. The tests
themselves are unaffected.

### HTTP route matching is ~4x cheaper

`__http_match_pattern_into` counted segments in both strings and then
walked them pairwise — and `__http_seg_at` re-scans from the start and
allocates a substring, twice per segment. On a route table nearly
every candidate is a miss, so the whole walk was paid to discover
that.

Every segment before a pattern's first `:` is literal, so a match
requires that text to prefix the path. Testing it first — before the
two segment counts, which scan both strings end to end — rejects a
miss in one comparison.

Measured over 200k requests, worst case (the request matches the last
route): 390 -> 91 ns per candidate route; 39.1 -> 9.8 us per request
at 100 routes; 2.1 -> 1.1 us at five. Both the `Router` and the
`is_route` ladder go through this matcher, so both benefit.

Behaviour is unchanged, and now pinned: trailing-slash tolerance on
both sides, a literal head that prefixes without matching segments,
count mismatches, captures, and the capture-clearing an `if`-ladder
depends on. The first version of the prefilter broke trailing-slash
tolerance on the pattern side and nothing in the suite noticed, which
is why those cases exist.

The remaining cost is that matching is still linear in the route
count — see GH #509.

### Element chains take fixed arrays and `bounded[T; N]`

`self.probes.filter(it.live).count()` over a `[Probe; 16]` failed with
`no field `get` on `[Probe; 16]``. Chains rewrite — post-parse, before
typecheck, so with no type to dispatch on — into a loop that fetches
each element through the source's `get`. A `@form(vec)` is a locus and
has that method; the two type-level collections are types, whose
operations are grammar intrinsics (`at(f, i)`), so neither could
anchor a chain at all. One downstream fleet counted ~11 would-be sites
in a single component and ~44 hand-rolled index walks across it
(downstream handoff).

Both now answer `get(Int) -> T fallible(IndexError)` — the accessor
the rewrite was already built around. `bounded` routes into its own
`at`, so a chain walks the LIVE slots rather than the capacity and
shares one bounds check with the intrinsic instead of a second copy
that could disagree with it. A fixed array's length is static and
every slot is live, so it gets its own check.

This is a deliberate exception to the types-have-no-methods axiom,
and the only one: `get` exists so the chain source protocol is
uniform across locus-form and type-level collections, and `at`
remains the idiomatic spelling for a direct index.

The diagnostic added earlier for this case — "`[T; N]` is not a
supported source form yet" — is gone along with the limitation;
it had become unreachable, and dead advice about a limitation that
no longer exists is worse than none.

### `restart(c) for N` lowers, and exhausting it quarantines

The retry-bound modifier checked and modelled — the topology
artifact has carried `retry_bound` on the supervision row since
schema 1.10 — but codegen refused it: "unsupported in codegen v0:
recovery modifier (for/until) not lowered". So the policy a consumer
was told it could read, "declared cap 3, observed 3 in 40s", was
unshippable, because no program stating the bound could be built
(downstream handoff). The workaround was to count failures in the
handler by hand, which states the same policy somewhere nothing can
read it.

`restart(c) for N` and `restart_in_place(c) for N` now lower. The
bound is per child instance and cumulative over its lifetime, and it
is compared before the restart, so `for N` admits exactly N restarts
and the next failure **quarantines** the child — a bounded
supervisor said when to stop, and stopping means the child does not
run. `for 0` is meaningful: do not restart this one. `N` is an
expression, not just a literal.

The bound had to become visible to the post-handler rerun check in
`__birth_closures`, which was a hardcoded `count <= 2`: a declared
`for 5` would otherwise have silently stopped restarting after two.
Every locus now carries a `__restart_bound` field seeded to that
default, so an unbounded `restart(c)` keeps exactly its previous
behaviour — including that it does NOT quarantine.

Two neighbouring modifiers stay unlowered and now say so precisely
rather than sharing one generic refusal: `quarantine(c) for d` (a
duration before an automatic restart — a different `for` from the
retry count) and `until` on any op. `spec/semantics.md` and the
failure chapter marked both, and the docs stopped advertising
`quarantine(child) for d` as if it worked.

## v0.18.0 — the canonical model (2026-08-30)

### A computed publish subject is confined to its declaration

A send whose subject is not a literal requires the locus to declare
a wildcard `publish` whose payload matches. That declaration was an
authorization nothing enforced — the type checker recorded it and
noted that "static subject-pattern verification is impossible by
definition", then let the computed string reach dispatch verbatim.

So a subject *outside* the declared pattern was delivered to
whatever subscribed to it, and the payload reinterpreted as that
subscriber's type. A two-field `LogEv { a, b }` published under a
`publish "io.tcp.**"` declaration onto `"app.order"` arrived at an
`Order` handler as `id=a qty=b`, field for field, deterministically,
with `hale check` reporting `ok`. Found while investigating a
downstream handoff.

Two checks now run at the publish site, on the computed path only —
a literal subject is bound to its declaration at compile time and
pays nothing:

- the subject must lie under one of the locus's declared patterns
  (`BusPublishUnauthorized`);
- it must not reach a subscription declared for a different payload
  (`BusPayloadMismatch`), which closes the complement the first
  check cannot: a subject *inside* the pattern whose subscriber
  disagrees about the type.

Statically, a subscription sitting under another locus's wildcard
pattern with a different payload is a warning rather than an error:
whether the hazard is live depends on whether that locus ever
publishes a subject reaching it, and the stdlib's TCP logging is
declared on every `Stream` but stays off until `log_subject` is set.

The runtime matcher and the model's must not drift — a publish the
model proved impossible must not be permitted at runtime — so
`wildcard_match` moved down to `hale-model` as the single Rust
definition (`hale-types` re-exports it), and a parity test runs it
and the C implementation over one shared case table.

### An unresolved publish is scoped by the pattern that bounds it

Because a computed publish can no longer escape its declaration,
the patterns bound which subjects an unresolved publish can address:
`AbsorbedEvent::PublishHole` now carries them.

`exact_publishes` is one bit for the whole program, so consulting it
for a subject-specific question meant a single `recv_bytes` call —
`std::io::tcp` publishes per-op log events to a runtime-chosen
subject, a genuine unresolved publish — withdrew the publisher
account for every topic and degraded every judgment family. It
blocked `@effects(depends:)` adoption on loci doing no I/O at all
(downstream handoff).

`depends:` now asks whether residue can reach *that* subject.
A declaration on an application topic certifies through unrelated
stdlib I/O; one on a subject genuinely under the stdlib's pattern
still refuses. Unbounded residue — an unfollowable interior call, a
truncated frontier, an interior publish whose subject expression
resolves to no subject row — still withdraws every subject.
`exact_publishes` itself is unchanged: the program-wide account
really is incomplete.

### `@budget` artifacts are admissible to a fleet plan again

`hale check` passed a `@budget` contract and `hale fleet check` then
refused the same artifact — "law ordinal N has no lowered evidence
row matching `bound alloc <= 0 ...`". GH #476 Change 5h routed
budget certificates through the evidence projection keyed
`(law ordinal, cert ordinal)` like every other family, but admission
kept also registering the pre-5h cert-less expectation. Nothing
emits that row, so the exact lowered↔law bijection was
unsatisfiable and every binary carrying a `@budget` contract was
inadmissible (downstream handoff).

Removing the stale expectation is strictly tighter — its `invalid`
branch also admitted an unclaimed cert-less `lowered` row. The
existing operand-mutation anti-control had been passing vacuously,
accepting the very message every budget artifact produced; it now
pins the exact error, and a fleet fixture finally carries a
`@budget` binary.

### The legacy claim evaluator is deleted (GH #476 Change 10)

`claims.rs` answered every claim family from a second walk over the
source, in parallel with the judgment engines answering the same
questions over the canonical model. Changes 5a–5h migrated the
families one at a time, each held byte-equal against that evaluator
over the whole corpus. With `causes:`, `depends:` and `@budget`
migrated, it answered nothing for anyone shipping.

It is gone — about 1900 lines, from 3177 down to 1285. What remains
is everything that runs BEFORE a verdict exists, and that name is
now accurate: clause enumeration across the world and library
tiers, constitution adoption and its normalized identity digests,
group resolution, and the vocabulary helpers the model builder
calls. Selection turns out not to need the bus graph at all — it
reads clause text, adoption and membership, never an effect walk.

**The corpus differentials became snapshots.** A comparison against
an independent implementation is the right instrument during a
cutover and impossible after it. Each snapshot was generated from
the final green differential run, so every line in it is literally
the evaluator's last word, and had to survive that comparison to be
committed:

- `claim_diags_snapshot.txt` — what `hale check` says, per corpus
  program (`HALE_REGEN_CLAIM_DIAGS=1`).
- `law_rows_snapshot.txt` — the artifact's claim rows: name, form,
  verdict, source (`HALE_REGEN_LAW_ROWS=1`).

A diff in either is a user-visible change. The lowered-certificate
comparison stays a live differential: the certificate ENGINES were
never what was being migrated, so they can still disagree.

Two invariants moved rather than died: the lowering's parity
obligation is now stated against law selection itself
(`claims::selected_clauses`) instead of the evaluator's outcome
list, which is what it always stood for; and the shared-reachability
guard now watches `judgment.rs`, since that is where the
prohibition walk lives.

### `@budget` is judged over the canonical model (GH #476 Change 5h)

The last law answered by an engine of its own. `@budget` now
certifies through the **evidence sidecar**, the same path the
`@effects` certificates take: the counting engines still measure —
that is an analysis, not a law — and hand over their certificate
and their own diagnostics. The VERDICT is the judgment's.

That removes the duplicate authority without duplicating the
analysis. `hale check` and the artifact previously read the
engines' answer directly, each in its own way; both now read one
judgment over one model.

Supporting facts, added in the same change:

- `relations.costs` — per-call cost sites (`alloc`, `block`,
  `frame_bytes`), SITE-grained, because a per-call budget is a
  statement about one invocation and the loop flag is what turns a
  finite count into an unbounded one.
- `RelationSet::COSTS` and the `exact_costs` capability. The three
  call-hole kinds (indirect call, untyped receiver, open interface)
  now REQUIRE the COSTS bit: a call whose target the caller chooses
  is exactly where a quantitative law must not certify through.
- **Fan-out counts subscriber DELIVERIES, not subscription
  declarations.** It is a publish-SITE query now, answered against
  the model's delivery join and the arrangement's instance
  population: three arranged replicas of one `Sink` are three
  deliveries where a declaration count said one, and two
  mutually-exclusive key filters are no longer both charged to a
  publish whose key can reach only one. A dynamic population, an
  unknown key, an external route, or a computed subject is
  unboundedness — never one. One consequence worth naming: a
  subscriber born inside a function body rather than in the main
  arrangement is not an instance the model enumerates, so a publish
  reaching it now measures *unbounded* fan-out where the old count
  reported a number. That is the honest answer — the population is
  genuinely not known — and the verdict is unchanged in every
  corpus program; only the diagnostic's wording moves.
  publish whose key can reach only one. It is also TRANSITIVE:
  `A → Relay::on_a → B → three Sinks` is four deliveries caused by
  one invocation, and the ordinary call graph never enters a
  handler through the bus. Population completeness is scoped to the
  loci on that delivery closure, so an unrelated dynamically-born
  locus no longer makes every fan-out in the program unbounded. A
  dynamic population of a REACHED subscriber, an unknown key, an
  external route, or a computed subject is unboundedness — never
  one. It is a WEIGHTED execution traversal: three `Relay`
  instances each republishing to one `Sink` is six deliveries, not
  four, because each work item carries how many handler invocations
  reached it, and the handler's own CALL TREE contributes execution
  counts: two calls to one publishing helper are two publishes,
  alternatives of one interface dispatch take the max, and
  recursion or a loop saturates. It also counts the recipients of
  ONE MESSAGE rather than the union of possible recipients — a
  message carries one key, so disjoint literal filters cannot both
  receive it and `where key == replica` selects the single instance
  whose index equals that key. A declared but never instantiated
  subscriber receives exactly zero, not "unknown". The choice of
  key and the choice of interface conformer are carried through the
  WHOLE downstream calculation before any maximum is taken —
  `max over keys of (immediate + downstream)`, and
  `max over alternatives of (sum over that alternative)`, rather
  than a maximum followed by the union of every branch. Through-
  stdlib multiplicity comes from the per-entry absorption account
  rather than the contracted endpoint relation, which collapses
  two entry sites into one — and the interior is walked as the
  GRAPH it is, with same-group interior alternatives taking the
  max. One authored call site is one choice whether its
  alternatives are user conformers, stdlib entries, or both. Key
  domains constrain which scenarios exist (an `IntRange` publish is
  never charged a `fallback` that its interval cannot trigger), an
  ordinary instance registers under the effective replica key 0,
  and a zero-population endpoint contributes zero however
  complicated the declaration's body.

Zero annihilates all the way down: a repeated publish or call that
delivers nothing delivers nothing (`loop × 0 = 0`), and an unknown
key filter on a locus with no instances routes nothing. Key
scenarios are built from the distinct ACTIVE routing partition
rather than from declarations — a `Bool` domain is exhausted by its
`false`/`true` filters so the `_` fallback can never fire, while
two declarations naming one value do not cover a two-value
interval.

An unknown subscriber population is not an absent one: a keyed
subscriber whose locus can also be born outside the arrangement
withdraws the bound rather than dropping out of the routing
partition, `where key == replica` will not count listed rows as
exact over an incomplete population, and a subject whose subscriber
count is unknown has no fan-out bound at all. Residue on an
unrelated locus or subject still says nothing.

**`ANALYSIS_SEMANTICS_VERSION` 3 → 6.** These results moved, and
`EvidenceTable::validate` treats an equal `inputs_digest` as proof
of current semantics rather than hashing the implementation — so a
sidecar produced by an older toolchain could otherwise share every
digest while carrying a fan-out verdict this one disagrees with.
- **Quantitative budgets cross seed boundaries.** The migrated
  evidence path called the engine without the import-rename table,
  so `lib::expensive()` stayed an unresolved qualified free call
  and contributed zero — `@budget(publish = 0)` could certify over
  an imported publisher. Every dimension was affected.

**Artifact schema 1.16 → 1.17.** `@budget` rows carry
`"family": "budget"` with their own `certs` evidence and an
`adequacy.budget` entry. `law.legacy`
is now empty for every program — no family is `unmigrated` any
more — and the section survives only so artifacts written by older
toolchains still decode.

One behavior note: a `@budget` diagnostic is now `Claim`-kind and
is reported only for programs that typecheck, like every other law.
A fixture that never typechecked no longer gets a budget verdict.

### `depends:` is judged over the canonical model (GH #476 Change 5g)

`@effects(depends: {…})` on a locus (RFC #330) is the backward dual
of `causes:` — the COMPLETE set of subjects that can transitively
reach any handler the locus owns. It is now judged over the
canonical model, by the same shared queries the forward walk uses.
That is the point: if the two walks disagreed about whether a
publish and a subscription meet, one of them was wrong.

Three corrections fall out of the shared join:

- **A declaration may name the wire subject.**
  `@effects(depends: { "evt" })` and `{ Evt }` address one
  endpoint. The old engine compared names and reported the wire
  spelling as an omission.
- **An operand naming nothing is `invalid`, not `violated`.** The
  old engine matched declared entries by name, so a typo covered
  nothing and every subject that reached the locus came back as an
  omission — a violation report about the subjects, when the defect
  is the typo. The diagnostic now names the unresolved entry.
- **An inbound route is uncertainty, not silence.** A `listen`
  binding on a reached subject means a peer this application does
  not model can publish into it, so the closure cannot be certified.
  An outbound `connect` route on the same locus does not taint it —
  the completeness query takes a direction.

The `sync`-discipline refusal (#340) is unchanged: a locus holding
a `@form(…, sync = …)` param is reachable by another pool's writes
with no bus edge recording it, and `depends:` closes over the
message graph only.

Diagnostics for the family are now `Claim`-kind, matching the other
law families.

**Artifact schema 1.15 → 1.16.** `depends:` rows carry
`"family": "depends"` with their own rendered `form`, an
`adequacy.depends` entry, and no `law.legacy` entry; that report now covers `@budget` alone.

### `causes:` is judged over the canonical model (GH #476 Change 5f)

`@effects(causes: …)` was the last effect law still answered by a
second engine. It is now judged by `judge_causes` over the
canonical model, on both the check path and in the artifact, and
the old walk in `frontier` survives only as a test oracle.

Three user-visible consequences, all corrections:

- **An unclassified walk is `uncertified`, not a violation.** The
  old engine saturated: one call it could not name turned into
  every class at once, and the diagnostic listed classes it had
  never proven. Not knowing is now reported as not knowing.
- **A known effect is never erased by uncertainty.** The lower
  bound is carried separately from the unknown flag, so a law
  that a proven effect already violates reads as `violated` even
  when the rest of the walk is opaque.
- **Delivery joins on wire identity.** A publish reaches a
  handler when their subjects meet, not when they were spelled
  with the same topic name — a literal `"t" <- …` send is judged
  exactly like the declared spelling it lowers to, and a binding
  that leaves the application makes the walk `uncertified`
  instead of silently ending.

Diagnostics for the family are now `Claim`-kind, matching the
other law families.

**Artifact schema 1.14 → 1.15.** `causes:` rows carry
`"family": "causes"` with their own rendered `form`, an
`adequacy.causes` entry stating whether the model is exact or
degraded for the family, and no longer appear in `law.legacy`; admission re-renders the form from
the typed payload rather than trusting an imported verdict.
### Every typed law row states its rendered form (schema 1.13 → 1.14)

A row whose law renders a compatibility form now carries it, and
admission REQUIRES it: the form re-renders from the typed payload,
so substituting an operand orphans it. Previously the field was
optional and the check was guarded on its presence — deleting it
skipped the operand binding entirely.

Requiring a field is a decoding-contract change, not an additive
one: a schema-1.13 artifact carries rows without the key and would
be refused by a reader built from this tree. So the schema number
moves with it.


### `LegacyProjection` is gone (GH #476 follow-up)

The model carried a compartment named for compatibility that held
two different things: data no other table held, and copies kept
only so serialized bytes would not move. Split and settled.

Deleted, because they were duplicate authorities inside the model:

- **`topology_v1_fns`** — the artifact's fn universe, stored beside
  `Function.summarized`, which is the same set. Its only model law
  asserted the two agreed. Derived now via
  `ApplicationModel::summarized_fns`, so they cannot.
- **`topology_v1_calls_via_stdlib`** — the pre-model contraction
  walk's verbatim output (one Boolean, no revisit), carried so the
  hashed `calls_via_stdlib` loop bit could not move away from what
  that walk produced. The artifact now projects the model's own
  `ViaStdlib` call rows, and the walk and its shared helper are
  deleted.

  **This is a versioned shape transition: schema 1.12 -> 1.13.** The
  two interpretations agree on endpoints and can disagree on the
  hashed `loop` bit: the old walk kept a set-valued `seen` per
  caller, so a stdlib body first reached on a non-looped path was
  never revisited when a looped path reached it later, while the
  model's relation revisits on strengthening. The model's answer is
  the sound one for what the bit asks — does this carrier repeat per
  iteration — and a program in the distinguishing class gets a new
  `shape_hash` and must be re-recorded once. No such program exists
  today: the stdlib re-emerges into user code only from inside its
  own loops, which sets the bit either way, so not one committed
  baseline hash moved. The schema still bumps, because the
  interpretation changed and identity discipline is about what the
  bytes MEAN, not about whether a corpus happened to notice.

Kept and renamed, because they are facts about the program that no
other table holds: `Analyses` carries the bus graph's per-subject
`dispatch_gates` and the `stdlib_absorption` interiors a
reachability walk needs. Neither is a bridge to anything.

`ApplicationModel.legacy` is now `ApplicationModel.analyses`.

### One authority per question (GH #476 Change 9)

The canonical-model epic's last change: the duplicate authorities
are gone.

**Claim verdicts.** `hale check` reported reachability, boundary,
endpoint and bound verdicts from an evaluator that re-derived them
from source, while the artifact reported the same four families from
the judgment engines over the canonical model. Check now calls the
judgment engines, so the document and the compiler that produced it
cannot disagree about a law. The judgment merge that had lived
inside the artifact projection is extracted as `judge_all` — one
entry point, two consumers. What stays with the claim surface is law
SELECTION (which laws exist at all: constitutions, adoption, group
resolution, the tier rule), which is not judgment and has one
implementation.

Two user-visible consequences, both fail-closed corrections:

- `require attributed` over a body with an indirect or opaque call
  is `uncertified` at check time, where the old evaluator fail-OPEN
  held. A build that passed on that fail-open now fails. The
  artifact has judged it this way since Change 6 (`semantics` 2).
- `only edges A -> B { publish Metrix; }` naming an undeclared topic
  is refused as an invalid law with a did-you-mean hint. It used to
  drop the grant silently, which evaluated a WEAKER claim than the
  one written and reported its violations as if the author had
  chosen them.

Review round 1 (PR #492), three blockers:

- **Selection is one result, consumed twice.** The lowering ran only
  clause enumeration, so it never saw group resolution: an unknown
  group member failed `hale check` while the artifact recorded no
  issue and could serialize the dependent law as `holds` — the
  checker and the document giving opposite machine-readable answers
  about one program, which is worse than the two implementations
  this change set out to delete. Both now consume
  `claims::select`. A law over a group selection refused is
  `invalid` rather than vacuously true; `may_be_empty` groups keep
  holding vacuously, which is what declaring it means.

  Round 2: selection's verdict on each group is CARRIED with the
  lowered laws (resolved / intentionally empty / selector failed /
  declaration refused) instead of being re-derived from the model's
  member count. Those are different questions, and the difference
  had three exploitable shapes: an unresolved selector leaves no
  member behind, so `{ Missing } may_be_empty` read as intentional
  vacuity, `{ Worker, Missing }` read as whole and was judged over
  the surviving subset, and a name declared twice read as fine while
  the model keeps the LAST declaration and selection keeps the
  first.

  Round 3: the guard covers every group OPERAND, not the endpoints
  only. `avoiding` is a domain too — its members become the mask
  that removes paths from the walk, so a partially-resolved gate
  masked with whatever survived and the claim was proved against a
  subset of the gate the author wrote. A table-driven control now
  pins all eight group-operand positions across the five families,
  and asserts its own coverage count so a new operand fails it.
- **Judgment requires a program that denotes a model.** Check called
  the judgment whenever a claim surface existed, including on
  ill-typed programs whose models are deliberately unlawful (a key
  filter on an unkeyed topic) — a debug-build panic at the builder's
  own assertion, and in release a walk over relations whose indexing
  assumes lawfulness. The model half now runs only when the resolver
  and type checker are clean; claim errors do not gate it. This
  surfaced a stale stdlib API in a test fixture that had been
  ill-typed and judged anyway.
- **Claim spans survive a bundle with no source map.** Claim lowering
  collapsed unplaceable spans to synthetic records, which every
  consumer renders at 0..0 — so through the public `check_program`
  and the LSP, every migrated claim diagnostic anchored at byte zero
  of the first file. Lowering now keeps the offsets, exactly as the
  model builder already did, and the LSP installs the source map it
  has always had.

**The artifact's second serialization.** Change 6 made production
emit the projection of `ApplicationModel` but kept the legacy
gathering as the corpus differential's comparison arm — relation
rows, labels, unknowns, groups, supervision, the whole unhashed
provenance tail, re-derived from source and thrown away. Deleted
(-752 lines from `topology.rs`), along with the collectors that fed
only it. `dump_topology_parts` returns one String.

Artifact identity is now pinned by a committed baseline of
`origin -> shape_hash` over 1398 corpus programs instead of by a
rival implementation. A model change that moves an artifact hash —
re-keying replay admission for every existing recording of that
program — fails the gate with the moved rows named and a regenerate
hint, so the move is a decision somebody made rather than one
noticed later. The emitter also still owes self-consistency: the
artifact it writes must hash to what the projection says, and its
emitted model half must BE the projection.

**Demand.** A claim-free program still never derives the model in
check — the LSP's cost contract. A program WITH claims now does,
because judging it means reading the model.

`LoweringIssue` gains a `family`, so the artifact (which carries
every issue) and check (which reports only issues no other engine
owns) filter coherently instead of guessing from message text.
`constitution_identities` stops the adoption walk at selection: both
consumers wanted only the identities and discarded the evaluation
that came with them.

The old evaluator survives as the comparison arm of three corpus
differentials — an independent implementation disagreeing is
evidence a baseline cannot give — with no production callers, a
boundary enforced by `legacy_oracle_is_test_only.rs` rather than
documented and hoped for.

### Deployment consumers: the arrangement + the dispatch plan (GH #476 Change 8)

Review round 1 (PR #491), five blockers plus two identity bugs they
uncovered:

- **Replica rows carry their index, not the count.** `replicas = 3`
  produced `[3, 3, 3]` where the runtime has replicas 0, 1, 2 — the
  keyed-delivery model the arrangement exists to feed would have
  named a population no process has. `validate` now enforces that a
  replicated field's instances form a contiguous 0-based set and
  that the index in the path and the index in the row agree.
- **Binding rows are decoded, not defaulted.** Every unix binding
  was modeled `role: Listen, loss: WaitCapable` regardless of what
  was authored, so an explicit (or inferred) `connect` binding was
  its own opposite in the model. Roles now come from ONE rule shared
  with the desugar, loss follows the role, and rows are canonically
  sorted BEFORE ids are assigned — two bindings authored in reverse
  subject order used to produce a model that failed its own
  canonical-order law.
- **A partly dynamic population is not same-domain.** The plan built
  its domain map from arranged instances only, so a locus with one
  arranged instance and one dynamic birth answered "main" for the
  whole population — a false stage-0 optimization opportunity, and
  an unsafe precondition for the placement-driven lowering it is
  meant to inform. A placement hole now deletes the locus's answer.
- **`hale build` artifacts carry an execution identity.** The
  identity plumbing reached `hale run` and `hale replay` but not the
  path that produces the binaries users ship, which set a model hash
  and no digest at all. It now stamps the digest from the FINAL
  build options plus the plan.
- **Canonical ids are identity-bound and total.** `model_hash` does
  not cover every table these ids index (binding rows are not in the
  artifact), so the header publishes `entity_id_digest` at `0x88`
  (proto 0.3) over the exact stamped rows: a consumer recomputes it
  from its own model and joins only on a match. The mapping table
  grows with the program instead of silently dropping entities past
  a fixed 512 — a dropped row read back as "unstamped", which is
  what a build with no ids at all looks like. Out of memory now
  withholds the whole channel, loudly, rather than half of it.

**Fix (observation identity).** A program with a `bindings { }`
block published `model_hash 0` for its whole life: registering a
binding creates the observation segment, segment creation snapshots
the identity fields, and the prelude stamped them afterwards. The
identity setters now run before anything that can register. Eager
recording/replay init deliberately stays after binding realization,
so a backend with no replay class still refuses at its own seam.

Review round 2: the replica-index law read `path.rfind('[')`, so any
descendant of a replica (`App.workers[0].leaf` — the arrangement
walks into each replica, and a leaf beneath one is an ordinary
child) was treated as a malformed replica row and refused the whole
model. Replica-ness is a property of the last path component only.

**Fix (binding role inference).** The documented inference
(publish-only → `connect`, subscribe-only → `listen`) was dead code:
the desugar ran it after rewriting topic references to literal
subjects, so it saw no topic ends and filled in nothing, and codegen
refused every binding that did not spell `role:` explicitly.

The model's arrangement tables are populated and their consumers
land on them. `locus_instances` / `realizes` / `owns` /
`placed_in` / `thread_domains` / `bindings` / `binds` now carry
the static deployment: the params-default tree rooted at the main
locus, `pinned(replicas = K)` fanned to K instances, thread domains
by the runtime's own rule (pinned owns a domain, `cooperative(pool
= X)` runs on X's worker, everything else inherits its owner, the
root is `main`, a binding reader is its own domain). What the
arrangement cannot see is a typed hole, not silence: instances born
outside it (method bodies AND free functions - `fn main() { W { };
}` is the corpus's most common shape) hide OWNS|PLACED at the born
locus, adapter transports hide BINDS|DELIVERY at the topic, and the
capability account gains `exact_ownership` / `exact_placement` /
`exact_routes` accordingly. This closed a fail-open: a program whose
loci are all born in `fn main` used to claim exact placement over
zero modeled instances.

`DispatchPlan::derive(&ApplicationModel)` owns the lowering
decision: one row per subject with the flavor (dynamic /
static_bucket / static_direct), the gate's reason, the subscriber
list, publisher and subscriber THREAD DOMAINS, and `same_domain` -
GH #464's stage-0 survey question as a model query rather than a
bespoke topology walk. `hale model dump` prints the plan and the
same-domain count. Codegen no longer decides: it still computes its
own gate facts over the merged, desugared program it emits from,
but the ladder is `DispatchFlavor::of` in hale-model and its ids /
direct set / direct-subscriber lists come from the plan. A corpus
differential runs both fact sources through the real binaries and
requires agreement on every subject the model names.

The plan is part of build identity: `exec_digest` frames
`DispatchPlan::digest()`, so two builds of identical sources that
lower dispatch differently - notably the `LOTUS_NO_BUS_DEVIRT=1`
control arm - no longer share a recording identity, and replay
across that boundary is refused by name.

Iris joins by ID instead of by name: manifest rows carry the
canonical model entity id in `aux_b` (a field in the entry layout
since v0, written as 0 by every path until now). MK_TOPIC rows
carry the `SubjectId` (the manifest fuses publishers by wire
subject), MK_LOCUS_TYPE the `LocusDeclId`, MK_BINDING the
`BindingId`, all as `index + 1` so **0 still means unstamped** -
harness builds, and entities the model does not name, read exactly
as before. The ids are indices into the model whose `shape_hash`
the header already carries, so the two travel together.

**Fix (bus dispatch, transport-bound subjects).** A subject named
by a `bindings { }` entry is exempt from devirtualization - the
adapter's peer is the real counterparty - but the exemption was
keyed only by the topic DECL name, while codegen builds its graph
after topic desugaring, where subjects are wire strings. A bound
topic could therefore be lowered into a static bucket its own
adapter is not part of. The gate now records both grains. Found by
the Change-8 plan differential: 1 of 21 bus programs in the corpus
disagreed.

### Typed FleetModel + the versioned shape transition (GH #476 Change 7)

Round 6: completeness with the polarity of the law. The blanket
holds->uncertified rule now applies only to the absence-certifying
families (forbid_reaches/only_edges); require_* witnesses are
positive facts no incomplete endpoint set can erase, and their
route-backed failures are definite (the plan's route table is
complete). Counts evaluate over a [known, known+hidden] interval
with per-verb relevant flags (publish+cardinality /
subscribe+cardinality): min met by known rows holds, max/eq
exceeded by known rows violates, the undecided interval is
uncertified - including the conjunctive eq/min/max form. The
claim row is serialized after the final verdict (it previously
recorded the pre-rewrite result). Track A is capped at exactly
the current schema; the canonical layout table is versioned by
it.

Round 5: layout is law, and the completeness account is
honored. The top-level key sequence is canonical and closed -
order defines the verified hash ranges, so moving a model-half
section outside the shape_hash interval, introducing an unknown
top-level key, or demoting artifact_digest from final position
all refuse (pinned). The typed component decode carries
capabilities + adequacy, and fleet claims fail closed over
admitted degradation: a family a component admits as `degraded`
cannot certify holds through it - `uncertified`, naming the
instances and withdrawn flags - scoped like the
unreachable-unknown rule so unrelated degradation does not
poison unrelated claims (pinned: exact_calls withdrawn, no
legacy unknowns, reachability prohibition does not hold).

Round 4: the raw admission pass is JSON-semantic and
path-aware. One structural walk (scan_top_level) drives
everything: duplicate keys compare DECODED names (an escaped
spelling cannot smuggle a second shape_hash past the scanner into
serde's last-wins map), and verify_shape_hash /
verify_artifact_digest locate their fields at the TOP LEVEL - a
nested decoy is data, never the verified value (pinned both ways:
the decoy neither confuses the verifier on an honest artifact nor
rescues a drifted one).

Round 3: the typed decode is strict, and keys are unambiguous.
ComponentModel::decode refuses malformed semantic rows instead of
filtering them - a number-typed call endpoint or unknown reason
is an error, never a silently dropped edge or erased residue
(both pinned against components that genuinely carry the shapes,
with asserted premises). Duplicate object keys are rejected
before parsing at both consumption boundaries: serde's last-wins
map parse would otherwise let a second shape_hash shadow the
raw-verified one (pinned).

Round 2: route identity is one grain, and the declared identity
is verified. Route role checks and edge construction both read
the typed topic-identity endpoint rows (a literal end on the same
wire satisfies neither; wire-grain accessors remain for the fleet
claims that quantify over wire subjects). verify_shape_hash
recomputes the model-half hash from the raw artifact at both
consumption boundaries before any decoding - a coordinated
hashed+unhashed endpoint edit under a stale shape_hash refuses.

Fleet composition is TYPED end to end: component artifacts are
decoded ONCE into a typed ComponentModel (fns, unioned call
relations, V1 publish/subscribe rows, the topics join surface,
unknowns, decl provenance, and the hashed endpoint identity)
immediately after the shared admission - the rest of composition
never walks generic JSON for a modeled fact (the #476
architecture canary). Route endpoint checks now read the
byte-exact hashed wire identity: "declares but never sends" is
judged from typed site rows, collision-proof.

Schema 1.12: canonical ENDPOINT IDENTITY joins the hashed model
half (`endpoint_identity`: verb, owner, source-order site
ordinal, byte-exact wire subject, declared topic). The V1
relations render a topic-covered end and a display-colliding
literal as one spelling, so the shape could not distinguish two
systems that talk to different wire addresses - which is exactly
what the shape exists to distinguish, and what replay admission
relies on. The unhashed endpoint sections must agree with the
hashed identity exactly, closing the round-12 substitution
residual. SHAPE HASHES CHANGE for every bus-carrying program:
re-record shape baselines and `.halerec` recordings once.


### Typed law artifact rows + adequacy (GH #476 Change 6)

The topology artifact's law rows are now PROJECTED from the
canonical model path: `claims` rows render from `ClaimIr`
(`ClaimRow::claims_form`, one authority) with verdicts from the
Change-5 judgments, and the effects-family `lowered` rows come
from the evidence sidecar (`@budget` rows keep their old producers
until the quantitative engines migrate). A corpus differential
holds the projection equal to the evaluator rows.

Schema 1.11 adds three unhashed, digest-covered typed sections -
`law` (every lowered ClaimIr row: ordinal, name, origin, judgment
family, machine verdict, provenance; plus `law_digest` and
`inputs_digest`), `capabilities` (typed positive completeness),
and `adequacy` (per migrated family: `exact` | `degraded`).
`shape_hash` is UNCHANGED - replay admission and recorded
baselines survive.

The emitted model half and `shape_hash` now come from the
`ApplicationModel` projection (`project_model_half`) - one
semantic authority; the legacy gathering survives only as the
corpus differential's comparison arm. Law rows carry a typed
tagged payload per `ClaimIr` variant (operands with raw/display
reference duality) and per-certificate evidence; unmigrated
families (`causes:`, `depends:`, `@budget`) carry the old
engines' authoritative results, so `clean` implies every law row
holds. Track A's claim view highlights from the typed payload
instead of substring-matching form strings. `exact_cardinality`
is now derived (closed-world endpoint counts).

Round 2: the unhashed `sources` / `provenance` / `topics`
sections project from the model too (`project_unhashed_tail`,
byte-green over the corpus); unplaceable spans intern as
`ForeignSpan` (offsets preserved); `SourceUnit.digest` keeps the
producer's exact string; bus selectors serialize candidate sets +
location; adequacy derives from the relation-level hole mask
(publish/subscribe independent); the unmigrated bridge refuses
rows the old engines never enumerated; Track A admission requires
and validates the 1.11 sections and the claim-to-law join.

Round 3: `PayloadContract` gains a structural `opaque`
discriminant (a field literally named `opaque` keeps its shape);
one shared V1 endpoint renderer joins the relation and provenance
sections on identical spellings; `exact_bus_endpoints` splits
into independent `exact_publishes` / `exact_subscribes`, and
adequacy reads the positive capability account (unvouched =>
degraded, holes are the cross-check); legacy `claims` rows carry
their law `ordinal`, and Track A admission enforces the closed
law vocabulary, per-kind payload shapes, reference existence,
contiguous ordinals, and the one-to-one claims-to-law join.

Round 4: the law payload is LOSSLESS (during/seed/user-class
dims are typed references with resolution status; every operand
carries its provenance; the four fleet variants have distinct
tagged payloads); law rows carry EVIDENCE (the judgments' ordered
diagnostics with source locations; certificate rows keep their
per-cert diagnostics); Track A decodes the payload against a real
closed vocabulary (exact enums for kinds/verbs/comparators/
via-edges/dimensions, complete variant shapes, reference
existence, builtin-class agreement), re-renders the claims-tier
form and requires byte agreement, binds source<->origin and
kind<->family, joins claims<->law in both directions, requires
the exact capability flag set, recomputes adequacy from the
positive account, and recomputes the document verdict.

`semantics` bumps to 2: machine verdicts are stricter in two
documented places (a certificate naming a cyclically-defined or
undeclared effect class is `invalid`, never a vacuous `holds`;
`require attributed` over an unanalyzable body is `uncertified`,
never a fail-open `holds`), and the document `verdict` follows
the machine. Artifact consumers reject unrecognized semantics by
design; re-dump with the current compiler.

Round 16: ownership is anchored to the entity identity. `owner
= Some(l)` requires the function's raw name and display to encode
l as their prefix, with the id range-checked directly - a fully
coordinated repoint (owner + member_of row + both analyzability
flags updated consistently) satisfies every relational law and is
refused precisely because Hidden::poke cannot canonically be
owned by App (negative controls at the model, the sidecar, and
the artifact). law.fn_universe rows carry the typed owner
(present iff non-free, display-anchored, cataloged), and
admission's locus coverage recomputes from it instead of
recovering membership from display prefixes.

Round 15: member_of is a closed ownership partition. Every
function carries its canonical `owner` (typed; None for free
fns), and member_of must be a total exclusive partition agreeing
with it exactly - a membership row can be neither deleted nor
moved to launder locus coverage or group projection. Ownership
is coverage-bearing (folded into the coverage digest), and ALL
ownership + coverage laws live in one shared validator
(ApplicationModel::validate_coverage) called by both
ApplicationModel::validate and EvidenceTable::validate - a model
whose ownership account is corrupted cannot certify anything,
digests notwithstanding. Negative controls delete AND move
Hidden::poke's membership; both fail model validation and both
fail to manufacture Holds through the sidecar.

Round 14: the coverage laws close at the MODEL. Application-
Model::validate now enforces analyzed => summarized (with the
existing converse: analyzed <=> summarized, the hashed anchor)
and derives LocusDecl::analyzable from the typed member_of
relation and FunctionKind (analyzable iff every non-failure
member is analyzed; empty set vacuously) - the same coverage
state is lawful or refused identically at the model, the
sidecar, and the artifact. Sidecar eligibility hardens to match:
fn subjects need analyzed AND summarized, and locus subjects'
member coverage is recomputed from member_of rather than
trusting the flag - both false->true upgrade paths (fn bit with
residue removed; locus flag over an unanalyzed member) fail
model validation AND cannot manufacture Holds through the
sidecar (four new negative tests).

Round 13: declared-end identity includes the typed topic, and
one closure rule. declares_publish's canonical key is (locus,
subject, declared_topic) - a literal declaration colliding with a
topic's wire subject and the typed topic declaration are distinct
facts that BOTH survive in either declaration order (the
first-writer collapse is gone; `require publishes(some G, topic
Orders)` no longer depends on source order). Closures never
count toward locus analyzability on either side: they are
invisible to the certificate machinery at every scope, so a
module-scoped closure-only locus is vacuously analyzable and its
honest artifact admits.

Round 12: span-anchored endpoints + the closed coverage
account. Site endpoint rows carry their authored span, exactly
one occupant per (verb, owner, site), and their per-owner span
multisets must equal the span-grained provenance section
one-to-one; declaration rows keep the owning locus in the
compared identity. Coverage closes: an unanalyzed body must
retain its UnanalyzedBody residue and an analyzed body carries
none (ApplicationModel::validate); a walked body is summarized,
anchoring `analyzed` to the hashed sorts.fns - the
coverage-upgrade flip (analyzed=true on a module body plus a
manufactured Holds certificate) refuses on the hashed anchor;
EvidenceTable::validate categorically refuses certificate
payloads for unanalyzed subjects/phases, so judge_certificates
can never replay them; and failure handlers share one typed rule
on both sides, so a module-scoped failure-only locus is
vacuously analyzable and its honest artifact admits.

Round 11: lossless site endpoints + the three-state coverage
account. Site endpoint rows carry their owning fn/handler and
authored site ordinal, project onto the V1 relations at (owner,
name) grain, and their per-owner counts must match the
span-grained provenance section - a literal end colliding with a
topic display can no longer disappear behind the dedup'd legacy
projection (the full narrowing attack refuses at every depth).
Coverage distinguishes its three typed states: `analyzed` (body
walked), `summarized` (summary row exists - this set IS
sorts.fns, and that equality replaces the wrong-universe
analyzed tie), and the engine-report account; failure handlers
carry the typed FunctionKind::FailureHandler, so a module method
named on_failure_helper is a real unanalyzed member and its
honest artifact admits. The coverage laws are validated in
ApplicationModel::validate, re-checked at admission, and folded
into the evidence coverage digest.

Round 10: typed endpoint identity + function-grain coverage.
Endpoint rows (and declares_publish rows) carry `declared_topic`
- the model's syntactic fact - so a literal wire address whose
text collides with a topic display stays a literal; admission
compares typed identities and projects site rows onto the V1
relations under their own declaredness, never inferring
topic-ness from strings (the colliding compiler artifact admits).
`Function::analyzed` is the model's coverage fact (false for
module bodies and on_failure handlers); fn_universe rows carry it
and the analyzed subset must equal sorts.fns exactly - the
on_failure-only locus admits, and locus `analyzable` recomputes
from member coverage with memberless loci VACUOUSLY analyzable
(no body = every phase contract holds by absence), closing the
memberless flip in both directions. Implicit-phase certificates
must be exactly the synthetic holds. Evidence identity gains a
coverage digest: a sidecar derived beside different coverage is
refused as stale (TopologyShapeV1 unchanged for recording
compatibility).

Round 9: no claim error disappears, and no section is a second
authority. A typed `law.issues` account serializes every
table-level law-selection failure (duplicate claim names,
unknown/cyclic constitutions, illegal adoption, collisions) with
locations; it participates in law_digest ({issues, rows} canon)
and the document verdict, and the duplicate-name case is
recomputed from the rows - two individually-holding duplicates
can no longer produce a clean artifact. The endpoints section
must project exactly from the artifact's own
publishes/subscribes relations plus the new typed
`declares_publish` relation section - deleting a literal
publish's endpoint row while the site relation remains refuses.
`LocusDecl::analyzable` is the model's fact (one walk, in the
model builder; evidence and emitter read the model), and
admission recomputes the flag against the hashed function
universe - flipping a module-scoped contract to analyzable
contradicts its member account. ANALYSIS_SEMANTICS_VERSION bumps
to 3: round 8's synthetic certificates and report-less
`uncertified` are result-affecting producer changes.

Round 8: the admission accepts every artifact the compiler
emits. Beyond references and classes, the claims evaluator's
other legitimate Invalid outcomes (operand-domain errors like
`require attributed` over a user class, vacuity, empty `during`,
`avoiding` overlap) admit by RETAINING the judgment's explanation
- an invalid row carries either a decodable invalidity or its
evidence. An implicit lifecycle phase with no hook body gets a
synthetic Holds certificate (no hook performs no effects), and a
report-less subject - a module-scoped body - judges `uncertified`
with its residue, never Invalid; `law.loci` rows carry an
`analyzable` flag so admission holds both shapes to their exact
verdicts. A typed `endpoints` section projects every bus endpoint
at wire-subject grain (including a declared publisher with no
send site), and `law.subjects` must equal exactly the subjects
the endpoint and topic sections carry, both directions - the
subject universe is validated against the model's own typed
projection, never reverse-engineered from the narrower V1
relations.

Round 7: the admission is SELF-CONTAINED and invalidity-first.
Static invalidity dominates: an unresolved operand or an
undeclared/cyclic effect class admits only `invalid` - the old
engine's replayed vacuous `holds` is never an alternative (the
exact fail-open the semantics-2 bump exists to prevent); a
certificate verdict is otherwise EXACTLY its recomputed evidence
severity, and a subject outside the legacy analyzable universe
carries no certificates. Machine-`invalid` rows still PRESERVE
the old engines' reports (law.legacy, keyed budget lowered rows)
as fingerprint-bound optional evidence - the compiler's own
cyclic-class artifacts admit through their own admission. The
catalogs are closed (unique, exact bijections with the public
sections; law.subjects tied to the model) so selector
recomputation cannot be widened underneath a certificate. Both
digests recompute at admission: law_digest is a canonical-JSON
fingerprint over the law rows (a row edit under a stale digest
refuses) and inputs_digest must match the consuming binary's
analysis snapshot. Evidence is validated, not presence-checked:
migrated violated/uncertified rows must retain their countermodel
/ residue, and violated certificates their diagnostics.

Round 6: the law account is EVIDENCE-CLOSED on every edge. The
`law` catalogs become canonical `(name, display)` pairs (loci,
groups, topics + wire subject, the wire-subject universe, and
`fn_universe`), and every resolved reference must match one exact
pair - the raw machine join key is anchored, so a singleton
`name` swap refuses; the catalogs cross-tie to the sections other
consumers join on. Bus selectors keep their candidate sets and
admission recomputes them from the catalogs with the compiler's
own matching rule (`bus_ref_matches`) - a candidate swap under an
unchanged selector name refuses. Every compatibility `lowered`
row is keyed to its law ordinal (and certificate ordinal), and
the section must project one-to-one from the typed account -
deleting law rows orphans their evidence even in an
annotation-only artifact. A new `law.legacy` report keys the old
engines' `causes:`/`depends:` verdicts by ordinal and by a form
fingerprint re-rendered from the typed operands (operand
mutations orphan the entry; a `causes:` row naming an undeclared
or cyclic class cannot hold). Fleet-family rows are refused
outright in application artifacts - the fleet account is Change
7's.

Round 5: the `law` section carries a `fn_universe` catalog (the
FULL model function universe - module-scoped annotation subjects
admit) and an `effect_classes` catalog (declared/cyclic status);
one shared admission (`validate_law_account`) runs for BOTH
Track A and fleet composition - fleet no longer trusts a
component's `verdict: clean` without decoding its law account,
and deleting law rows is refused by the two-way claims join, not
vacuously accepted; annotation laws bind to their evidence (a
certificate-class swap fails the re-rendered expected form; a
budget `per_call` mutation fails the lowered-row match; flipping
`resolved` to false cannot rescue a `holds`); and evidence
locations are source-space-honest - a foreign-space diagnostic
(stdlib parse space) is never re-resolved against bundle sources,
even when its offsets numerically fall inside a bundle file.

### The pointwise-certificate judgment - family 5e (GH #476 Change 5e)

`judgment::judge_certificates` - `@effects(none/only/publish)`,
`@no_panic`, and `@phase_effects` on the canonical model. The
certificate engines remain the one analysis authority: the builder
runs them (the grouped report shares the exact pass `hale check`
uses, split into strata) and stores each certificate's outcome and
diagnostics as typed `CertificateEvidence` on the model - the
artifact's #392 lowered rows, with their diags; the judgment
renders verdicts and diagnostics from model data, adds the
undeclared-class validation over the typed effect-class table, and
the lowering emits undeclared `is:` classes as issues (carries has
no law row). `Provenance::ForeignSpan` preserves stdlib-space
diagnostic offsets verbatim. Corpus differential compares the
evaluator's law strata as a multiset (cross-certificate stream
order differs by design between `hale check` and ClaimIr ordinal
order; within-certificate order is the evidence's own). Negative
control: clearing model.evidence invalidates certificates.
Remaining 5e surfaces (`@effects(causes/depends)` which live in
the check.rs graph pass, and `@budget`/quantitative dims) follow
in-review.

### The quantitative-bound judgment - family 5d (GH #476 Change 5d)

`judgment::judge_bound` - `bound C <= N on paths from G` on the
canonical model: call-tree SUM of carrier sites with MAX over
dispatch alternatives (shared site ordinals ARE the dispatch
groups), unbounded on recursion cycles, loop-nested carriers,
unfollowable calls, and computed subjects - the evaluator's
site_count ported over both vertex kinds. The absorption sidecar
gained count-grade facts (per-node carrier classes, loop bits and
dispatch groups on interior events) and Publish rows gained
in_loop, so a carrier inside a stdlib body or behind a loop-nested
send counts exactly as the evaluator counts it. Same corpus
differential (now five families); negative control drops labels.

### The endpoint/coverage/count judgment - family 5c (GH #476 Change 5c)

`judgment::judge_endpoints` - `require publishes/subscribes`
(declared-end existence over `declares_publish` and the
subscription rows, joined by canonical spelling), `require sealed`
(universal over group loci with the empty-projection vacuity
refusal), `require attributed` (the direct-site predicate computed
by the evaluator's own extracted rule into new model facts
`Function.attribution` + `Function.opaque_call`; the opaque-call
Uncertified fallback included), `cover` (seed topic domain via
`declared_in`), and `count` (distinct declared-end loci with the
full who-list rendering). Validation ports: unknown topics with
did-you-mean, unresolvable import paths, non-attributable classes,
topicless seeds. Adoption-collision diagnostics from the clause
enumeration surface as lowering issues at the head of the
diagnostic stream, exactly where the evaluator emits them. Same
corpus differential; negative control drops declares_publish.

### The boundary-grant judgment - family 5b (GH #476 Change 5b)

`judgment::judge_only_edges` - the `only edges` family on the
canonical model, direct-edge law with no walk: call edges into the
destination group are never grantable; bus edges match grants by
the subscription's canonical spelling (topic name for declared
subscriptions, authored pattern for literal/wildcard ones - the
same graph keying `subscribers_of` iterates, wildcards included).
Fail-closed holes, projection vacuity, unknown-group validation,
and the full three-diagnostic violation rendering (claim line,
un-granted publish site, delivered-at subscription) are ported
with the evaluator's exact spellings, held by the same corpus
differential as 5a. Negative control: dropping subscribe rows
flips the verdict.

### The reachability judgment - family 5a on the canonical model (GH #476 Change 5a)

`hale_types::judgment::judge_forbid_reaches(&ClaimIrTable,
&ApplicationModel, source_bases)` - the first judgment family
migrated onto `ClaimIr` x the canonical model, with full
diagnostics parity: verdicts AND byte-identical diagnostics
(messages, spans, related notes) against the authoritative
evaluator, held by a permanent corpus differential. The walk
reuses `model_graph::search` with a two-kind vertex: user
functions over model rows (site-grained calls, publish x
subscribe composition, `member_of` for group projection,
`phase_of` for `during`, typed holes failing closed) and interior
stdlib vertices from a new `LegacyProjection.stdlib_absorption`
sidecar - the interior GRAPH the evaluator's merged-summary walk
traverses (kept as a graph because BFS layering decides
hole-vs-hit timing), with witness spellings demangled through the
shared table. The validation pass (unknown names with
did-you-mean, effects()-in-source, undeclared classes,
avoiding-overlap, projection vacuity, duplicate claim names) is
ported over ClaimIr refs. `Function.direct_effects` joins the
model (computed by the evaluator's own `claims::direct_effects`).
Negative controls prove the engine reads its relations (dropping
call rows or hole rows flips verdicts). The old evaluator stays
live and authoritative until Change 9. No user-visible behavior
change.


### ClaimIr - every law surface, lowered (GH #476 Change 4)

`hale-model` gains `ClaimIr` - one typed variant per law form across
every surface the language has grown: the eight claims-block forms
(reachability, boundary grants, path bounds, endpoint existence,
sealing, attribution, coverage, cardinality), constitution clauses
with recorded origin, library-tier claims with alias attribution,
the annotation surfaces (`@effects(none/publish/causes/only)`,
`@no_panic`, `@effects(depends:)`, `@phase_effects`, `@budget` in
both alloc and quantitative forms), and deployment-plan claim rows
(name-level until Change 7's FleetModel).
`hale_types::claim_lowering::lower_claims(bundle, model)` lowers
the program surfaces through the evaluator's OWN clause enumeration
(`enumerate_clauses`, extracted from `claims_report_inner` so two
walks cannot drift); `fleet::lower_plan_claims` lowers plan rows.
Lowering is total over parseable programs (unresolved refs keep
raw+display spellings with no id - Change 5's `invalid` residue),
rows keep authored order, and a corpus-wide differential holds the
claims-family rows equal to the evaluator's outcomes on count,
name, order, and constitution source. Lowering only: the old
evaluators stay active and authoritative; nothing consumes these
rows yet. `@effects(is:)` deliberately does not lower - carries is
a classification fact the model already records as labels.


### The model-backed artifact projection (GH #476 Change 3)

`hale_types::topology_projection::project_model_half(&ApplicationModel)`
renders the topology artifact's hashed model half from the canonical
model alone, and `project_shape_hash` reproduces `TopologyShapeV1`
exactly - proven byte-for-byte against `dump_topology`'s own
derivation over every checkable corpus program by a permanent
differential gate (`tests/topology_projection.rs`). The projection
maps the model's raw canonical identities back to the artifact's
display spelling, merges site-grained rows to the legacy endpoint
grain, renders `calls_via_stdlib` from the preserved legacy
contraction rows, and re-folds typed holes + dead interface
dispatches into the legacy `unknowns` vocabulary. Both derivations
stay live until the Change-6 versioned identity transition: a model
change that would silently re-key `.halerec` replay admission now
fails the differential instead. No user-visible behavior change.

### The ApplicationModel builder - demand-gated derivation (GH #476 Change 2)

`hale_types::model_builder::derive_application_model(&Bundle)` - one
entry point assembling the canonical model from the same trusted
analyses the topology artifact consumes (AllocSummary, BusGraph,
model::Model, effects/frontier) plus direct AST reads for facts no
summary carries (topic key/bound policy, subscription filters and
bounds, authored group selectors, the declaration universe,
@sealed). Sites stay site-grained; through-stdlib contraction uses
the artifact's own walk; uninhabited dispatches land in
`dead_interface_calls` while genuine residue becomes typed holes;
capabilities are COMPUTED from the holes so the two completeness
accounts cannot drift. Ownership/placement/bindings stay empty
tables with capabilities false (Change 8 completes them - the
artifact exports none today).

DEMAND-GATED, proven cross-process: the builder runs only for
`hale model dump <target>` (new, internal non-stable format,
same ill-typed refusal as the artifact); plain `hale check` - even
with claims present, even with `--dump-topology` - provably never
derives it (HALE_MODEL_TRACE=1 stays silent; pinned by test).

Tested three ways: family fixtures (keyed delivery incl. fallback
+ replica, bounds, literal/wildcard endpoints, supervision,
groups, dead-vs-indirect separation); a model/artifact
DIFFERENTIAL asserting both extractions agree on fns, loci, calls
(endpoint projection), through-stdlib edges, publishes,
subscribes, unknown anchors, authored group selectors,
supervision, and per-fn effects; and a whole-corpus property -
derivation never panics on any parseable program, and a derived
model may violate a schema law ONLY where the checker also
refuses the program (negative fixtures' models mirror the
checker's refusals; checks-clean-but-unlawful is a builder bug).
The corpus property earned its keep immediately: its first run
caught a fail-open glob-alias fallback and two non-total lookup
paths in the builder.

### `hale-model` - the canonical semantic model schema (GH #476 Change 1)

The first change of the canonical-model epic: a new source-independent
crate holding the typed schema and its laws - nothing derives it yet
(Change 2) and nothing consumes it yet, so there is zero behavior
change. What Change 1 pins is exactly the set of facts that would be
expensive to retrofit: typed entity sorts (declaration/instance split
included) and relation tables with per-row provenance; keyed delivery
as first-class schema (topic keys, publish key domains, subscription
predicates incl. `EqReplica`, the may/must-deliver polarity documented
as law); bounds and loss policy (capacity, shed policies, publish
dispositions, binding loss behavior) recorded BEFORE any must-arrive
claim exists to consume them; typed holes that must hide at least one
relation family; positive capabilities with a validated
no-contradiction law (a model cannot claim `exact_calls` while a hole
hides CALLS); `ModelHashKind::TopologyShapeV1` naming the legacy hash
algorithm so the Change-3 projection cannot ship an unnamed identity;
and canonical-order validation (an unsorted table is not a model).
The crate depends on NOTHING - source independence and
no-serialization-promise are enforced by an architecture canary that
fails on any dependency line at all. Design note ships as the crate's
rustdoc; 8 canary tests in `tests/architecture.rs`.

### `hale topology graph` - deterministic visuals from the artifact (GH #476 Track A)

A committed `--dump-topology` artifact can now be rendered:
`hale topology graph <artifact> [--view system|code|bus|claim|residue]
[--format svg|mermaid|dot] [--claim NAME] [--config file.json] [-o out]`.
An ARTIFACT CLIENT by construction - it reads exactly the JSON a third
party reads, never Hale source, and has no dependency on the future
canonical-model crate (its input adapter swaps later without changing
output). Deterministic by design: fixed character-cell text metrics
(no font measurement - byte-stable across machines), stable IDs from
artifact names, no timestamps; rendering twice is byte-identical and
moving source lines does not move a pixel (spans change, the model
shape does not - pinned by test). The claim view highlights the named
claim's groups and states its verdict on a card; the residue view
renders unresolved holes as first-class nodes, and every other view
notes them on a card rather than omitting them. Experimental surface
pre-1.0. Pinned by `topology_graph_cli.rs` (7 tests incl. checked-in
mermaid/SVG goldens for a pinned fixture); docs claims chapter gains
the rendering section.

### Listen-binding ingest no longer loses messages at the edges (GH #468)

Three defects, one delivery contract. The issue's two observed
shapes (boot-window drop, tail loss "at peer close") plus the real
mechanism behind the second, found while root-causing:

- **The unlocked-queue enqueue race - the actual mid-stream loss.**
  `program_has_offthread` counted adapter bindings but not unix/udp
  ones, so a plain `bindings { }` program ran the cooperative queue
  UNLOCKED while two binding reader threads and the main drain
  raced it - a concurrent enqueue pair could tear, silently losing
  a message the counters show as delivered. Under recording the
  window widened to ~1-in-3 on an idle machine. Fixed on both
  layers: any `bindings { }` block now bakes the off-thread flag at
  compile time, and the runtime re-asserts `lotus_bus_mark_pinned`
  whenever it spawns a reader thread (covers `LOTUS_BUS_CONFIG`
  routes codegen cannot see).
- **Boot-window buffering.** A message received between transport
  realization and the same birth's later subscriber registrations
  was silently dropped by the no-deserializer path. Readers now
  buffer it - bounded per binding (64 msgs / 1 MiB, oldest-first
  eviction) - and flush FIFO the moment a matching registration
  lands, so an in-run waiter sees it without another inbound
  message. Counted as `buffered_early` / `dropped_early` in the
  counters dump. Relay-shaped programs degrade to the old drop
  behavior at the cap, counted instead of silent.
- **Exit quiesce.** Teardown freed the deserializer registry
  BEFORE joining readers, so a storm-descheduled reader drained
  its kernel queue into an empty registry - the boot-window drop
  mirrored at exit (the kernel itself never discards: AF_UNIX
  queued data survives peer close and even `shutdown(SHUT_RDWR)`,
  verified by probe). Codegen now emits
  `lotus_bus_ingress_quiesce` at every main-exit point before
  pools join and loci dissolve: listen fds half-close, readers
  drain to true EOF through the intact registry, buffered residue
  flushes, one final local drain delivers to live handlers.
  Bounded by `LOTUS_BUS_QUIESCE_MS` (default 500ms, 0 disables) -
  a silent peer holding a connection open cannot stall exit.

Also hardened: the non-framed SEQPACKET recv/send retry `EINTR`
instead of treating a signal as connection death (the old -1 broke
the serve loop and the re-arm `close()` discarded the queue). And
one follow-up the ASan corpus caught: the reader thread used to
destroy its transport on exit, racing the quiesce's second
`shutdown()` the instant the first one woke it (use-after-free on
`t->conn_fd`). Ownership law now: the reader NEVER destroys the
transport - destruction happens only on the main thread after
`pthread_join` (reclaim or teardown), where no concurrent reader
can exist.

The two-listener replay CLI test drops its record-session retry
(the issue's promised cleanup) - a lossy live session is now a
regression, not weather. New deterministic canaries in
`binding_ingest_468.rs` drive the windows with test-only env hooks
(`LOTUS_BUS_TEST_BOOT_HOLD_MS`, `LOTUS_BUS_TEST_READER_STALL_MS`).
Spec: the publish contract gains the listen-side delivery bullet
(semantics.md); runtime.md documents the new knob and counters.

### `std::http::is_route` - one locus, many endpoints

`std::http::build_context(req)` + `std::http::is_route(ctx, method,
pattern)` make a single locus a complete API surface: its own
`handle` dispatches through an `if`-ladder of `is_route` checks and
each endpoint is a plain method with `self` in scope - so shared
per-instance state (a db handle, a session table) needs no Router
and no per-endpoint `RouteHandler` loci. `build_context` builds the same
per-request bundle the Router does (query string split into
`params.qs`); `is_route` runs the Router's own matcher (`:name`
captures via `path_param`, trailing-slash tolerant, no implicit
wildcard), fills captures on a hit, and clears them on ANY miss -
including a method-only miss - so the ladder is first-match-wins in
written order with no stale-capture bleed. Composes with
`pinned(..., replicas = K)` placement: each replica is its own
instance, so that db handle is per-thread by construction. Pinned
by `tests/hale/is_route_test.hl`; docs routing chapter and spec
stdlib row updated.

### Stdlib values get their real types - the fail-open `Ty::Unknown` class is closed (GH #470)

Found via a downstream server whose responses curl rejected: a
middleware locus with the wrong `after` arity typechecked and the
Router's interface fat-pointer call corrupted the HTTP response
in-memory (garbage status integer, dangling header, the request's
method and path bleeding into the output). Root cause: stdlib
qualified paths resolved to `Ty::Unknown`, which is bidirectionally
compatible - so `router.use(badMw)` performed no signature lookup,
no interface-satisfaction check, and codegen coerced blindly. Even
`let x: Int = router;` and `std::log::TotallyFakeSink {}` (a
nonexistent name) typechecked.

The fix is the one the sealed-loci injection (GH #436) deferred:
the ENTIRE Hale-source stdlib surface - loci, types, interfaces,
free fns - is now registered into the checker's top scope
(signatures only; stdlib bodies are never added to the checked
bundle, so no diagnostics can land on stdlib source). `std::...`
type expressions and struct/locus literals resolve through
PATH_RENAMES to their nominal symbols and validate for real:
literal fields are checked, methods resolve against true
signatures, interface coercions run the structural verifier, and a
`std::` literal that matches nothing is a hard error. Diagnostics
render the public spelling (`std::http::Middleware`, not the
mangled name). Rust-implemented builtins keep their historical
tolerance (their path-call names were already validated by the
stdlib surface registry).

Measured blast radius: the whole fixture corpus and workspace pass
untouched except two tests that pinned the old permissiveness -
one of which was an invalid fixture the tolerance had been hiding
(a call to a Router method that does not exist). Ergonomics
follow-through: `std::http::Request` gained defaults for
`version`/`headers`/`body` so the documented hand-built-Request
pattern stays one line. Canaries: the wrong-arity middleware, the
`Int = router` absurdity, and the typo'd-literal case are all
compile errors now.

### Record & replay, phase 6: `where async_io` pools replay (GH #296)

The last refusing surface joins the replay story. The
nondeterminism of an async pool is its drain's SCHEDULING — which
cell starts when, which parked coro resumes when (epoll readiness
order), when a timed park expires relative to both. Recording now
stamps every such decision on the pool worker's private ring as a
step stream (`ASYNC_START`/`ASYNC_RESUME`/`ASYNC_EXPIRE`; coros
named by birth ordinal, stable across runs because start order is
enforced), and replay DRIVES the drain from that stream instead of
the clock:

- START steps reuse the phase-4 cell gate unchanged (start order
  is consume order; the async live path now gates too, closing the
  never-reached gap the old refusal hid);
- RESUME steps wait for the named coro's readiness — early-ready
  coros park aside until their recorded turn;
- EXPIRE steps resume immediately with the timed-out sentinel: the
  recording proves the deadline fired here, so replayed sleeps
  fast-forward rather than re-waiting wall-clock time;
- an unsatisfiable step counts an `async-schedule` divergence
  (new status-file key, new summary row) and is skipped — degrade,
  never deadlock; a dry or pre-phase-6 tape hands the pool back to
  the live drain.

Hardened by two review rounds. Round two closed the degradation
and compatibility paths: a skipped START retires its birth ordinal
AND its consume slot (one missing delivery no longer cascades into
wrong resume pairings — steps naming a retired slot skip
immediately); the dry-tape flag is set BEFORE the ready-head/held
flushes so post-tape work is always classified; the CLI comparator
carries the artifact's async-capability bit and skips schedule
comparison for pre-phase-6 artifacts (matching the runtime's
coverage note instead of contradicting it); the one-shot warning
is atomic; the test module is Linux-gated; and the public
`hale replay --diff` path is exercised end to end (exact match,
mutated-step failure, and old-artifact compatibility).

Round one: the artifact carries an
async-capable header bit, the dry-tape states are named instead of
silent (pre-phase-6 artifact → one-shot coverage note; truncated
tape → stated coverage boundary; a finalized tape running dry
under continued async work → `async_post_tape` divergences, with
schedule steps left unconsumed at exit as
`unconsumed_async_steps`), and `--diff` compares the per-consumer
async step streams bidirectionally like every other stream.

Boundary, stated: the schedule replays; the data of unjournaled
I/O still re-executes live (syscall-class, gated), and bindings
ingress arrives via the phase-5 injector. Tests: a staggered
two-locus async pool records and replays byte-identically with
zero divergences; a 2.5-second recorded park replays in a fraction
of its own measured recorded wall time (the fast-forward PROOF —
the old bound was satisfiable live); readiness arriving in the
OPPOSITE order of the recorded resumes is held on ready_head and
replayed in tape order across a mixed START/RESUME/EXPIRE stream;
and two replays of one recording agree with each other.

### Record & replay, phase 5: durable recording, the hermetic wire, and feed mode (GH #296)

Three deliverables close the two loudest gaps in the v0.17.0
replay story — "a crashed run loses its recording" and "a replayed
server talks to the real world" — hardened by two full adversarial
review rounds (10 + 8 findings, all resolved; the notes below fold
them in). Round two closed the trust boundaries: ONE file object
carries the recording from CLI admission into the child (no
reopen, no path-substitution window, plus a runtime
identity-vs-binary defense for direct invocations); the eager
crash-identity stamp is a release/acquire COMMIT of the complete
identity (the old cross-thread reads raced and could persist a
half-published digest); a header-only 96-byte file — the earliest
crash window — admits as a prefix under `--allow-truncated`; the
feed verdict derives its unclassified remainder at report time, so
an `std::process::exit(0)` before injection can never ride to
success; and boot-snapshot injection names late-created
subscribers as their own `late_subscription_uncovered` coverage
class (a teardown-time registry rescan distinguishes them from
genuinely absent subscribers).

**Durable recording / crash-prefix recovery.** Named precisely: a
durable flight recorder, NOT a write-ahead log — the application
never gates on the recording reaching stable storage. The header
identity (`model_hash`, `exec_digest`, policy flags) is stamped
eagerly — not only at finalize — so a SIGKILLed run leaves an
artifact that is attributable and exact up to one torn frame at
the tail. Replaying a trailer-less recording is an explicit
opt-in (`hale replay --allow-truncated` /
`LOTUS_REPLAY_ALLOW_TRUNCATED=1`): both loaders stop at the first
incomplete frame, report the parsed extent, and replay that
prefix. Under `--diff` the runtime verdict and the comparator
AGREE on prefix semantics: recorded-history-exhausted events
count into a separate `post_prefix_live_fallback` status key —
the unknown post-crash suffix, executed live — while any mismatch
inside the prefix stays a divergence (a withheld env read inside
the prefix still fails the diff; the surplus past the tape does
not). A finalized artifact keeps the exact full-parse + count
checks. `LOTUS_OBS_RECORD_DURABLE=1` is the power-loss grade:
`fdatasync` per flushed sweep AND on the finalize trailer,
parent-directory sync at creation, grade recorded in the header.

**Hermetic wire + ingress injection.** Hermeticity is a
binding-kind capability, not a blanket assumption: native
`unix://`/`udp://` transports (and the transport-locus form) are
suppressed at realization — husk entry, no socket, outbound
fanout sends nothing (still recorded and compared under
`--diff`) — while a backend with NO replay class fails closed:
`shm_ring` is refused by the CLI (named) and independently at the
runtime shm-open seam. Injection runs as an explicit boot phase:
codegen emits `lotus_replay_start_ingress()` at the main locus's
boot/run boundary, where the runtime snapshots the subscription
registry on the main thread (injector workers never read the live
registry) and spawns one worker per RECORDED ingress source, each
carrying its source's recorded consumer identity so `--diff`
aligns per-consumer streams across multiple listeners. Tape
identity is the full subject string plus the subject's canonical
payload shape (FNV-32 kept for reporting only — it is
collision-prone, and a colliding pair now routes correctly by
name); a shape mismatch refuses injection as *incompatible*
rather than feeding plausible wrong values. Only ACCEPTED
messages enter the tape — wire capture runs after the
deserializer says yes, so rejected traffic is never re-fed.
Strict injection is PACED against recorded progress (inject one,
wait for its recorded consumes) so bounded queues shed nothing
they didn't shed originally. Every tape entry is classified
(injected / rejected / unmatched / incompatible / unprocessed-at-
shutdown / start-failure); under strict replay every non-injected
class is a machine-readable divergence. Found along the way: the
injector, as a cross-thread producer, must select the locked and
GATED main-queue drain (`lotus_bus_mark_pinned`) — without it a
replaying main drained ungated and cross-source order silently
diverged with a clean verdict. Consequence of hermetic native
wire: `bindings` blocks with covered backends no longer trip the
`--allow-live-effects` gate; the remaining residue
(`syscall`/`ffi`/`unclassified`) is genuinely user-level.

**Feed mode — same recorded ingress, changed code, live
nondeterministic environment.** `hale replay --feed rec app.hl`
(`LOTUS_REPLAY_FEED=<path>`) consumes the recording as an input
tape: recorded ingress injected, wire hermetic, no journal
serving, no order enforcement, no model admission (a hash
mismatch prints as information; feeding a tape to changed code is
the point), no `--diff`/`--at`. The effects gate STILL applies —
`--feed` bypasses identity admission, never effect safety;
live-effect programs need `--allow-live-effects` beside it. Feed
targets that dropped or rearranged the listener binding still get
their tape (tape presence decides injection, not the old
declaration). The exit report classifies every tape entry, and an
unfed remainder fails the run by default —
`--allow-unmatched-feed` is the explicit acceptance.

Tests, beyond the happy paths: SIGKILL-prefix admissibility +
eager identity stamp + refusal naming the flag; durable-grade
finalize; hermetic strict replay (byte-identical stdout, zero
divergences, socket never created); truncated `--allow-truncated
--diff` accepting the post-crash surplus while an in-prefix
withheld read still fails; feed requiring the effects gate;
rejected wire excluded from the tape; the FNV-32 collision pair
routing by full subject; capacity-one bounded queue replaying
with zero shedding; feed into a binding-less target; incompatible
payload shape failing feed closed; `shm_ring` refusing replay
without touching shared memory; two listeners surviving CLI
`--diff` with per-source identity.

---
## v0.17.0 — the run comes back the same (2026-08-13)

### Record & replay, phases 2–4: `hale replay` (GH #296)

Building on the phase-0/1 recording below: **re-run a recorded
execution and get the same schedule and the same journaled inputs,
with an explicit, checked coverage boundary.** (Scoped per review:
external ingress and non-journaled I/O re-execute live and are
gated — see the safety flag below.)

Hardened by two full review rounds before merge. Round 2 (the
identity/admission/trust round): the safety gate consumes TYPED
effect rows and fails closed on `unclassified` and on
transport-bound `bindings` (a bound publish sends real traffic
that user-level effects cannot show); `exec_digest` is a framed
SHA-256 over the toolchain source hash + version + options + full
source paths/lengths/contents (32 bytes in the header, stamped in
four parts, re-stamped at finalize); journal entries carry their
EXACT encoded arguments (replay memcmps them — the 32-bit hashes
they replace folded adjacent integers); env VALUES are withheld
from recordings by default (`LOTUS_OBS_RECORD_ENV=full` opts in;
withheld reads replay as named divergences); raw in-process
payloads store metadata only (no pointer/padding bytes on disk);
`msg_id` is full-width (consumer:16|seq:48, loudly guarded) and
the delivery identity includes the target locus; the runtime
loader independently validates the whole artifact (one open +
fstat + private mmap, checked arithmetic, exact trailer position
and count, per-kind value shapes); `--diff` compares per-consumer
PUBLIC bus streams via file-side identity maps (direct dispatch
is visible) and groups journal comparison per consumer.

Round 1: recorder events
moved off iris protocol ekinds onto process-private rings; replay
admission is by executable identity (`exec_digest` = compiler
version + source bytes), not just structural `shape_hash`, and
unstamped recordings are refused without `--allow-unverified-model`;
replay is **safe by default** — a program whose effect frontier
reaches `syscall`/`ffi` is refused without `--allow-live-effects`;
capture failure, write failure, and finalize failure all fail the
run (never a silent gap); a recording is clean only if the entire
artifact validates (exact trailer position + entry count); the
journal carries per-call argument identity (a changed env name or
rand bound is a named divergence, not a substituted value);
`--diff` is bidirectional and fails on ANY runtime divergence via
a machine-readable verdict; payload topic identity is a stable
subject hash (manifest ids are registration-order and race);
raw-struct payload captures are flagged as ABI snapshots and
compared by size (canonical recording codecs staged); `--at
consumer:N` is the stable multi-consumer debugger coordinate.

- **`hale replay <recording> <program.hl>`** — compiles through the
  same pipeline as `hale run`, admits the recording by
  `model_hash` (a recording from a different model is rejected,
  never misreplayed; a truncated recording is refused), then
  re-executes under `LOTUS_REPLAY`. `--diff` records the replay and
  reports the first per-consumer divergence; `--at N` stops
  (SIGSTOP) at the Nth consume so a debugger can attach.
- **Delivery identity + payload capture** (format v0.2, tagged
  entries): every queued delivery carries a deterministic
  `pub_id` (stable consumer id + per-publisher-thread seq — a
  re-executed run re-derives the same ids with no global
  coordination); payload bytes are captured once per queued
  publish, external wire ingress flagged. Synchronous direct
  dispatch captures nothing by design: a closed-world same-thread
  call cannot carry external input.
- **The input journal**: `std::time::now`, `std::time::monotonic`
  / `monotonic_ns` (now a named runtime primitive —
  they lowered as inline `clock_gettime` IR nothing could
  interpose), `std::rand::next_int`, `std::os::getrandom`, and the
  `std::env` surface are journaled per consumer under recording
  and served back under replay. A read past the recorded history
  falls back live and is counted — **replay degrades, never
  refuses** — with a divergence summary at exit.
- **Recorded-order enforcement**: each consumer (cooperative pool
  worker, pinned mailbox, main queue) re-consumes its queued
  deliveries in the recorded order, holding early arrivals in a
  per-consumer buffer with a bounded (1s) degrade path, so two
  racing pinned publishers replay in exactly the interleaving that
  was recorded — pinned end-to-end by a CLI test that replays a
  40+40-delivery race twice. `where async_io` pools refuse replay
  loudly (their coro interleaving is a later milestone).
- Remaining, explicitly staged: ingress injection + fleet replay
  (Phase 5, needs `LOTUS_OBS_WIRE` fleet identity), and
  replay-under-a-different-plan (Phase 6, blocked on #262's plan
  admission).

### Record & replay, phases 0–1: the determinism guarantee, and a recording that never drops (GH #296)

- **Single-pool determinism is now a stated guarantee**, not an
  implementation accident. A program whose loci all run on the main
  cooperative scheduler produces the same publishes and deliveries
  in the same order on every run, given the same inputs — one pool
  is one consumer thread by construction, so there is no scheduling
  freedom for an order to vary within. Written into
  `spec/testing.md` § Determinism (both former "determinism mode"
  open questions resolved against it), pinned by
  `replay_determinism.rs`, which compares the complete ordered
  BUS_PUBLISH/BUS_DELIVER stream across repeated runs. A divergence
  there is a runtime bug, never a flaky test.
- **`LOTUS_OBS_RECORD=<path>` — lossless recording mode.** The
  observation plane's sampler disposition (overwrite-oldest,
  count the drops) becomes a flight recorder's: an in-process
  drain appends every ring record to the file, a producer whose
  ring is full **blocks** against the drain cursor instead of
  overwriting, a thread that cannot get a ring **fails the run**
  (recording defaults to the 64-ring maximum first), and a write
  failure fails the run rather than truncating silently. Never
  drop, by construction. Implies `LOTUS_OBS=1` and counts as an
  attached observer, so a recording needs no external consumer.
  Wholly opt-in: unset, the lowering is instruction-identical to
  the unobserved build, per the established gate discipline.
- **`BUS_CONSUME` (ekind 8, recording mode only): the per-consumer
  delivery order.** BUS_DELIVER is enqueue-time and lands on the
  publisher's ring — it cannot say in what order a consumer ran
  its handlers. Under recording, every dequeue-driven handler
  invoke stamps a consume record on the consuming thread
  (subscriber locus + the thread's gapless count); its ring
  position is the order a replay will serve back. Synchronous
  direct dispatch deliberately emits none — its deliver record is
  its consumption point. Never emitted outside recording, so
  existing consumers never meet an unknown ekind.
- The recording file format is **pre-stable** (v0.2 as shipped:
  tagged header + ring records + payload/journal blobs +
  clean-finalize trailer); the fully self-describing artifact
  shape may still evolve with the remaining GH #296 phases.

### Observability: supervision in the model, backpressure on the wire, identity in the segment

Five items from a downstream observability handoff, resolved as one
batch:

- **Supervision is in the topology artifact** (schema 1.10, hashed —
  existing `shape_hash` values change). `on_failure` had no
  representation at all, while `RESTART`/`SUPERV_TRANS`/`DISSOLVE`
  are the richest live signal an observer has — every restart was
  visible with no declared policy to belong to. One `supervision`
  row per handler: supervising locus, supervised child + error
  types, the recovery ops the body invokes, and a literal retry
  bound when written (`restart(c) for 3`); spans ride in
  `provenance.supervision`. A policy change now moves the model
  identity, and "declared retry cap 3, observed 3 in 40 s" is an
  annotation a consumer can actually draw.
- **The per-binding backpressure cells are written.** PROTOCOL §6
  reserved `queue_depth` / `send_block_ns` / `retries` (cells 3–5)
  since v0; no path wrote them, so a saturated edge was
  indistinguishable from a healthy one until something dropped.
  Now: cell 3 is a last-write-wins gauge of kernel send-queue
  occupancy sampled at send time, cell 4 accumulates transport-send
  duration (a stalled consumer makes it explode), cell 5 counts
  reconnects — counters-tier, measured only under `LOTUS_OBS`.
  Pinned by an overloaded-consumer test: depth climbs and block
  time accrues ahead of any loss.
- **The observation segment carries the model identity** (proto
  0.2): a `model_hash` u64 at header offset `0x80` — the topology
  artifact's `shape_hash`, computed by the CLI from the same bundle
  it typechecks and stamped in the codegen prelude. A consumer
  joining a live manifest against a source-derived artifact can now
  establish the running binary was built from the model it compares
  against; a comment-only rebuild keeps the value, a model change
  moves it. Harness builds read 0.
- **Topic declarations carry provenance spans**: every name in
  `sorts.topics` now has a `provenance.decls` entry, so an editor
  lens can anchor on the `topic` line a developer actually looks
  at, not only on the publish/subscribe sites.
- **The spawned-publisher counting claim closes as stale**: the
  exact missing conjunction from the field report — an
  `accept()`-spawned publisher on a remote-only *plain* topic with
  the observer attached after steady state — counts correctly on
  current HEAD (40/40, five runs), and is now a permanent pin
  beside the four earlier flavors.
### Placement pairings: replica-sharded delivery and pool affinity

Two compositions the placement matrix was missing, found writing a
webserver tutorial: partitioned *placement* never meant partitioned
*delivery* (bus pubsub is broadcast; delivery selection lives on the
subscription — deliberately, so placement stays semantics-free), but
the two axes had no bridge, and cooperative pools took no affinity
at all.

- **`where key == replica`** — each instance of a
  `pinned(..., replicas = K)` fan-out registers its 0-based replica
  index as its subscription key, so K replicas shard an Int-keyed
  topic with one subscribe line and K spelled once, in the placement
  entry. A non-replicated instance is replica 0. Placement stays
  semantics-free: the filter is written on the subscription; the
  placement only decides how many indices exist. The webserver
  shape becomes: listener publishes `Conn { fd, shard: fd % K }`,
  workers subscribe `where key == replica`. Requires an Int-family
  key; `replica` is contextual (only that exact RHS position).
- **Every `where key == …` filter now requires a keyed topic.**
  Closes a silent pre-existing trap: a Specific filter on an
  unkeyed topic registered a key match no publish would ever run —
  the subscriber received nothing, forever, with `check` green.
- **Pool affinity** — `cooperative(pool = X, core/cores/node/l3 =
  …)` binds pool X's worker thread to the core set, the same forms
  and topology-name resolution `pinned` has, kernel-verified by
  test. One pool has one worker: entries naming a pool must agree
  (a bare entry inherits; a *different* affinity is a type error
  citing both entries), and affinity on the main pool is rejected
  (that thread belongs to the operator).

### `std::http::Router.add_fn` — a route can be a bare function

Requested ergonomics: registering a route no longer requires
declaring a `RouteHandler` locus when there is no state to hold —
`router.add_fn("GET", "/hello/:name", greet)` takes a plain
`fn(Context) -> Response`. The fn pointer is stored in the route
entry itself rather than behind an adapter locus (an adapter
instantiated inside the register method would dissolve at method
exit, out from under the entry), so fn routes and locus routes share
one list and one first-match-wins precedence order, and captures,
query params, middleware, and the 404 default all behave
identically. `add` remains the stateful form.

### A subscriber handler's parameter type is checked against the payload

**Soundness fix**, from a downstream handoff. The spec has always
said a subscriber's handler signature must match the topic's payload
exactly (semantics § type-check rules, rule 1) — but nothing enforced
it. Any parameter type was accepted, and the published value was
reinterpreted field-by-field at the handler: a `String` field read
through an `Int` parameter printed the string's **heap pointer** —
a live, ASLR-moving address obtained from safe code, with `check`
and `verify` both green. Sharper still, the string-subject `of type`
conflict diagnostic already modelled this exact hazard and its
advice steered users toward the unchecked `topic` construct.

The subscribe-validation loop now compares the handler's parameter
against the subject's payload for **both** subject forms (declared
topics and string subjects with `of type` — the latter was only
checked cross-site, not at the handler boundary), enforces
one-parameter arity, accepts `Drain<T>` batch handlers by their
element type, and leaves `Unknown` payloads (cross-seed topics,
stdlib paths) permissive. The natural mistake that led to the report
— annotating the parameter with the **topic** name
(`fn on_hello(msg: Hello)`) — gets its own message naming the
payload type to use; it previously survived to codegen and died as
`unknown type name`, mangled and ungreppable across a seed boundary.
Reported at the parameter's own span: the caret is on the thing to
change. Verified against the full downstream corpus (pond, native
suites) with zero false positives.

### `hale lsp` follow-ups: stdlib-cache files stay diagnostic-free

The materialized stdlib cache from the go-to-definition feature had
a sharp edge (same handoff, reported live): jumping into
`~/.cache/hale/stdlib-<version>/` and opening the file sprayed
spurious errors over correct stdlib code — the per-domain files only
resolve inside the merged program, not as standalone seeds. The LSP
now recognizes stdlib-cache paths and publishes an empty diagnostic
set for them (clearing, not skipping, so anything a client already
showed is removed).
### `hale init` bootstraps a project

There was no way to scaffold a project — `hale.toml` was hand-written
from the spec. `hale init [dir]` (default: the current directory) now
writes the canonical minimal shape: a `[deps]`-only `hale.toml`
skeleton with the entry syntax in a comment, a hello-world `main.hl`,
a first `tests/main_test.hl` (in a subdirectory, per the seed model —
a test file carries its own `fn main`, so beside `main.hl` it would
collide; it imports the parent seed, the established convention), and
a `.gitignore` covering the build artifact and `vendor/`. The
scaffold is fmt-canonical from birth and passes `run`, `test`, and
`verify` out of the box — all asserted by test. Strictly
non-destructive: existing files are kept and reported, so `init` is
also safe for filling gaps in a partially-scaffolded directory.

### `hale lsp`: go-to-definition on `std::` paths

Downstream handoff. `textDocument/definition` on a `std::` path
(`std::http::Server`, `std::bytes::BytesBuilder`, …) now jumps into
the stdlib's own source. The rename table maps the user-facing path
to the mangled declaration in the embedded `AP_SOURCE`; the
declaration's owning per-domain file (the stdlib is a concatenation
of `core.hl`, `http.hl`, … — now exported per-file as `AP_FILES`,
with a test pinning the concatenation layout the span math depends
on) is materialized into a versioned read-only cache
(`~/.cache/hale/stdlib-<version>/`), and the location is a plain
`file://` URI. That answers the handoff's install-story concern —
an `install.sh` binary has no stdlib checkout — without the
synthetic-URI schemes rust-analyzer-style setups need client
support for: a materialized real file works in every editor.
C-backed path-call primitives (`std::str::*` and friends) have no
Hale definition and still return nothing.

### Diagnostics carry secondary locations, and the pre-pass stops eating them

Two defects from a downstream editor-tooling handoff, both worst on
the duplicate-top-level-name error — the easiest error to hit under
the per-directory seed model (a second file with its own `fn main()`).

**Structured related spans.** The duplicate-name diagnostic rendered
the previous declaration by `{:?}`-formatting a `Span` into the
message (`… at Span { start: Pos(5), end: Pos(11) }`) — visible in
every editor via the LSP. `Diag` now carries
`related: Vec<(Span, String)>`; the site that raises the error has no
source text, so the renderers — which have the sources and the
file-base table — resolve each entry: the text renderer emits
`note: previous declaration at path:line:col` (cross-file correct),
`--json` adds a `related` array (absent when empty, so existing
consumers see an unchanged shape), and `hale lsp` publishes
`DiagnosticRelatedInformation`, which clients render as a clickable
second location. The two other `{:?}`-span leaks found by the sweep
(duplicate topic binding, duplicate capacity slot) got the same
treatment, and the fn/const duplicate path — which had no previous
location at all — now recovers it from the symbol table.

**Pre-pass diagnostics route through normal reporting.** Resolver
diagnostics raised inside the `apply_sync_inference` pre-pass were
printed through a bare `render()` and an early bail: no filename,
`--json` silently ignored (empty stdout with exit 1 — a CI gate saw a
failed build with zero explaining diagnostics), the wrong stream, and
multi-file positions resolved against the alphabetically-first file's
text (coordinates in neither file). The pre-pass now discards its
return value and lets `check_bundle` re-raise through the normal
path — exactly what `hale lsp` always did, which is why the LSP
attributed the same diagnostic correctly while the CLI did not. The
`run`/`build` twins of the bail are gone too (they double-reported).

### An intra-subtree publish is no longer invisible to observation

Downstream handoff (P23). `desugar_intra_locus_topics` rewrites a
`Topic <- payload` whose only subscriber lives inside the publisher's
own locus subtree into a direct call to the subscriber's bus handler
— before lowering, so the handoff-5 fix that gave the *codegen*
direct-dispatch flavors their probes never reached it. Delivery
worked; observation saw nothing: no `BUS_PUBLISH`, no `BUS_DELIVER`,
no counters, and no manifest row at all — "declared but never
published" and "compiled to a direct call" were the same absence,
which poisons any source-topology-to-manifest join.

The desugared call site (already identified in codegen by the
payload-reclaim work — a bus handler is not callable from Hale
source) now emits both probes, branch-gated on `lotus_obs_live` like
every other flavor: `BUS_PUBLISH` attributing the publisher,
`BUS_DELIVER` attributing the subscriber, enqueue-time-equivalent
(the direct call is the delivery). The first probe creates the
topic's manifest row, so a trafficked intra-tree topic registers and
counts exactly like its bus-dispatched sibling; a zero-traffic topic
stays absent on every flavor uniformly, so absence once again means
"never mentioned at runtime". Pinned by the reporter's controlled
pair (child-subscriber topic vs sibling-control topic in one binary),
A/B'd against the pre-fix compiler.

(The report's secondary observation — the sibling subscriber showing
zero deliveries — is the documented birth-order trap, not this
defect: the publisher was declared first with a long-running `run()`,
so the sibling was never born during the burst.)

### Element chains: the second vocabulary tranche

The remainder of the chain vocabulary recorded open when #380
shipped, minus stream sources (still future work). Everything fuses
into the same single loop — or, for the whole-set terminals,
materializes into caller storage — so the zero-allocation contract is
unchanged.

New **stages**:

- `take(n)` / `skip(n)` — positional selection. Both count elements
  arriving at their own position in the chain, so
  `filter(p).skip(2)` skips the first two *matches*; the two orders
  of `skip`/`take` are different chains and both are pinned by
  tests. Limits are evaluated once, before the loop.
- `enumerate()` — binds `idx`, the 0-based count of elements
  reaching the stage, in every later stage and the terminal. An
  explicit opt-in stage rather than an always-bound name, so a
  chain never captures a user's own `idx` local; a second
  `enumerate` shadows the first for everything after it. A
  `min`/`max` key mentioning `idx` is rejected rather than silently
  miscompared (the best element's count is not recoverable from the
  element at compare time).

New **terminals**:

- `sum(seed)` — the seed is the accumulator's starting value *and*
  its typed zero, so `sum(0.0)` sums Float elements and `sum(100)`
  starts an Int sum at 100. Closes the "Int elements only" gap
  without literal suffixes, and stays pre-typecheck-safe.
- `sort_into(target, cmp?)` / `reverse_into(target)` — push the
  survivors into a caller-owned `@form(vec)`, then reorder it in
  place (the vec's own `sort()`/`sort_by(cmp)`, or an end-swap
  loop). The spec's boundary paragraph already said whole-set
  operations are "terminals that materialize into caller storage";
  now they exist.
- `group_count_into(target, key?)` — one hashmap `bump(key)` per
  survivor (increment-or-init tallying); the bare form keys on the
  element itself. Accumulates across chains, since `bump` does.

The whole-set terminals are recognized even stage-less
(`xs.sort_into(sorted)`) — their compound names belong to the
vocabulary, unlike bare `into`. The recognition gate's it-mention
walker also learned to see through if/match/block expressions, so a
conditional key (`group_count_into(t, if it % 2 == 0 { "even" }
else { "odd" })`) counts as the it-mention it is.

Coverage is pairwise over the interaction classes — every new stage
against every terminal family, both orders of the order-sensitive
pairs, `idx` reach into each-blocks and keys, hashmap `.entries`
sources, counter re-init on re-execution, and the facade-safety
negatives.

### Free-fn locus rebinding no longer reclaims live memory

**Critical regression, shipped in v0.16.0** (the GH #402
temporary-reclaim pass) and reported by a downstream handoff with a
full discriminator matrix. A free fn that rebinds a locus-typed
`let mut` binding — `let mut a = make(n); if c { a = make(n); }
return a;` — reclaimed the value while the binding (and then the
caller) still held it. Returned: the caller received a locus whose
`capacity` heap buffer was gone (params intact, every `get` failing —
silently wrong answers). Not returned: double-free, SIGTRAP at frame
exit. The trigger was **static**: a rebinding inside `if false { }`
was enough, because the temporary's dissolve slot was alloca'd in the
entry block but stored only where the expression ran, so a bypassed
store left stack garbage for the exit flush to dissolve. The
downstream shape was a neural forward pass — training converged
byte-identically and every prediction then read zero.

Three rules close it:

- An `=` into a locus-typed slot decides the RHS factory call's
  owner (the slot), the same way a `let` RHS and a `return`
  expression already did — no frame-temporary registration.
- A binding on either side of a bare-local `=` (`a = nx;`) is
  disqualified from frame-scoped reclamation: a moved value has two
  names, and a dissolve through either can fire on a value the other
  still holds. Conservative by construction — the old leak, never a
  double-free.
- Deferred-dissolve slots are NULL-initialized at entry, so any path
  that bypasses the initializing store (a branch not taken, an early
  `fail`, a zero-iteration loop) reads the flush's
  "instantiation-bypassed" sentinel instead of stack garbage. This
  also closes a second latent shape the fix exposed: an early `fail`
  before a loop whose body `let`-binds factory results dissolved the
  loop bindings' uninitialized allocas (a guard `fail` on an empty
  model, in the downstream reproducer).

### Five surface fixes from a downstream compiler-bugs pass

The same handoff's non-regression items, each unchanged since at
least v0.13.0:

- **An implicit block-tail return lowers.** `fn double(n: Int) -> Int
  { let d = n * 2; d }` typechecked and then failed codegen with
  "falls through without returning a value" — the check/build
  divergence class that lets a checked-green library fail to ship.
  The tail of a fn-shaped body (free fn, locus method, mode) with a
  declared return type now lowers as a real `return`.
- **`hale check` resolves sibling-file declarations in a seed.** A
  `topic` declared in one file and subscribed from a sibling file of
  the same seed built and ran, but `check` reported "unknown topic":
  the no-import multi-file path kept one program per file, and the
  sync-inference pre-pass resolved each file alone. A multi-file seed
  now merges before checking, exactly like the import-bearing path.
- **`err` is bound in `or fail E { … }` payloads.** Inline error
  translation — `src() or fail DstError { kind: err.kind }` — works
  without a per-edge `wrap_x(err)` helper; the payload sees the same
  `err` binding a substitute RHS gets.
- **`-> ()` is a no-op unit annotation everywhere.** A non-fallible
  locus method with an explicit unit return type hit "tuple type must
  have at least 2 elements; got 0" while the fallible spelling was
  accepted. The empty-tuple return annotation is now normalized to
  "no return type" once, on the merged AST, for every fn-shaped
  declaration.
- **`or <substitute>` coerces LocusRef→Interface for `@form`
  methods.** `list.get(i) or NoopTool { }` on a `heap items of Tool`
  vec was rejected with a substitute type mismatch while the same
  substitute on a plain fallible call compiled; the substitute now
  gets the same fat-pointer coercion as argument-position and
  let-ascription sites.

### An element chain over an unsupported source names itself

A chain (`.filter(…).count()` and the like) desugars to a loop that
fetches each element through the source's `get`. A fixed array `[T; N]`
or a `bounded[T; N]` has no such accessor, so the chain failed with a
bare `no field \`get\` on \`[Int; 4]\`` — no mention of chains, and no
pointer to the source forms a chain does support. `hale check` now
adds, for exactly that shape, that the form is not a supported chain
source yet and that chains anchor on a `@form(vec)` directly or a
`@form(hashmap)` via `.entries`. Ordinary field typos keep their
did-you-mean hint. (Supporting arrays and `bounded` as chain sources,
and a `Decimal`/`Float` `sum`, is a larger change — chains desugar
before type-checking, so the source form and element type aren't yet
known where the accessor and accumulator are chosen.)

### An intra-locus publish no longer accrues its payload per delivery

The fifth item from the downstream handoff (the four below shipped
first). A `Topic <- payload` for a topic used only within a locus tree,
with no transport binding, is rewritten by `desugar_intra_locus_topics`
into a direct call to the subscriber's bus handler — sidestepping the
bus queue for speed, and with it the queue's per-delivery cell reclaim.
The payload then built as an ordinary call argument in the publisher's
method-scratch subregion and was not freed until `run()` returned, so a
long-running publish loop accrued one payload per delivery (~31 B/cell
on a string payload; unbounded over a daemon's lifetime).

Codegen now confines each such payload to its own arena subregion,
destroyed once the synchronous handler returns — the same lifetime the
bus queue's cell reclaim gives a delivered payload, and safe for the
same reason (the handler has returned, and Hale copies on every field
store, so nothing it kept still points into the freed region). A bus
subscription handler is not callable from Hale source, so a
statement-position call to one is unambiguously this desugar; the
reclaim applies to nothing else. Measured: the publish-payload accrual
goes to zero (a residual reflects only transient `let`s the user's own
code leaves in scratch, not the delivery). String and Int payloads
verified intact across the reclaim boundary under ASan.

### Four fixes from a downstream handoff

A substrate friction report surfaced four defects, fixed here as one
batch. (A fifth, a per-delivery memory accrual on same-thread bus
dispatch, is investigated but not fixed — see the PR for the root cause;
its reported mechanism did not match the runtime.)

- **A factory-built `mode` projection is no longer reclaimed before the
  caller reads it.** A `mode bulk() -> M { let out = make(...); return
  out; }` — where `make` is a free-fn factory — handed the caller a
  reclaimed locus: the factory result stayed in the mode's method-scratch
  subregion, which the epilogue destroys at return, so reads came back
  empty (or showed another projection's recycled storage). A regression
  shipped in v0.14; a literal-built projection and a free fn returning a
  factory result were the controls that still worked. A returned binding
  in a mode body now routes its allocation through the caller arena, as a
  free fn already does. Modes are the one member surface that legitimately
  returns a locus, so the hole sat on the projection contract's core.

- **Element chains work over `@form(hashmap)`, anchored on `.entries`.**
  `m.entries.filter(...).count()` (and the rest of the chain vocabulary)
  now compiles to the fused loop over a hashmap's occupied-slot cursor.
  Chains previously anchored only on `@form(vec)`, so a hashmap source
  lowered to a key/index type mismatch. `.items` is accepted as the
  explicit vec anchor; a bare vec source is unchanged.

- **`hale check` rejects a namespace-lotus method spelling on an imported
  locus**, instead of accepting it and dying in codegen. `mat::Grid {
  }.method(x)` where `method` is a free fn (not a method on `Grid`) passed
  `check` because an imported qualified-path literal typed as `Ty::Unknown`
  — hiding field and method access behind it. Imported literals now
  resolve to their merged type, so the missing method is a typecheck error
  in the author's spelling. A real imported method still resolves, and
  imported literal fields are not newly validated (no regression on
  existing multi-seed code).

- **`std::secret::Signer` takes a `decode` transform.** `Signer { env_var:
  …, decode: "base64" }` decodes the source before keying, so a venue that
  issues a base64 secret gets the DECODED key rather than one keyed under
  the text of the base64 (which produced valid-looking MACs under the
  wrong key, silently). An unavailable source or an unrecognized transform
  fails closed — `ready() == false` — never a key that isn't the one the
  source names.

### A target model, and the first step toward Windows (GH #445)

Targets were `Native | Wasm32`, where "native" meant "whatever host
compiled the compiler", and every platform question — which system
libraries to link, whether `-Wl,--wrap` exists, what an executable is
called — was answered by asking Rust's `cfg!(target_os = ...)` about the
HOST. On a Linux box building for Linux those coincide, so nothing looked
wrong. They stop coinciding the moment a second target exists, and then
the conflation is not a refactor away from a bug, it is the bug.

`TargetSpec` is now the model: a canonical triple with a parsed arch, OS
and ABI, owning its own file conventions and linker facts. `--target`
accepts canonical triples as well as the `native`/`wasm32` aliases, and
`hale --list-targets` prints every target with its support tier.

- Eleven host `cfg!(target_os = "macos")` decisions in codegen now ask the
  target. Ten were linker and runtime-cflag choices; the eleventh gated
  emitted code — a macOS-hosted build of a Linux artifact would have
  silently dropped the `async_io` enable call.
- `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc` parse, describe
  themselves, and are refused at argument-parsing time with an error that
  names the issue, rather than failing later inside the linker.
- `TargetCpu::Baseline` is now `TargetCpu::X86_64V3`. It pins AVX2/BMI2/FMA,
  which is not a "baseline" anywhere but x86-64. The `--target-cpu baseline`
  spelling still works, and `x86-64-v3` is accepted too.

No language, syntax, or behavior change on any existing target: Linux,
macOS and wasm32 build exactly as before.


### Secrets: confine, classify, claim (GH #436)

**`@sealed locus L`** — a locus's `params` become readable only from
inside its own methods. Others may still CALL it; they may not read
its state. This closes the gap the whole secrets story rested on:
loci are otherwise **not** field-encapsulated, so `self.child.key`
typechecks from anywhere holding the locus, and "the key never leaves
its owner" was a property you checked rather than one that was true.

Opt-in, one word, breaks no existing program — the `@supervised`
shape. (No claim form requires an annotation yet, so a constitution
cannot demand `@sealed` across a group; that is a follow-up, and the
same gap applies to `@supervised`.)
Only `params` are confined; capacity slots and methods are untouched,
because sealing confines state rather than making a locus uncallable.
Param **initialization** is deliberately not restricted: a parent
writing `Signer { key: … }` already holds the value it passes, so
sealing the initializer would cost ordinary configuration and buy
nothing. Load real secret material inside `birth`.

The guarantee then needs no new analysis — confine with `@sealed`,
classify the one privileged method with a user effect class, and state
the law with claim forms that already exist:

```hale
effect secret_use;

@sealed locus Signer {
    params { key: Bytes; }
    @effects(is: { secret_use })
    fn sign(m: Bytes) -> Signature { … }
}

claims {
    no_plugin_secrets: forbid reaches(plugins, effects(secret_use));
    one_op_per_request: bound secret_use <= 1 on paths from handlers;
}
```

Stated exactly: *the secret lives in a locus that owns it, the domain
cannot obtain it, the only operations on it are classified, and the
domain's claims constrain who may reach them and how often.* That is
confinement, **not** information flow — a value derived from the key
is not tracked, a constant-time compare still lets the verdict be
published, and the sealed locus's own body is trusted.

**BEHAVIOR: `@secret` is now a lint (warning), not an error.** It was
reported as a certificate while being a local identifier walker over a
*fragment* of one fn body — it walked `then` branches but not `else`,
and had no notion of aliasing, so moving a publish across a branch or
renaming through one `let` made the finding disappear rather than
surface as uncertified. Both of these checked clean:

```hale
fn leak(@secret token: String, flag: Bool) {
    if flag { print("nothing"); } else { Out <- Msg { v: token }; }
}
fn leak2(@secret token: String) {
    let alias = token;
    Out <- Msg { v: alias };
}
```

The true positives it *can* see are still reported, as warnings. The
default traversal is deliberately left narrow: widening a lint in
place newly fails programs that compile today, which is a userspace
break even when every new finding is a real bug.

**New: `hale check --strict-secret`** runs the widened, fail-closed
walk — every branch including `else` / `else if` / `match`, alias
propagation through `let`, and `uncertified` for anything it cannot
follow (an unfollowed call, a field store, a return). Opt-in because
it is loud, which is the honest signal that one body's reasoning is
not a containment proof.

Worked end to end in
`crates/hale-codegen/tests/fixtures/examples/secrets-sealed-handler.hl`.
Spec: `spec/verification.md` § Secrets.

### `std::secret` — key material an application never holds (GH #436)

`Signer` (`sign` / `sign_512` / `verify` / `ready`) and `Credential`
(`matches` / `ready` / `fingerprint`), both `@sealed`, both taking the
**name of a source** rather than the bytes:

```hale
locus Gateway {
    params {
        s: std::secret::Signer =
            std::secret::Signer { env_var: "SIGNING_KEY" };
    }
    fn go(m: Bytes) -> Bytes { return self.s.sign(m); }
}
```

`self.s.key` from `Gateway` is a compile error naming the methods to
call instead. The key is read during `birth`, so it exists only inside
a sealed locus from the moment it enters the program.

`birth` is the **only** writer, including the no-source case. Params
are constructible by whoever holds the locus, so without that a caller
could write `Signer { key: b"…" }` and hold the material after all —
defeating the one thing the module exists to prevent. Passing `key:`
now yields an empty key and `ready() == false`, which surfaces at
startup. `Credential.fingerprint()` (first 8 bytes of SHA-256, hex) is
the publishable handle: safe to log, correlates across processes,
carries nothing.

Every privileged operation carries the `secret_use` effect class,
declared by the module and travelling with the import.

**One narrow resolver change made this possible.** A qualified path
naming a **sealed** Hale-source stdlib locus now resolves to the
mangled name that source declares, instead of `Ty::Unknown`. `@sealed`
keys off the receiver's resolved type, so without it a sealed stdlib
locus had readable params — verified before the fix: `self.signer.key`
returned real key bytes and `hale check` passed. Only *sealed* stdlib
loci are injected; every other qualified path still resolves to
`Ty::Unknown` exactly as before, because resolving them all would
switch on field-existence and method arity/fallibility checking across
the whole stdlib surface at once — a genuine improvement, and one that
can newly reject programs that compile today, so it wants its own
change and its own measurement.

Also fixed: `rename_targets_exist` matched a bare `locus ` prefix, so
every *annotated* stdlib declaration was invisible to it and a rename
row pointing at one read as stale. It now skips leading decorators —
the gap would equally have hidden a `@form` locus.

### `require sealed(all G)` — confinement as law (GH #436)

A new claim form: every locus in `G` is declared `@sealed`.

```hale
group vaults = { Signer, TokenStore };

claims { vault_confined: require sealed(all vaults); }
```

```text
claim `vault_confined` violated: a locus in `vaults` is not `@sealed`,
  so its state is readable by anything holding it — TokenStore
```

A **universal** over the group's members, which is why the quantifier
is `all` rather than the `some` the other `require` forms take: those
ask whether an endpoint exists anywhere in the group, this asks whether
every member holds. `require sealed(some G)` is a parse error rather
than a form that quietly means the opposite of how it reads. Every
unsealed member is reported in one diagnostic — a baseline is adopted
once and the reader wants the whole list, not one name per build.

Without it, sealing is per-locus discipline, and one unsealed member of
a vault group is the whole hole. It composes through constitutions like
any other claim, so a security baseline is adopted once:

```hale
constitution SecretBaseline {
    vault_confined: require sealed(all vaults);
    no_plugin_secrets: forbid reaches(plugins, effects(secret_use));
}
```

This closes a gap found while writing #437's spec: an early draft
claimed a constitution could already require sealing across a group,
which was false — no claim form required an annotation. The same gap
still applies to `@supervised`.

### Confinement and claim fail-opens closed (GH #436 review)

External review of the landed work found three release blockers. All
reproduced; the negative controls in `secrets_fail_opens.rs` were
written and verified failing first.

**BREAKING: every built-in effect name is now reserved.** `effect
secret_use;` is an error — and so are `effect syscall;`, `effect
block;`, `effect entropy;` and the rest. Those declarations were
always silent no-ops (`EffectClass::from_ident` wins at every use
site), so a program that wrote one believed it had declared a class
while every claim naming it quietly meant the built-in. The break is
narrow in practice and broader than "`secret_use` is new", which is
why it is stated as the general rule.

`secret_use` itself is now a compiler built-in. User effect classes intern per-`Program`, so
the stdlib-declared class had no identity an application's claims
could name: `forbid reaches(plugins, effects(secret_use))` silently
missed `std::secret::Signer.sign` — the law over the recommended
secrets path was unenforceable — and with an application declaring its
own classes first, the stdlib's bit aliased onto whichever class sat
at that index. A built-in has one identity by construction. `bound`
accepts it, being the one counted built-in with no `@budget`
spelling.

**`@sealed` now stops writes.** It hooked only the expression
field-access path, so `self.vault.key = 999` typechecked — for
`std::secret`, outside code could CHOOSE the signing key.

**`std::secret` fails closed.** `ready()` reported false and nothing
consulted it, so `sign` returned a valid HMAC under the empty key and
`verify` accepted the matching forgery. Precisely: when the source is
unavailable, signing returns an **empty result** and verification
rejects — refusal by sentinel, not a startup failure. Gate startup on
`ready()`; a structural birth failure may be better later and is not
needed for the security property. Sharpest case:
`matches(b"")` against an unloaded credential returned **true** — an
authentication bypass on any unconfigured deployment.
`fingerprint` is no longer described as "safe to publish" and now
carries `secret_use`.

`require attributed` now attaches to the first **application-owned**
fn crossing out, including `@ffi` declarations and Hale-source stdlib
bodies. It previously ignored every resolved call, so a publish
through `self.logger.info(m)` was invisible while the same operation
as a path call was caught — coverage depending on how an API happens
to be implemented. An ordinary application callee is still judged on
its own row, which is what keeps attribution direct. `publish` also
parses as a class name now; it is a built-in class and a reserved
keyword, and `expect_ident` had rejected the one class most worth
attributing.

Also: `require attributed` accepted `ffi` / `spawn` / `recursion`,
which its evaluator answered with unconditional success; it ignored
direct allocation sites; the sealed/attributed collectors did not
recurse into modules; `require sealed` held vacuously over a
locus-free group; `--strict-secret` missed tuples, indexes, block
tails, `or` substitutes, `fail` payloads, expression `match` arms and
`LetTuple` (the expression walk is now exhaustive by construction —
no catch-all arm, so a new `Expr` variant fails the build); and
diagnostics rendered mangled stdlib names.

**Schema 1.9:** `sealed` joins the hashed model, so sealing a locus
moves `shape_hash`. `require sealed` replays from the artifact;
`require attributed` is compiler-certified, since the artifact exports
inferred rather than direct effect sites.

### Effect-system completeness (GH #436 review 2)

Adding a compiler-owned class had two integration seams the first
test matrix did not cover: the **closed universes**.

`@effects(only: {…})` is the complement of a hardcoded class list and
`@phase_effects` iterates another, so a class absent from either is
one those contracts can never forbid. **`only: {}` certified a fn
reaching `secret_use`**, and `@phase_effects(run: {})` admitted it
during run — contracts whose entire purpose is rejecting unlisted
effects were weaker than they read. Both universes now include it.

Three attribution holes closed:

* An **`@ffi` declaration** is now the direct boundary site rather
  than its caller. The caller-side branch was unreachable (an `@ffi`
  fn is a bundle decl, so the bundle test short-circuited first) and
  the wrong shape besides — it returned true for every mask, so one
  foreign call would have read as `publish`, `entropy` and
  `secret_use` at once.
* A fn's **own `@effects(is: {C})`** counts as a direct site. Without
  it, `@effects(is: { secret_use }) fn sign(…)` — the shape the whole
  secrets architecture rests on — was invisible, because it calls
  nothing classified and allocates nothing.
* An **indirect or opaque call** now leaves the claim `uncertified`
  unless the caller already names a purpose. Attribution had asked
  only whether a textual name sat in the stdlib registry, so a
  callback parameter contributed nothing and the law reported `holds`
  over a boundary it could not see.

`--strict-secret` also missed **block tails** (`fn f(@secret t) { t }`
— an implicit return; `block_mentions` promised the tail in its doc
comment and checked only statements) and `fail` / `violate` payloads
and `ShmWrite` bodies.

### `@sealed` and `contract { expose … }` are mutually exclusive (GH #436)

Two contradictory claims about the same boundary, and sealing wins. The
contract consistency check passes — a matching `consume` binds — and
then every use of the exposed field is rejected, leaving a construct
that reads as a permission and grants nothing. The pair is now a check
error at the declaration.

```text
locus `Greeter` is `@sealed`, so `expose greeting` cannot grant anything:
sealing denies every read from outside the locus, including one a
coordinator `consume`s.
```

`expose` cannot serve as the sealed allowlist without redefining it:
it is the coordinator/coordinatee surface, so honouring it would grant
reads to an `accept`ing parent while still denying them to a parent
holding the same child as a param — one field, public to one kind of
holder and not the other.

The `consume` side needs no check of its own: a sealed locus cannot
declare an `expose`, so a coordinator consuming from one lands in the
existing "does not expose it" arm.

### `hale check --sealable` — the adoption survey (GH #436)

`@sealed` is opt-in, so "would this collide with real code?" is a
question about an existing codebase that nobody can answer by reading.
It is mechanically computable:

```text
sealability: 4 of 5 loci can be `@sealed` today

  free to seal (nothing outside reads their params):
    Already
    App
    Holder
    Private

  would break callers:
    Exposed — 1 external read(s): Exposed.k
```

The survey **runs the real check** against an all-sealed clone of the
bundle rather than reimplementing the rule. A hand-written walk would
drift from `check_sealed_read` the first time either changed, and a
survey that disagrees with the checker is worse than none — it would
report a locus as free to seal when it is not.

Measured over the in-tree corpus: **148 of 151 loci across 94 programs
could be sealed with no changes.** The three that cannot are the same
shape — a parent reading a child's result field directly rather than
calling a method — which the no-locus-return rule already discourages.

### `require attributed(all C)` — every boundary crossing names a purpose (GH #436)

```hale
effect audit;

claims { io_attributed: require attributed(all syscall); }
```

```text
claim `io_attributed` violated: a fn performs `syscall` with no declared
  purpose — classify it (`@effects(is: {...})`) with a user effect class
  so the operation is attributable: Rogue::sneak
```

**Orthogonal to interposition, not a weaker form of it.** `forbid
reaches(app, effects(syscall)) avoiding gate` — which already worked —
constrains WHERE a boundary is crossed and says nothing about what any
crossing is FOR. This constrains attribution and says nothing about
location. Neither implies the other: all I/O can funnel through one
`write(path, bytes)` that everyone calls for everything (interposed,
unattributed), or forty loci can each touch the OS while every one
names its purpose (attributed, un-interposed). A hybrid wants both.

It also closes a coverage hole `avoiding` necessarily has: that claim
is scoped to a group, so a locus outside it is unconstrained and one
written next month is uncovered until someone edits the group. This is
a universal over the whole closed world.

**DIRECT, not transitive**, and that is load-bearing: transitively,
every caller downstream of one attributed fn would inherit the label
and pass, making the claim nearly vacuous. The attribution point is
the site where the boundary is crossed. A built-in in `is:` does not
count — it restates what the compiler already infers. The class must
be a built-in; a user class there would be trivially true while
reading like a contract.

---

## v0.16.0 — the law composes, the certificate travels (2026-08-06)

### Signed fleet components & binary attestation (GH #408 Phase 7)

A composition proves a world against artifacts it can read; now a
signature proves those artifacts are the ones a key-holder meant.
`hale fleet keygen | sign` produce ES256 keypairs and detached
sidecars (`<artifact>.sig`, `es256:<hex r‖s>`) over the artifact's
**exact bytes** — sound because artifacts are byte-reproducible
(schema 1.8), and necessary because the in-band `artifact_digest`
is FNV-1a, a tripwire rather than a trust anchor. ES256 because the
system already speaks it: it is `std::crypto`'s suite, so a Hale
program — a supervisor, a deploy gate — verifies the same sidecar
with the language's own stdlib. One algorithm end to end.

Trust is **strict when declared**: `--trust <pub.pem>` on
`check`/`dump`, or `[fleet_trust] keys = [...]` in the manifest,
makes an unsigned or unverifiable component a composition error.
There is no `require = true` knob, for the reason `no_base` exists:
a trust set that quietly admits unsigned artifacts is law that
looks bound and binds nothing. Verification runs before the
integrity digest — provenance before integrity before meaning, each
check covering the bytes the next one reads. The fleet artifact
records the admission as unhashed provenance: `sha256` of the
admitted bytes, `signed_by` the verifying key's identity or `null`
— a fact, not an omission, so "unsigned admission" and "verified
under this key" stay distinguishable downstream.

`hale fleet attest <plan>` answers the remaining question — are the
executables the plan deploys the ones the operator hashed? Plan
schema 1.1 (1.0 still reads) adds optional `binary` /
`binary_sha256` rows per instance, and attestation is
all-or-nothing over them: a missing row is a refusal, not a skip,
because a partial attestation would report coverage it does not
have. Attest checks bytes at rest; whether a *running* process is
still that binary is runtime observation territory (7b), and
nothing here claims otherwise.

The honest boundary, stated where it can be quoted: signing
certifies provenance and integrity, never behavior. Out of scope by
design: compromised builders, malicious compilers, runtime memory
tampering.

### The application checker moves onto the shared engine (GH #408)

`forbid reaches` had its own breadth-first walk, written before the
fleet tier existed. Both are now `model_graph::search`, which owns the
queue, the visited set, the parent tree, root seeding, masking and the
step ceiling — the bookkeeping that is identical everywhere and easy
to get subtly wrong. What stays with each tier is what genuinely
differs: what a vertex is, which edges exist, what counts as the
target, and the diagnostic for an edge the walk cannot follow.

Behavior is unchanged, and that is checked rather than asserted:
`hale check` output and topology artifacts — claim verdicts included —
are byte-identical across all 88 corpus programs and three real
downstream applications, before and after.

The two tiers keep their opposite policies on an unfollowable edge,
because both are deliberate. The application checker stops at it and
names the edge, since the repair is to make that edge resolvable. The
fleet tier walks past and refuses only if no path is found at all,
since a concrete cross-binary counterexample is worth more than a
refusal. `search` expresses both.

Hole propagation moved INTO the engine: a caller reports that a
vertex's edge set is incomplete and picks a policy, and the engine
decides the verdict. Forgetting to consult a hole is no longer a
mistake the API allows. Two fail-opens in the fleet wrapper closed
with it — source/target overlap answered `None` instead of a
zero-length path, and a tripped step ceiling answered `None` instead
of refusing. Search *exhaustion* can prove an absence; search
*abandonment* never can.

Scope, stated precisely: this centralizes unweighted TRANSITIVE
reachability, which is what `forbid reaches` asks at both tiers.
`only edges` stays direct crossing-edge enumeration — a cut-edge
subset query with no transitive walk — and `bound` keeps
`site_count`, a weighted traversal, because a quantitative semiring
is a different algorithm over the same edges rather than a duplicate
of this one. `no_prohibition_evaluator_defines_a_private_bfs` fails the build
if a third frontier appears, its companions check that both
evaluators still call the engine and that the engine holds exactly
one queue.

### One reachability engine, shared by both tiers (GH #408)

The fleet tier read the topology artifact and rebuilt a smaller graph
of its own, and the omissions were not random.

**It dropped `calls_via_stdlib`.** The artifact exports user→user
paths whose interior is stdlib code, contracted to their user
endpoints, and the application checker walks the union with `calls`.
Fleet composition read `calls` alone, so a component whose routed
handler reaches its routed publisher through `std::http::Router`
contributed no edges at all — and a prohibition spanning it reported
a false absence.

**It ignored uncertainty.** Component `unknowns` were copied into the
fleet artifact and never consulted, so an indirect call could remove
the only modeled path to a target and `forbid_reaches` answered
`holds` — an absence certified by not looking. Fleet claims can now
answer `uncertified`, which fails like `violated` but says the repair
is to resolve the unknown edge rather than to fix the program.

Both are now one engine (`hale_types::model_graph`) that takes edges
and holes and answers path / certified-absence / refusal, replacing
the private graph walk. Uncertainty stays **reachability-sensitive**:
only a hole the claim's source can actually walk to blocks
certification, so an unrelated unknown elsewhere in the deployment
does not poison every law.

One nuance the engine encodes: `uninhabited_interface_call` is
recorded as an unknown but is *not* fail-closed — in a closed world an
interface with no conformers has no values, so the site is dead.
Treating every unknown as a hole would make dead dispatch refuse to
certify anything downstream of it.

### Publish provenance no longer mixes coordinate systems (GH #408)

Schema 1.8 promises file-local spans. Calls, subscriptions and
declarations localized both endpoints; publishes localized only the
start and emitted the raw bundle-global end. A publish in any source
whose virtual base is nonzero therefore produced a row naming a file
with a span reaching past the end of it — a file-local start with a
bundle-global end, which is not a coordinate in any single system. A
consumer resolving it lands outside the file it was told to open.

Concretely, a publishing locus in an imported library serialized as
`source 1, span [182, 328]` where that file is 319 bytes, while a
subscription in the *same file* was correct.

`shape_hash` is unaffected — provenance is excluded from it by design
— and an application whose publishes all sit in its first source is
byte-identical, which is why the cross-artifact conformance loop never
surfaced this. The locality test now checks calls, publishes,
subscribes and declarations rather than declarations alone, and
asserts both endpoints.

### Fleet claims: one verb per claim, real endpoint roles, and no vacuous prohibitions (GH #408)

Three fail-opens, found by an external review of the effects / claims
/ constitutions / fleet stack and confirmed by reproduction.

**A claim naming several verbs judged only one of them.** Every verb
on a claim row is an `Option`, and the evaluator is an `if/else`
chain emitting one row, so a claim pairing a holding
`require_subscribes` with an impossible `count ... eq: 999` passed —
and the artifact recorded the whole claim's name as holding. A claim
is one sentence, as it is in source: naming more than one verb is now
refused, with the fix being to split it so each half has its own name
and verdict.

**A route endpoint did not have to hold the role the plan gave it.**
Admission checked only that the instance *declared* the topic — which
any component importing the topic module does, published or not. A
route could therefore name a phantom producer, and a law about the
consumer side would hold with nothing feeding it. The artifact is now
the authority: a publisher endpoint must publish, a subscriber
endpoint must subscribe, and a plan that misdescribes its components
is refused before any claim is evaluated.

**Overlapping prohibitions held vacuously.** `forbid_reaches(g, g)`
reported `holds` while every path in the fleet ran inside the
prohibition, because the search refuses a destination that is also a
source. An instance in both groups is now a zero-length violation,
matching the application tier. Likewise `avoiding` may no longer name
an endpoint of its own claim — masking one deletes the domain being
quantified over, which makes any prohibition hold.

### A cardinality claim must name a bound (GH #408)

`count_publisher_instances` / `count_subscriber_instances` with no
`eq`, `min` or `max` compared nothing: every bound defaults to true
when absent, so the claim held against any fleet whatsoever while
reading like real law in review. A claim naming no *verb* was already
refused; a verb naming no *bound* is the same emptiness one level
down and now gets the same answer.

### A multi-hop fleet witness names the route of each hop (GH #408)

Route labels in a witness were shifted the wrong way — each node
carried its OUTGOING route rather than the one it was entered by. On
a two-node witness the fallback happened to produce the right answer,
which is why every existing witness test passed; a three-node witness
labelled *both* hops with the last route, sending a reader to the
wrong route entry and, in a real deployment, the wrong config file.

### A fallible method no longer frees the loci it is about to dissolve

A free-fn factory allocates its locus out of the caller's published
arena, and a method publishes its own per-call scratch subregion. So a
factory result let-bound inside a method frame has its struct living
in that scratch — and the binding owns it, so the frame's exit
dissolves it by loading its `__arena` field and destroying that arena.

The **fallible** method epilogue destroyed the scratch *before*
running those dissolves, so the dissolve read a locus struct out of
freed — and, via the subregion freelist, possibly recycled — memory
and handed whatever it found to `lotus_arena_destroy`. Every other
epilogue already flushed first; this one alone had the two reversed.

The bug needed all three of: a locus method, declared `fallible`, that
let-binds a factory result. A downstream handoff hit exactly that
shape — a trainer whose `fit(...) -> () fallible(E)` preallocated two
row buffers — and segfaulted at method exit *after* computing entirely
correct results, which is the worst way for this to present. It is a
latent use-after-free rather than a reliable crash: at small sizes the
freed bytes still hold the old contents and the program exits 0, so
the regression test asserts on the emitted order rather than waiting
for a segfault.

### Three levels of locus nesting compile again

A function's prologue — entry-block allocas, deferred-dissolve slots,
the method scratch arena, the caller-arena snapshot — is emitted
before the body's first statement establishes a debug location, so it
inherited whatever location the *previously emitted* function left
live. LLVM rejects that module outright:

```
!dbg attachment points at wrong subprogram for function
```

Debug info is always on, so this was a hard build failure with no
workaround but restructuring the program. It reproduced on three
levels of locus nesting where the middle one calls a free-fn factory
(`Demo.go` → `Trainer.fit` → `Model.train_step`); two levels survived
only because nothing had left a location live across the boundary.

The reset existed on the free-fn path and on none of the twelve other
function-body entry points. It is now one helper called at every
entry, and `di_entry_reset_is_universal` fails the build if a new
entry point forgets it — the gap was invisible for months precisely
because nothing enforced it.

### `require_subscribes` needs a route, not just an endpoint (GH #408)

Found by running the fleet checker against a real downstream
deployment slice rather than a synthetic fixture.

`require_subscribes` / `require_publishes` checked only that some
instance in the group *exposes* the endpoint. So a plan where the
ledger subscribes `exec.fill` and nothing in the plan publishes it
reported `holds` — and the law "fills must reach the ledger" could not
catch a missing route, which is the one thing it exists for.

Both halves are now required: the endpoint exists **and** a route in
the plan carries the subject to (or from) it. The diagnostic
distinguishes the two failures, because "nobody subscribes" and
"somebody subscribes and nothing connects them" have different fixes.

A synthetic fixture hides this, because whoever writes one routes
everything they assert. It surfaced only because the real slice's
publisher sat outside the selected instances — and adding that
instance makes the claim hold again, which is the round trip that
confirms the check is not simply always-failing.
### `[fleets]`: check every declared deployment (GH #408 Phase 5)

```toml
[fleets]
production = "ops/fleet/prod.plan.json"
staging    = "ops/fleet/staging.plan.json"
```

`hale fleet check` with no plan now checks every deployment the
workspace declares. A repository usually has more than one, and
checking whichever one somebody remembered to name is the same
partial-coverage problem `--matrix` solves for entrypoints. Every
fleet runs even when an earlier one fails; the exit status is the
worst of them, so a missing plan is not masked by an ordinary claim
failure elsewhere. A workspace declaring no `[fleets]` is a usage
error rather than a vacuous success.

`[fleets]` and `[environments]` are **separate axes**. An environment
binds law to an entrypoint at the application tier; a fleet is an
arrangement of deployed instances. A workspace may declare both, and
`production` in one need not mean `production` in the other —
collapsing them would force every entrypoint's law to be a function of
some deployment it may not even appear in.

There is deliberately no coverage check over plans: unlike
entrypoints, which are discoverable seeds, a plan is an arbitrary file
path, so "every plan in the repository is declared" cannot be asked
without guessing.

### Fleet claims over the composed model (GH #408 Phase 2)

`forbid_reaches` (with `avoiding`), `only_edges`, `require_subscribes`
/ `require_publishes`, and instance cardinality — evaluated over the
fleet model composed in Phase 1, carried in the plan as normalized
rows rather than source grammar.

The flagship result is an interposition claim catching a route that
skips its mediator, with a witness that crosses artifacts:

```
fleet claim `orders_pass_oms` violated — witness:
  prober-0::Probe::submit  [rogue/main.hl]
  -(route `bypass`)->
  gw-0::Gateway::on_order  [gw/main.hl]
```

Both components are individually legal; only the deployment is wrong.
The witness names the instance-qualified vertices, the route carrying
the hop (a route, because there is no cross-process call to name), and
the source file each vertex lives in — the last of which is exactly
what Phase 0's source maps were built for.

Fleet cardinality counts instance-qualified **endpoints**, a different
sort from the application tier's declaration count, so it gets a
different spelling: a second deployed publisher fails
`count_publisher_instances` while each component still checks clean on
its own.

Groups quantify over instances by id or label, and an unknown or
empty group is an error rather than a vacuously satisfied claim.

### `hale fleet`: compose artifacts across applications (GH #408 Phase 1)

```sh
hale fleet check prod.plan.json    # compose and validate
hale fleet dump  prod.plan.json    # write the fleet artifact
```

A fleet is a named deployed system of application **instances**, and
it composes *artifacts* — never source. A source-merged "super-main"
would be unsound in both directions: an unbound topic is in-process by
default, so merging two binaries makes matching publishers and
subscribers look connected when no route joins them; deploy-time
routes existing only in config would not appear; and calls, which
cannot cross a process boundary, would become ordinary reachability.

So **matching wire identities establish compatibility; only an
explicit route creates a fleet edge.** Two instances that both declare
a topic and are not routed stay unconnected, which has its own test.

Validation refuses anything a certificate cannot rest on: integrity
first (the whole-body digest, since `shape_hash` covers the model half
and cannot vouch for the `topics` rows the join reads), then the
`semantics` version, then the component's own verdict — local law is a
precondition of admission. Routes join on `(subject, payload_hash)`,
never the local topic name, so a shared name over a different payload
is a plan that cannot be formed rather than a silent mismatch.

`fleet_shape_hash` covers instance identities and cardinalities,
routes and their wire identities, component shape hashes and the
composed relations — and excludes provenance. Verified in both
directions: a comment added to a component leaves it unchanged, while
a changed transport or an added instance moves it.

### Artifact schema 1.8: source maps and model semantics (GH #408 Phase 0)

The prerequisite for composing artifacts across separately compiled
applications, and the reason #408's Phase 0 is where the risk is.

**Spans resolve to files.** They were bundle-global byte offsets — an
artifact of concatenating a seed's files, meaningful only inside the
process that produced them. `[1204, 1231]` cannot be turned into a
location by anyone else, so no cross-artifact witness could say where
to look, which is most of what a witness is for. Provenance rows now
carry `"source": <id>` alongside a file-local span, resolved through a
new `sources` table.

**Artifacts are reproducible.** Source paths are relative to the
workspace — the nearest ancestor holding a `hale.toml`, else the
deepest common ancestor of every source — and are canonicalized
before being made relative. Both were needed: an absolute path makes
the artifact machine-specific, and rooting at the target alone left
imported seeds absolute, since they usually live outside it. The same
sources now produce a byte-identical artifact from any working
directory.

Each source carries a content digest, so a consumer can tell whether
two artifacts came from the same text — and catch a stale artifact
paired with edited source — without the source being shipped. A span
the map cannot place reports `"source": -1` rather than being
attributed to the nearest file.

**`semantics`**, a version distinct from `schema`. Schema says a row
has these fields; it cannot say what they mean. Two compilers
agreeing on the schema and disagreeing on the semantics would compose
artifacts into a model neither would certify, with nothing in the
document revealing it.

*Breaking:* `provenance.decls` rows change from `[start, end]` to
`{"source": id, "span": [start, end]}`, and every other provenance row
gains a `source` field.

### Constitution edge cases: silent-ignore combinations and manifest provenance (GH #409)

An edge-case sweep before starting the fleet tier. Four gaps, all of
the same shape the two reviews found — a combination that behaves
plausibly instead of refusing.

- **`--matrix --dump-topology` emitted N concatenated artifacts** to
  one stdout — not valid JSON as a whole — and exited 0. A matrix is
  many evaluations, so there is no single artifact to emit; likewise
  `--matrix --check-topology` compared one entrypoint's model against
  another's baseline and reported a failure that meant nothing.
  `--workspace` already rejected these flags; `--matrix` did not.
- **`--workspace --env prod` silently ignored `--env`**, reporting
  "N seed(s) checked" with no environment law applied — a green run
  the user believes was gated. A workspace sweep includes libraries;
  an environment binds law to an entrypoint. Now rejected.
- **`--matrix --env` / `--matrix --workspace`** likewise selected
  nothing the matrix did not already enumerate. Rejected.
- **A manifest-required constitution had no provenance.** An injected
  adoption has no source line, so `unknown constitution \`Prod\``
  pointed at a main locus containing no `adopt` at all. The
  diagnostic now names the environment and the manifest.

Five behaviors that were already correct but unpinned now have tests:
one policy seed imported under two different aliases is one identity
(the review's required control — and the property that makes the
feature usable at all); a `claims` block outside `main locus` is a
parse error with `adopt` in it; a repeated `adopt` is idempotent
rather than a collision; an unknown `extends` base is an error; an
environment with an empty entrypoint list leaves the real entrypoints
unbound; and tampering with the artifact's `evaluation` section — the
constitution digests a consumer compares — fails the body hash.

### Constitution identity, corrected at the boundary (GH #409)

A second review found that the identity mechanism shipped in the
previous entry had two holes of its own. Both were reproduced before
fixing.

- **Pure-composition constitutions vanished from identity.**
  Identities were derived from the `source` of emitted claim rows, and
  a constitution contributing no clause of its own emits none. So
  `constitution Dev extends Left { }` — directly selected by the
  manifest — appeared in no artifact and in no comparison. Two
  entrypoints resolving the *same* `Dev` to different bases shared no
  comparison key and the matrix passed. Pure composition is not
  exotic: the corpus fixture added last release uses it. Identities
  now come from the adoption **traversal**, which already visits every
  constitution reached, and an evaluation reports its `roots` (named
  directly) as well as its `closure`.
- **`[claims] base` was compared per environment.** The key included
  the environment name, so two environments with disjoint entrypoint
  sets never shared one — a base resolving to different closures in
  dev and prod went undetected. The mechanism proved consistency
  *within* each environment and nothing about the base being shared.
  The base now compares workspace-wide.

Three smaller corrections:

- **A workspace declaring environments must decide about a base**,
  either `base = "…"` or `no_base = true`. An absent `[claims]`
  section is indistinguishable from a misspelled one, and the intended
  baseline would vanish while every environment still looked valid.
- **`--env` requires an entrypoint** whether or not the environment
  contributes a constitution. The check happened while injecting, so a
  `source_only` environment with no base injected nothing, checked
  nothing, and reported success for a library path.
- **Duplicate bases normalize.** `extends Core, Core` and `extends
  Core` evaluate identically, but the digest hashed the base twice and
  reported a false mismatch — so a value called a *normalized* closure
  was not normalized.

Artifact schema **1.7**: `evaluation` splits into `roots` and
`closure`, and gains the `environment` label. The prose promised this
section says which deployment a run certified; only the label can.

### Constitution review fixes: three fail-opens, identity, and the spec (GH #409)

An outside review of the constitutions PR found problems in shipped
code. Three were fail-opens — which matters especially here, since the
feature exists to stop law going missing quietly.

- **A duplicate clause inside one constitution was silently dropped.**
  Two clauses named `rule`, the second of which would have failed, and
  the build *passed*. Diamond duplication is resolved at constitution
  level, so a repeated `(origin, name)` can only be a duplicate
  declaration; it is now an error.
- **A misspelled manifest field silently removed all environment
  law.** `constituton = "Prod"` parsed, left `constitution` as `None`,
  and the entrypoint still counted as bound. `EnvSpec` now denies
  unknown fields, and an environment that adds no law must say
  `source_only = true` — an omission is indistinguishable from a typo.
- **A malformed seed vanished from matrix coverage.** A seed with a
  syntax error, listed in no environment, reported `ok: 1 pair(s)
  checked`, exit 0 — while the same seed made valid was correctly
  flagged. Breaking a file was a way out of the gate. Parse failure is
  now an unknown entrypoint, never a non-entrypoint.

**Constitution identity.** Names are flat and unmangled so diagnostics
cite them as written — right for display, useless for identity. Two
seeds can each declare `Core` with different clauses, so binding a
bare name proved only that each entrypoint had *some* `Core`. A
constitution's identity is now the digest of its normalized closure
(its own clauses, sorted, plus its bases' digests), and `--matrix`
rejects one environment resolving a name to two different claimsets.

**`[claims] base`** makes "an environment may add law, never drop it"
true of the mechanism rather than only of `extends`: every matrix
evaluation carries the base by construction, so an environment can
only add.

**Artifact schema 1.6** gains an `evaluation` section naming the
adopted constitutions and their closure digests. Per-claim `source`
says where a clause came from; it cannot say which deployment the run
certified, and two environments over one base can produce identical
claim rows.

**Documentation correction.** The previous docs said an entrypoint not
importing a seed "gets an empty group, with `may_be_empty` as the
opt-out". It does not: an undeclared group is an unknown-name error,
and `may_be_empty` applies only to a group that IS declared and
resolves to zero members. An entrypoint lacking a component writes
`group thing = { } may_be_empty;` explicitly — a line a reviewer can
see rather than an absence they must infer.

The formal grammar gains `constitution_decl` and the `adopt` form, and
`spec/verification.md` a normative account of placement, composition,
collisions, diamonds, cycles, identity, environments and matrix
completeness.

### `--env` and the entrypoint x environment matrix (GH #409)

The tooling half of constitutions. `adopt Core;` in source means
*always, everywhere* — but an entrypoint deployed to two environments
must satisfy both claimsets, and it cannot write two conflicting
`adopt` lines. So the environment binding lives in `hale.toml`, where
the deployment facts already are:

```toml
[environments.prod]
constitution = "Prod"
entrypoints  = ["apps/prober"]
```

```sh
hale check apps/prober --env prod    # adopts Prod for this run
hale check --matrix                  # every (entrypoint, environment) pair
```

`--env` injects the environment's constitution exactly as if the
source had written `adopt` — same evaluation, same closed world, union
with whatever the source already adopts. One entrypoint therefore gets
one verdict per environment without its source knowing where it will
be deployed.

`--matrix` checks every pair the manifest declares. **An entrypoint
listed in no environment is an error, not a skip** — that is the one
hole composition cannot close by construction, since no single
compilation can see that a sibling was left out. A seed with no `main
locus` is not an entrypoint and is not demanded of the manifest; a
listed path that does not exist, an unknown environment name, and a
`--matrix` run against a manifest with no `[environments]` are all
errors rather than quiet successes.

### Constitutions: one claimset across many entrypoints (GH #409)

`claims { }` is only legal inside `main locus`, and that rule is
load-bearing — claims are closed-world statements and `main` is the
only place a world is closed. But it constrains *evaluation*, not
*authoring*, so a law meant to hold for twenty entrypoints had to be
copy-pasted into twenty main loci, where the copy somebody forgets
fails open silently.

```hale
constitution Core {
    tenant_iso: forbid reaches(billing, research);
    one_writer: count publishers(topic Settled) == 1;
}

constitution Dev extends Core {
    no_real_payments: forbid reaches(app, payment_provider);
}

main locus App {
    params { … }
    claims { adopt Dev; local_rule: require …; }
}
```

Every clause is still evaluated in the adopting entrypoint's own
closed world. One text, N evaluations, N worlds — the soundness
argument is unchanged.

Composition is **union and nothing else**. A derived constitution may
add clauses and may not replace one: two constitutions declaring a
name is an error naming both origins, and so is a local clause
shadowing an adopted one. That looks like a restriction and is the
point — if override were allowed, weakening would be expressible and
would read exactly like ordinary composition at the adoption site,
and telling strengthening from weakening means proving one sentence
implies another, which fails open when it gets that wrong. A stricter
bound is simply a second named claim coexisting with the inherited
one. Weakening isn't rejected; it's unexpressible.

A diamond contributes its shared base exactly once, deduped by origin
rather than by name so a genuine two-origin collision still surfaces.
`extends` cycles, unknown names (with a did-you-mean) and `adopt` in
a library-tier block are all errors — adoption is the closing world's
act, since it fixes which world the clauses are evaluated against.

Artifact schema **1.5**: a claim row gains an optional `source`, the
constitution an adopted clause came from. Once environments may add
laws, product laws and environment rails (deliberately false in
production) live in one block and read identically — provenance makes
the distinction structural instead of a marker that can drift.

### `only edges` and `bound` say where to edit (downstream review)

`forbid` names the crossing call, or the publish and the receiving
subscription. The other two path claims anchored only at the claim
line, which for `only edges` defeats the point — it is explicitly a
*reviewable boundary inventory*, and making the reviewer hand-find
the crossing is the work it exists to save.

**`only edges`** now points at the un-granted publish, at the
subscription that receives it (with the exact grant line to add), and
at an un-grantable call — with why a grant is not the fix there.

**`bound`** knew which of four conditions made a count unbounded and
printed all four:

> a recursion cycle, loop-nested carrier, indirect call, or computed
> publish subject makes the count unbounded

Now it names the one that applies and points at it:

```
claim `one` violated: paths from `planners` carry an unbounded number of
`llm` sites (limit 1) — a carrier is reached from inside a loop in
`Planner::go`, so it repeats per iteration

bnd.hl:10:22: claim `one`: this is the loop-nested carrier
                self.n = model_call(i);
                         ^^^^^^^^^^^^^
```

The classification was always computed — reaching the verdict
requires it — and then discarded. Keeping it costs one enum on the
unbounded side of the heaviest-path result. Recursion and the step
ceiling are properties of a walk rather than of one expression, so
they name the fn and omit the caret; the other three carry a site.

Secondary spans follow the same bundle-only rule as the `forbid`
witness: stdlib bodies parse in their own offset space, so a span
from there would point at the wrong source.

### Interface dispatch is named in the witness (downstream review)

A call on an interface fans out to **every** conforming locus, which
is sound and deliberately conservative. But the witness rendered that
hop as an ordinary direct call, so a claim could report:

```
`A::go` -> `notify` -> `Sms::send`
```

while the line in front of you constructs an `Email`. A correct proof
that reads as a compiler bug is expensive — people stop trusting the
checker, or work around it.

The hop is now rendered as what it is, and the "where to edit"
diagnostic explains the fanout rather than leaving the reader to
disbelieve it:

```
witness: `A::go` -> `notify` -(dispatches Notifier.send)-> `Sms::send`

claim `no_sms`: the boundary into `texting` is crossed by this dispatch
through `Notifier`. A call on an interface reaches EVERY conforming
locus, whatever this expression happens to construct — so the witness
names one the claim forbids. Narrow the receiver's type, or exclude
the conformer from the group.
```

The fact was already in the model (the artifact tags these edges
`via_interface`); it just never reached the human. Witnesses with no
interface in them are unchanged.
### Topology artifact schema 1.4: one verdict vocabulary, and the document's own verdict

Bundle claims and fn-grained certificates (`@effects`, `@budget`,
`@phase_effects`) are the same kind of statement at different
granularity — #392 §8 already reports the second as the claim form it
is pointwise sugar for. They disagreed about how to spell an outcome:
claim rows carried three states, `lowered` rows carried a bool.

Both now use `Verdict`, and it distinguishes a case that was
previously folded away:

| verdict | meaning | repair |
|---|---|---|
| `holds` | proved | nothing |
| `violated` | disproved — a counterexample exists | fix the program |
| `uncertified` | well-formed but not provable — the graph has an unknown | resolve the unknown edge |
| `invalid` | the statement is malformed | fix the claim |

`violated` and `uncertified` were one value because **unknown ⇒
violation** — an indirect call fails closed rather than certifying an
absence nothing established. That rule is unchanged and both still
fail the build. What changes is that the artifact records which
happened: the repairs differ, and composing models across binaries
needs a propagated unknown to read as "not provable" rather than as
disproved when nothing disproved it.

The artifact also gains a top-level **`verdict`** (`clean` /
`law_failed`) — every law reduced to one field, so a consumer does
not reconstruct it by walking two arrays and knowing which strings
count as passing. Only `holds` passes: a law that could not be
checked has not been satisfied. It says nothing about whether the
program typechecks, because an artifact is only emitted for one that
does.

### A topology artifact is only emitted for a program that typechecks

`hale check broken.hl --dump-topology` emitted a **full artifact** for
a program with a type error: populated relations, and claims evaluated
over a graph derived from source the compiler could not understand. A
claim would report `"result": "holds"` for a program that cannot
compile — a certificate asserting a property of something that will
never run.

That is worse for a consumer than emitting nothing, because it fails
open: an admission step looking for "no violated claims" *passes* it,
since there are none.

The artifact's existence now means the model is sound. A program that
does not typecheck emits no artifact and says why.

A **violated** claim is the opposite case and still emits, unchanged:
the model is well-defined, the row is a truthful report, and replaying
a violation independently is the point of publishing the model. The
new `DiagKind::Claim` separates the two — errors like any other, and
rendered identically (the message already begins "claim `x`
violated", so a distinct prefix would only stutter), marked at the one
place every claim diagnostic funnels through rather than at its ~30
construction sites.

`check` now runs its analysis once and shares it between the artifact
gate and the diagnostic report, so this costs nothing on a
`--dump-topology` run.
### `hale check --workspace` (downstream review)

`check` operates on one seed and does not recurse — correctly, since
a directory is one compilation unit. The consequence was that a
repository with several seeds needed a shell loop each project wrote
itself, and a library or main-locus claim was enforced only where
somebody remembered to point `check`.

```sh
hale check --workspace .        # every seed under `.`, each on its own
hale verify --workspace .       # same, with advisories gated too
```

Every seed runs even when an earlier one fails: a runner that stopped
at the first failure would report a subset of the truth, which is the
shape of thing this exists to remove. The summary names the failing
seeds, and the exit status is the worst of them so a usage error is
not masked by an ordinary failure elsewhere. `vendor/`, `target/` and
dot-directories are skipped — a seed you do not own is not yours to
gate.

It does **not** connect seeds. Each stays its own closed world; two
binaries publishing and subscribing one topic are not linked by this,
because nothing about a deployment is visible from source and
inventing those edges would certify a system nobody deploys.
Per-seed artifact flags are rejected in combination with
`--workspace`: N seeds are N models, so there is no single artifact
to emit or gate against.

### Topology artifact schema 1.3: an integrity digest

`shape_hash` is an **identity**, not an integrity check. It covers
the model half only — deliberately, so a moved comment or a renamed
claim doesn't churn the model's identity — which leaves `topics`,
`provenance` and the claim results outside it.

That was fine while the only consumer was the compiler that had just
produced the document. It stops being fine as soon as anything trusts
an artifact it did not produce, and two concrete holes follow:

- **The baseline gate could be forged.** `--check-topology-shape`
  greps the `shape_hash` line out of a committed baseline, and
  nothing forced the rest of the document to agree with it. Editing
  that one line made the gate pass.
- **The cross-binary join key was unverified.** Composing artifacts
  from separately compiled applications joins endpoints on the
  `topics` rows (wire subject + payload hash) — a section
  `shape_hash` does not cover. Verifying the shape hash and then
  joining on those rows means the key was never checked.

Schema 1.3 adds `artifact_digest`: FNV-1a/64 over the entire body,
results and provenance included, emitted as the final key so
everything preceding it is exactly what was hashed — verification is
a prefix hash, with no re-serialization or canonicalization step.
Both baseline gates now reject a file whose digest disagrees with its
contents as *corrupt* (exit 2) rather than reporting it as a
mismatch.

`verify_artifact_digest` returns `None` for an artifact with no
digest (anything before 1.3) rather than `true`. A consumer may
choose to accept an older artifact; it must never read "nothing to
check" as "checked and intact".

### `check` / `verify` argument handling, and claims in the README

The rest of the v0.15.0 claims developer-experience review. The
critical topology findings shipped in the previous entry; these are
recommendations 3 and 7.

**Real argument parsing.** `check` took its target from `argv[2]` and
treated everything else as scenery, so an unknown flag, a stray second
positional, and `--help` were all ignored while the command still
reported SUCCESS. A typo'd gate flag meant CI checked nothing and said
so in green. Now: unknown flags and extra positionals are usage errors
(exit 2), `--help` prints per-command help instead of being read as a
path (it used to print `not a file or directory: --help` and exit 0),
and flags may appear on either side of the target.

**`--dump-topology` takes its destination as `=<path>` only.** Its
operand is optional, so consuming the following token has no safe
reading — and once flags became legal before the target, `hale check
--dump-topology app.hl` OVERWROTE `app.hl` with the artifact. A bare
`--dump-topology` still writes to stdout. The two `--check-topology*`
gates take their mandatory operand either way.

**Discoverability.** `hale check --help` documents the topology,
effects and budget flags with what each one gates; the top-level
usage points at it. The README gains a claims section — the six
forms, a worked cross-seed example with its real witness output, the
group/unknown/library-tier rules, and the artifact commands — since
the shipped README discussed effects but never mentioned claims.

Recommendations 4, 5, 6 and 8 (secondary provenance for `only edges`
and `bound`, interface-fanout rendering, `hale claims` inspection
commands, workspace-level verification) are not in this change.

### The claims artifact's external contract (downstream handoff)

Four findings from an outside developer-experience review of the
claims tooling. Every one of them was a **fail-open**: the tool
reported success while doing nothing, which is the worst failure
mode for something whose job is to gate CI.

- **The artifact was not valid JSON.** The `claims` array was
  never closed before `"lowered"` began, so *every* artifact —
  no claims, one, many — was rejected by any standards-compliant
  parser. It survived because the existing tests assert on
  substrings and grep out the `shape_hash` line; none parsed the
  whole document. The new `artifact_is_valid_json` does, which is
  the only shape of test that could have caught it.
- **Observing a program changed its verdict.** `hale check
  failing.hl --dump-topology` exited 0, while the same file
  without the flag exited 1 with its witness — dump mode returned
  before the checker ran. A CI job that added the flag to collect
  an artifact silently stopped gating. The dump now prints and
  falls through to the check.
- **`--flag=value` was ignored, and the command still succeeded.**
  Both spellings (`--flag value` and `--flag=value`) are now
  accepted, and a missing operand is a usage error (exit 2)
  rather than a silent no-op. `--dump-topology=<path>` writes the
  artifact to that file instead of ignoring it.
- **The baseline gate could not distinguish a moved comment from
  a changed program.** `--check-topology` compares the entire
  artifact including provenance offsets, so a leading comment
  failed the gate reporting that the "model" had changed when
  `shape_hash` — the model's identity — had not. Both gates now
  exist and are named for what they compare:
  `--check-topology` is the exact snapshot (law + model +
  provenance); `--check-topology-shape` gates the model alone and
  is immune to source motion and claim renames.

`crates/hale-cli/tests/topology_artifact_contract.rs` pins all
four, in both directions where a gate is involved — a loose gate
that never fires is the same fail-open wearing a different hat.
### The hot-path lint now sees factory calls (GH #402)

The advisory that flags a locus allocated per loop iteration matched
a locus **literal** only. But a method cannot return a locus (m90), so
factoring construction out of a body leaves a free-fn factory — and
`let m = zeros(rows, cols)` in a loop allocates a fresh arena every
iteration exactly as `Matrix { }` would, reclaimed only when the
enclosing fn returns.

The result was silence precisely where it mattered. A workload built
entirely on factories — the #402 reference workload is — grew linearly
with no diagnostic at all. The lint now resolves a call's callee to
its signature and fires when the return type is a locus, naming both
the factory and the locus it returns. Cross-seed calls resolve too
(the callee keeps its author spelling while the symbol is merged
mangled); an ambiguous tail name is dropped rather than guessed.

Only the `let`-bound form warns: since the previous release an unbound
factory result is registered and reclaimed at its statement, so
dropping the binding is a *fix* the advisory suggests, not a finding it
reports. Straight-line calls outside a loop or handler stay silent, as
they always have. The in-tree corpus produces zero new warnings.

### The per-topic observation identity, pinned and exported (GH #399)

The observer protocol's open item — "exact shape_hash definition,
needs the canonicalization pinned down with the hale team" — is
closed, and the reconciliation ruling with it:

- **One implementation.** `hale_types::topic_identity` owns the
  wire-subject rule (parent-joined `subject:` dot-path, declared
  name as fallback), the canonical payload shape (`field:tag`
  list, tags `i f b d t u s y struct`, name-free for compounds;
  empty for non-struct payloads), and the hash (FNV-1a/64 over
  `subject ++ ':' ++ shape`). Codegen now calls it for both bus
  routing and observer shape registration; the pinned test vectors
  are wire contract.
- **Emitter fix.** Shape registration previously keyed by the
  UNJOINED declared subject while publish-side manifest rows key
  by the parent-joined wire subject — a parented topic's manifest
  row hashed the empty shape. Registration now uses the joined
  subject, so parented topics' identities include their payload
  shape. (Manifest hashes for parented topics change.)
- **Artifact schema 1.2 — the join document.** The topology
  artifact gains an unhashed `topics` section: per topic, the
  author-spelled name, the RAW wire subject (byte-exact manifest
  join key — a subject-less imported topic registers under its
  mangled local name; declare `subject:` on shared topics to fuse
  across binaries), the canonical shape, and `payload_hash`. A
  recording/WAL segment carrying `(name, shape_hash)` matches a
  row and names the exact checked topology it ran under. The
  ruling is REFERENCE, not fusion: payload field shape does not
  affect claim evaluation, so the model `shape_hash` is unchanged
  — a payload field edit changes the topic row and no model
  identity (pinned by test).

Protocol: iris PROTOCOL.md §4 (definition + vectors; open item
struck). Spec: `spec/verification.md` § Claims. Docs: the claims
chapter's artifact section. Tests: `topic_identity` unit vectors,
`topology_v2.rs` identity row + hash-separation canary.

### Factory-returned loci: unbound temporaries and call-valued return arms (GH #402)

Two shapes #383 left on the program-lifetime path now reclaim too.

**Unbound temporaries.** `add(matmul(w, a), b)` — the inner result is
consumed as an argument and never named, so #383's binding-scoped
rule had nothing to attach ownership to. The frame that evaluates it
owns it now. The suppression flags that say "this value's owner is
already decided" (a `let` RHS, a `return` expression) are one-shot
rather than sticky: left sticky they swallow the whole subtree, and
the nested temporary in `let z = add(matmul(..), b);` goes unowned
again — which is the bug itself.

**Call-valued return arms.** A factory whose guard arm is
`return error_matrix();` was disqualified wholesale, even though that
arm hands back a value as fresh as the main one. Freshness is
transitive there for the same reason it is for let-bindings, and the
fixpoint already had the machinery to decide it.

Measured on the reference workload: leak 5320 → 3888 bytes, values
correct, no use-after-free. The residual is still linear in call
count, so #402 stays open — the remaining allocations are matrices
the analysis does not yet prove fresh, not a new mechanism.

### Factory-returned loci are reclaimed by the binding that names them (GH #383)

`let m = zeros(n);` used to leak: a locus returned from a free fn is
placed in a program-lifetime arena (m90) and nothing ever reclaimed
it — its arena and any `@form` storage it grew lived until process
exit, which in a `fit()`-style loop is unbounded.

It is now owned by the binding and dissolves at that scope's exit,
like any `let`-bound locus. What makes this sound is the v0.14
ownership rule: a locus value cannot be assigned into a locus-typed
field, so the binding is the only place a factory result can come to
rest. The four earlier attempts on this issue failed precisely
because they had to guess that owner.

Two guards keep it correct, both pinned by tests:

- A whitelist analysis admits a fn only when every return is a locus
  literal or a single let-binding that is itself fresh (a literal, or
  a call to another qualifying factory — fixpointed, so helpers built
  on factories qualify), and that binding never escapes into argument
  position or another literal. Anything unrecognized answers *not*
  fresh: the old leak, never a double free.
- A fn that does **not** qualify as a factory can still return a
  locus it bound from one, so every free fn's returned bindings are
  recorded and never dissolved by their own frame. Without this the
  caller receives a dissolved locus, which reads back as zeros rather
  than crashing.

Also fixed en route: `di_current_loc` was not reset when lowering
moved to a new function, so an entry-block alloca emitted before the
body's first statement inherited the *previous* function's debug
location and LLVM rejected the module ("!dbg attachment points at
wrong subprogram"). Latent before; reachable now.

Still leaking, and tracked on #383: unbound temporaries
(`add(matmul(w, a), b)` — the inner result is never named, so a
binding-scoped rule cannot reach it) and factories whose second
return arm is a call. On the reference workload this closes roughly
a third of the total; the remainder is those two shapes.

---

## v0.15.0 — claims: the law travels, the witness says where (2026-08-04)

### Library-tier claims: law that travels with an import (GH #392 thread 2)

A library seed states its own law in a TOP-LEVEL `claims { }` block
— no `main locus` required. The block travels with the import and
re-evaluates in **every closing build** over the merged world:
checked standalone the seed satisfies itself; when an app quietly
wires a second subscriber onto the library's topic, the library's
own `count subscribers(topic Charges) <= 1` refuses the build —
with seed attribution (``claim `pay::single_settle` violated``,
never a mangled symbol) and a span pointing at the library's own
claim line.

The tier split is the enforcement surface for "a dependency may
not brick downstream builds with world-claims": a seed swears
about *itself and its own boundary* (it can only name what it can
see — its own decls, its own imports), world-quantification stays
main's, and a seed that declares `main locus` writing the
top-level form is a check error. Traveling blocks are marked at
the mangle stage (which only ever touches imported seeds); their
group and topic references canonicalize across the seed boundary
exactly as group declarations do (#334), and claim names are
never mangled — attribution is `alias::name`. Claim-name
uniqueness is per seed.

Grammar: `spec/grammar.ebnf` (`top_decl` gains `claims_block`).
Spec: `spec/verification.md` § Claims (library tier). Docs: the
claims chapter (placement + a dedicated section). Tests:
`library_claims.rs` (tier rejection, standalone evaluation),
`xseed_library_claims.rs` + the `xseed-library-claims` fixture
(travel, re-check at close, attribution, demangling).

### The normalized, provenance-bearing model (GH #392 thread 1)

The architectural milestone every reviewer of the #382 stack named:
one derived model — declaration provenance, the phase relation, the
seed sort — consumed by every judgment and exported whole.

- **Witnesses say where to edit.** A `forbid` violation now emits
  secondary diagnostics in the effect system's root + leaf shape:
  the call that crosses the boundary (or, for a bus hop, the
  publish site and the subscription declaration) and the forbidden
  destination's declaration. Spans are emitted only for bundle
  decls — stdlib bodies parse in their own offset space, and a span
  from there attributed to a user file names the wrong line (a
  pre-existing misattribution this change also stops).
- **`during` rides the phase relation.** Lifecycle hooks and modes
  are hook-phases, methods their own source-slice phase; the
  relation is explicit, exported, and what the evaluator reads.
- **Topology artifact schema 1.1 — the model export.** The hashed
  model half gains call-edge weights (`loop`, `unbounded`,
  `via_interface`), the through-stdlib contraction
  (`calls_via_stdlib`: user→user paths with stdlib interiors,
  collapsed with a conservative loop flag), the `phases` relation,
  the `seeds` sort, and compiler-derived per-fn `effects`. A new
  UNHASHED `provenance` section carries per-edge and per-decl spans
  as bundle-global byte offsets — moving code changes every span
  and no identity. v2 scope: every claim verb replays independently
  over the exported relations; still compiler-certified: `bound`
  over built-in classes and walks past the step ceiling. Existing
  `shape_hash` values change (the hashed half grew).

Spec: `spec/verification.md` § Claims (witness provenance, phase
relation, artifact scope). Docs: the claims chapter (witness,
`during`, artifact schema). Tests: `model_provenance.rs` (witness
spans incl. the foreign-span guard, phase selection, model
derivation), `topology_v2.rs` (sections, motion-insensitivity
canary, contracted edges).

### §8 — one schema of record, and `@phase_effects` user classes (GH #392 thread 1)

- **Certificates as lowered claim rows.** The topology artifact
  gains an unhashed `lowered` array: every fn-grained certificate —
  each `@effects` assert, each `@phase_effects` phase contract, each
  `@budget` in both families — reported as the claim form it is
  pointwise sugar for, with its verdict (`forbid reaches({F},
  effects(money))`, `bound alloc <= N on paths from {F}`, `only
  effects {…} on {L} during birth`). Rows come from the same
  evaluations that gate the build, so the report and the build
  cannot disagree. One schema of record: all law, bundle-quantified
  and fn-grained, in one artifact.
- **`@phase_effects` closes over user classes.** A phase contract is
  now closed over the live class universe like `only:` — a phase
  reaching a declared user-class carrier without listing the class
  violates, and listing it permits it (atomic-only complement;
  composed classes own no bit). Previously the walker iterated a
  hardcoded nine-class list and the parser rejected user class
  names outright — the documented deficiency. Programs with
  `@phase_effects` AND declared user classes may see new (correct)
  violations.

### Interface-dispatch edges: closed-world fan-out (GH #392 thread 4)

A method call on an interface-typed value
(`route.handler.handle(ctx)` — the stdlib router's own shape)
used to land as an unresolved edge that every walker silently
dropped: the one receiver shape left, after the v0.14.0 root fix,
where a real call contributed nothing to any judgment. An
`@effects(none: …)` certificate over a fn dispatching through
`std::http::Router` certified while the handler performed the
effect.

The world is closed, so the implementor set is enumerable. The
summarizer now fans the written edge out to every conforming
locus (structural name-and-arity conformance over the
declarations — a superset of the checker's typed conformance;
over-approximation only adds edges). Reachability and effect
judgments walk every alternative. Counting judgments — `bound`,
`@budget`, the quantitative dims — take the **max** over one
dispatch site's alternatives (a dispatch invokes exactly one
target; a sum would count phantom calls no execution performs),
carried by a `dispatch_group` tag on the fanned edges and a
`join_alternatives` fold in the shared call-graph walker.

An interface **no** locus conforms to has no values in a closed
world (an interface value only arises by coercing a conformer),
so its call sites are dead — they contribute nothing, rather than
failing closed. The everyday instance is the router's
`m.before(cur)` over an empty middleware list: failing closed
there would refuse every certificate through the stdlib router.
The topology artifact records each such site
(`uninhabited_interface_call:<iface>.<callee>`) inside the hashed
model half, so a conformer appearing later changes `shape_hash`.

Two adjacent holes closed in the same change:

- **Path-written stdlib types now type receivers.** A param or
  field written `std::io::tcp::Stream` / `std::http::Router`
  recorded only its last path segment, a name no summary map is
  keyed by — so method calls on such receivers were unresolved
  and invisible. They now resolve through the same std-vs-user
  rule struct-literal receivers already used. The committed
  effects baseline gains `publish` on two corpus `handle_request`
  rows — real: `Stream::recv` publishes to the tcp log-event
  topic, previously unseen.
- **`@budget(alloc_per_call)` fails closed on untyped
  receivers.** The v0.14.0 root fix wrote the dual-cause message
  but widened only the other walkers' guards; the budget's own
  guard still tested `indirect` alone, leaving the receiver
  branch dead and the budget certifiable through a wrapper. The
  shared predicate (`CallEdge::opaque_method_call`) now backs all
  five walkers.

Spec: `spec/verification.md` § Claims (fan-out, max-over-
alternatives, dead-dispatch rules). Docs: the claims chapter's
fail-closed section and artifact schema. Tests:
`interface_dispatch.rs` (14: canary + control per judgment form,
incl. the router end-to-end pair), artifact rows + `shape_hash`
sensitivity in `claims_artifact_unknowns.rs`.

---

## v0.14.0 — claims: the program owns the law, the compiler owns the proof (2026-08-04)

### Receiver typing: the summarizer root fix (GH #382 soundness audit)

The four receiver shapes behind the audit's false-certificate class
are now TYPED at the source: a struct-literal receiver
(`B { }.work(n)`), a chained field (`self.mid.inner.work(n)`, via
the per-locus field maps applied transitively, plain-struct fields
included), a call-result receiver (`let b = make_b(); b.work(n)`,
via a free-fn return-type map — methods never return loci, per the
no-locus-return rule), and a uniform if/else value. Typing also
covers the iteration shapes real programs walk: `for` binders over
array-typed fields, capacity slots, array-typed params, array
literals, and the implicit accepted-children collection (`for
child in self.children` types from the `accept` param); `or`
dispositions unwrap to the inner value's type; and a single-slot
collection's `.get(i)` returns its element type (the stdlib
chain-runner shape). Net effect on the committed corpus baseline:
ZERO rows changed — identical certificates, with the
false-certificate shapes closed and no over-fire. Each resolves to a real call-graph edge,
so `@effects(none:)`, every `@no_*` certificate, `@budget`, the
quantitative dimensions, AND the claims evaluators now see the
path and report real witnesses instead of refusing to certify —
the audit's repro programs ship as standing negative controls for
both systems (`receiver_shapes.rs`).

The residue — a receiver that still cannot be typed (an index
result, a match value, a foreign expression) — now fails closed
consistently in every judgment that traverses calls: the claims
backstop, `@effects` classes, `@budget`, and the quantitative
dimensions all treat the edge as may-do-anything. This closes the
temporarily inconsistent state where a bundle-level claim refused a
path that a fn-level certificate quietly passed. The topology
artifact keeps recording residual edges
(`untyped_receiver_call:<callee>`) inside the hashed model half.

### Claims: the unresolved-callee backstop (GH #382 soundness audit)

An adversarial audit of the claims evaluators found a
false-certificate class: four receiver shapes — a struct-literal
receiver (`B { }.work(n)`), a chained field (`self.mid.inner.work`),
a call result, a branch value — land in the call graph as
unresolved edges with no receiver type, and a walk that ignored
them certified `forbid reaches(A, B)` while the forbidden path
executed at runtime. Claims now fail closed on EVERY such
untyped-receiver call in any judgment that traverses calls (forbid,
only-edges, bound) — a follow-up review showed a name-keyed
backstop was still blind to wrappers reaching the target
transitively, so no name comparison is sound; the edge itself is
the uncertainty. The topology artifact records each one
(`untyped_receiver_call:<callee>`) inside the hashed model half,
so introducing one changes `shape_hash`. Synthesized form/builtin
methods carry a known receiver type and are exempt, so existing
certificates over `counts.set(x)` and friends are unaffected. The underlying
summarizer gap is shared with the effect system (`@effects(none:)`
misses the same shapes — pre-existing, not new to claims); the
root fix (typing those four receiver shapes in the summarizer,
which repairs both systems) is tracked on #382.

### Claims phases 2–5: grants, families, budgets, coverage, the artifact (GH #382)

The claim surface is now the full verb set from the issue's build
order. `only edges A -> B { publish T; }` makes a boundary's grant
list exhaustive and reviewable — every un-granted direct edge is
reported, and call edges are never grantable. `bound llm <= 1 on
paths from planners` puts `@budget`'s per-call semiring behind a
claims surface (a loop-nested or recursion-reachable carrier is
unbounded and violates). `require subscribes/publishes(some G,
topic T)`, `cover topic in seed(a): subscribed_by(some G)`, and
`count publishers(topic T) == 1` cover existence, seed-wide
coverage, and the single-writer cardinality. `forbid` gains
`during <phase>` (quiet-boot claims) and `avoiding <group>` — the
interposition form: "every path passes the gate" is `forbid
reaches(A, B) avoiding gate`.

Effect classes gain **indexed families**: `domain wing = { delta,
gamma }; effect knowledge(wing);` interns every instantiation as an
ordinary class and `knowledge(*)` as an auto-populated composed
class — a reduction onto shipped machinery, so a misspelt index is
an undeclared-class error and a domain member added later lands
outside every existing `only:` contract. Companion:
`@budget(<user class> = N)` bounds calls to declared carriers.

And the checked model now leaves the compiler: `hale check <t>
--dump-topology` emits the **topology artifact** — sorts,
relations, and every named claim's result in author spelling,
under a schema version and a `shape_hash` over the model half —
and `--check-topology <baseline>` fails CI when topology or law
changes without review (the `.hale.effects` precedent). Spec:
`spec/verification.md § Claims`, `spec/grammar.ebnf`.

### Claims: domain requirements as checked sentences (GH #382, phase 1)

The judgment layer every structural check already used — derive a
graph from source, evaluate a property, witness the failure — is now
a user-facing surface. `group NAME = { … };` declares vocabulary (a
named set of loci / fns, including imported decls via `alias::Name`
and `alias::*`), and the new `claims { }` member on `main locus`
holds named, bundle-level sentences over the program graph:

```hale,fragment
group delta_wing = { delta::*, DeltaStore };
group gamma_wing = { gamma::Research };

main locus Org {
    claims {
        iso_dg: forbid reaches(delta_wing, gamma_wing);
        no_spend: forbid reaches(gamma_wing, effects(money));
    }
}
```

Phase 1 ships one verb — `forbid reaches(SRC, DST) [via { calls,
bus }]`, absence under the composed call ∘ bus closure — with the
soundness posture the effect system established: unknown group
member = error (never an empty set), empty group = vacuity error
unless `may_be_empty`, indirect calls and computed publish subjects
fail closed, and a violation renders a minimal countermodel path in
author spelling (`` `delta::Triage::on_task` -(publishes
"org.metrics")-> `gamma::Research::on_metric` ``). Claims are
errors gating `hale check` — weakening the law is a source diff,
which is the review event the surface exists to create. Groups
cross seed boundaries through the same mangle-stage canonicalization
as topics (#334); witnesses demangle. `claims { }` is main-only:
main is the closed-world gate, so bundle-wide claims cannot be
evaluated anywhere earlier. Spec: `spec/verification.md § Claims`,
`spec/grammar.ebnf`; the remaining #382 phases (`only edges` grants,
`require`/`cover`/`bound`, indexed families, the topology artifact)
are tracked on the issue.

### A locus-typed field may only be assigned a locus literal

`self.conn = Connection { url: next };` stays what it always was — a
lifecycle transition, break-before-make, the new instance built into
this locus's arena and owned by the field. But assigning a locus
value produced **elsewhere** — `self.held = make_row(…);`, or
`let c = Conn { … }; self.conn = c;` — is now a typecheck error.

The reason is ownership: the field claims the instance (its teardown
reclaims it when self dissolves) and so does the frame that produced
the value (its scope exit reclaims what it built), and nothing in the
language decides between them. That ambiguity was previously hidden
rather than resolved — a locus returned from a free fn is routed to a
program-lifetime arena and never reclaimed, so such a store *appeared*
to work only because nothing ever freed it. **The leak was the safety
mechanism**, the same shape as the synced-map clone-on-read and the
form-vec zero-reads earlier in this cycle.

Two remedies, both existing shapes: assign a literal (construction in
place), or route membership through `accept(c: L)`. This is the same
principle as the no-locus-return rule on methods — a locus is
structure, not a value to hand around. Ordinary `let`-bound loci,
including factory results, are unaffected.

Surveyed before adopting: **zero occurrences across 60 bundles in five
downstream repositories**, so the restriction forbids a pattern nobody
writes. It is the ownership decision GH #383 was blocked on; the
codegen half of that issue (reclaiming factory-returned loci) remains
open, but the semantics it needs are now settled.

### A locus `let`-bound in a method now dissolves when the method `return`s

`lower_return`'s method arms destroyed the per-call scratch and
returned **without flushing the deferred-dissolve frame**; the
terminated-body arm then popped that frame unflushed. Only a
fall-through exit ever ran the dissolves. So a locus bound in any
method that returns was never torn down — its `dissolve()` never ran,
its subscriptions stayed registered, its arena and `@form` buffers
leaked — silently, with no diagnostic:

```hale
fn step(i: Int) -> Int {
    let w = Watcher { id: i };   // subscribes, opens an fd, …
    return i * 2;                // ← w never dissolved
}
```

Nothing about factories or cross-seed calls is involved; a plain
locus literal leaked the same way, on every value and void return
path. Found while investigating GH #383, which remains open for its
own (harder) case: a locus returned *by* a free-fn factory still has
no owner the compiler can name, so it stays program-lifetime.

Ordering is load-bearing and is now pinned by a test: the flush runs
**before** the scratch destroy, because a dissolve is a method call
whose call site publishes the caller-arena TLS — flushing afterwards
publishes a pointer into freed scratch, reproducing the #375/#381
use-after-free exactly.

### Element chains: the full vocabulary

The v0.13.0 chain mechanism (`filter` / `count` / `into`) gains the
rest of its table. Stages: `map(expr)` rebinds the element.
Terminals: `sum()` (Int elements; `map` first to project), `any(pred?)`
/ `all(pred)` (Bool, with spec'd vacuous truth — `any` on empty is
false, `all` on empty is true), `first()` / `find(pred?)` /
`min(key?)` / `max(key?)` (element-valued, **fallible on empty** —
they lower to an index search whose value is the source's own
`get(idx)`, so an empty result is the ordinary IndexError and
`or raise` / `or fallback` / `or handler(err)` apply with zero new
error machinery), and `each { … }` (the block is spliced as the fused
loop's body with `it` bound — no closure exists, and `break` /
`continue` act on the loop; the desugared loop increments first, so
`continue` advances rather than spinning).

Everything still fuses to one loop with nothing produced between
stages, so every chain remains legal under `@budget(alloc_per_call =
0)` / `@hot`, and predicates' effects stay attributed to their own
source lines. Recognition stays conservative: user facade methods
named like terminals resolve normally — stage-less recognition
requires an argument that mentions `it` (unbound outside a chain) or
`each`'s block, both shapes no ordinary call can have. `min`/`max`
return the *element* with the least/greatest key (`min_by_key`
shape). `map` does not compose with the fallible terminals (the `or`
fallback would need the mapped type while `get` yields the element);
project after the find. `sort` / `reverse` / `group` remain future
materializing terminals into caller storage.

### The caller-arena TLS can no longer outlive the arena it points to (GH #375)

A free-fn factory that builds and grows a `@form(vec)` locus
segfaulted deterministically when called after a locus-method failure
was caught with an `or` handler — the cross-seed shape downstream
worked around with inline construction. Root cause: the caller-arena
TLS is a set-and-forget channel. A fallible method chain that
published its own method scratch and then exited down the error edge
left the TLS pointing at the destroyed scratch, and the next TLS
reader without its own preceding publish allocated out of freed
memory (ASan: heap-use-after-free in `lotus_arena_alloc`).

Three layers, each independently sound:

1. `lotus_arena_destroy` clears the TLS when it points at the dying
   arena — the single point every arena death passes through, so a
   destroyed (or recycled) arena is never reachable via the TLS
   again; readers fall back to the documented global-arena path.
2. Free-fn prologues publish their `__caller_arena` param to the
   TLS, re-healing it on entry and giving TLS readers the same
   lifetime an inlined body would have used. Gated on the body
   containing a construct that can read the TLS (an instantiation
   or a call) so scalar leaf fns skip the publish — ungated it
   measured +86% on the 2ns/call microbench.
3. Method epilogues restore the entry-time snapshot on both the ok
   and the error exit, making method calls TLS-neutral.

The reported reproducer (two pond seeds + probe) runs 10/10 clean,
ASan-clean; a minimal two-seed regression test is pinned in-tree —
notable because every single-file reduction stays clean, which is
why the issue's toy matrix could not reproduce it.

### Synced `@form(hashmap)` maps now retire replaced String clones (downstream handoff P1)

A `sync = serialized` map never installed a retire descriptor, so
replaced String clones accumulated in its arena for the life of the
process — the long-standing residual behind churned recorded-state
maps growing linearly in set count. That was not an oversight: `get`
memcpy'd the cell, so its String fields came out as raw pointers into
the map's arena, and a reader on another pool could hold one across
the writer's activation boundaries. The leak *was* the safety
mechanism.

Every read path on a synced String-bearing map (`get`, `entry_at`,
`for` iteration) now clones the cell's Strings into the caller's
arena, inside the same critical section that read the cell — so the
reader owns its copy, the writer's blobs have no off-thread readers,
and the ordinary activation-boundary flush becomes sound with no
epoch scheme. The map's arena serializes its allocator and retire
lists (`retire_lock`, distinct from the allocation lock; documented
lock order). `striped` and `lockfree` maps are unchanged pending
their own audit. Values read out of a synced map remain plain owned
values — no user-visible lifetime rule changes.

### Two hot-path regressions fixed (bench attribution vs released compilers)

- **`@form` set/put on scalar-only cells no longer pays the cell
  single-owner tax.** The v0.11.12 single-owner fix emitted a stack
  snapshot + owned-clone walk on every set/put, but both exist solely
  to keep heap-pointer (String/Bytes) leaves un-aliased — a cell of
  pure scalars has nothing to protect, and the snapshot's
  store-to-load round trip serialized hot insert loops. Now gated on
  the cell type tree; unrecognized types conservatively keep the
  snapshot. 1M Int-keyed inserts: 62.7 → 49.7ms.
- **`@form` set/put no longer arena-allocates the argument struct
  literal.** `m.set(Entry { ... })` bump-allocated the literal's
  shell into the enclosing scope's lifetime arena and then memcpy'd
  it out — 16 MB of dead shells per million sets. A literal of the
  exact cell type in set/put argument position now builds in a stack
  slot (the pattern `lower_send` has used for publish payloads since
  m67). 1M-insert bench maxrss 92.5 → 81.4 MB.
- **A bundle that can never enqueue a bus cell emits no drains at
  all.** Cells come only from subscriber dispatch, wire ingest,
  cross-pool accept handoff and transport-loss dispatch; a program
  with no topics, bus blocks, bindings, accepts or perspectives
  provably has an empty queue at every statement boundary / scope
  exit / sleep slice, so the drains are elided at emission.
- **Runtime observation probes are now gated at the call site.** The
  bus publish/deliver/net probes were called unconditionally from
  the C dispatch path, paying the probe's prologue before its own
  `obs_on()` check — +3ns/event on the 3-stage pipeline bench, the
  v0.11.12 native-obs-emission regression. The C call sites now
  branch on `lotus_obs_live`, the same dormant-cost gate generated
  code has used since #328 (set in a pre-main constructor whenever
  `LOTUS_OBS=1`, so 0 proves the probes are permanently dormant).
  Pipeline bench restored to byte-parity with v0.11.11; obs
  emission/fleet-contract suites unchanged.
- **A no-op bus drain no longer builds its frame.** Codegen emits
  `lotus_bus_queue_drain` at every statement boundary, scope exit and
  sleep slice; the drain called `pthread_self` and the transport-loss
  dispatcher before looking at the queue. In single-threaded mode
  (`g_bus_has_pinned` unset — set pre-spawn by both pinned
  registration and pool startup) an empty queue with nothing lost now
  returns via a call-free fast path. Locus birth+dissolve cycle:
  2.01 → 1.61ms, 6% faster than the v0.11.3 baseline.

### The birth-order trap is now diagnosed (downstream handoff)

A params field whose `run()` runs inline on the main thread and never
returns prevents every field declared after it from being **born** —
not merely from running. The later locus's `birth()` never executes, so
the subscriptions it registers and the sockets it binds silently never
exist. The process completes what looks like a normal boot and then
idles, and the symptom ("my handler never fires") points at the bus
rather than at the params block.

`hale check` now warns wherever it can prove the shape: a terminal
`while` loop with no `break`/`return`/`terminate`, in a field placed on
the main thread (default placement or an explicit
`cooperative(pool = main)`), with at least one field declared after it.
The proof is conservative and reports only the first blocker, so there
are no false positives on the 198 `.hl` programs in tree — but absence
of the warning is not a guarantee of correct ordering.

Only the *blocking* field's placement matters: `pinned` and any
non-`main` cooperative pool lift the block, while the later field's own
placement is irrelevant (instantiation itself runs inline on main).
Both remedies — reorder, or place the blocker off-thread — are named in
the diagnostic.

This also **corrects a misdiagnosis of our own.** A downstream handoff
reported "a bus handler's write to a `self` param isn't observed by
`run()`"; we re-diagnosed it as ordering rather than coherence
(correctly) and then filed it a second time as an open *handler
cadence* question — "only the main locus is serviced from inside its
own `run()`" — with an `#[ignore]`d reproducer pending a model
decision. There was no model decision to make. A cooperative child's
sleep-slice drain services its handlers correctly; the reproducer had
simply declared the subscriber before the publisher, so the publisher
did not yet exist. Swapping the two declarations makes the handler fire
mid-loop exactly like the main locus's, and that reproducer now runs
live instead of ignored.

`spec/semantics.md` gains § "Birth order is load-bearing" with the
measured placement matrix, and its shm_ring birth-order note is
corrected: the advice to "move the publishing into a `run()` body that
runs after all child births" is sound only for the **main locus's**
`run()`, never a child's.

## v0.13.0 — the certificates fail closed; the ordinary layer arrives (2026-08-03)

Minor bump, and two unrelated stories.

**The effect system stopped failing open.** v0.12.0 closed four ways
around `@effects`; this release closes the ones underneath them, and
the pattern is uncomfortably consistent — every hole was a contract
that read as verified and quietly wasn't:

1. **An indirect call voided every certificate.** `@no_syscall` on a fn
   whose body is `return f(v);` typechecked while the program
   performed the syscall, and `@budget` leaked identically. Function
   pointers were the first genuinely open-world construct in the
   language and nothing had noticed.
2. **A class past the effect mask's ceiling saturated to PURE** —
   "reaches nothing" — so `@effects(none: {…})` certified a fn calling
   a declared source of it.
3. **A misspelt class was a new class.** `@effects(none: { monye })`
   typechecked clean against a `money` source, holding vacuously.
4. **User classes did not travel over the bus**, so a `causes:`
   contract was satisfied while publishing into a money-moving
   handler. The identical shape with a built-in class reported
   correctly, which is why spot-checks missed it.
5. **Two seeds' classes aliased onto one bit**, so a `none: {money}`
   was checked against another seed's `pii`.
6. **`hale check` accepted an unknown `std::` namespace** —
   `std::totally::fake()` passed the checker and only codegen caught
   it, so a typo was invisible to the editor and to CI.
7. **wasm silently stubbed package C.** `[ffi] csrc` was never
   compiled for wasm32, so every `@ffi("c")` symbol became an
   undefined import the loader filled with `() => 0`. The build
   reported success and every call returned 0 forever.

`@effects(only: {…})` is the new contract that stops the first class of
these recurring: it states what a fn MAY do and is checked against the
complement computed from the classes that actually exist, so a class
declared later is outside it automatically. A hand-enumerated `none:`
list cannot have that property.

**And Hale grew an ordinary programming layer.** The architecture
surface has been deep for a while and the everyday one was thin enough
that you met it in the first hour: no way to split a string, no regex,
no sets, no way to read back a timestamp you had written. That is
mostly fixed. The interesting one is **element chains** —
`xs.filter(it > 2).count()` — which is not an iterator and not a
lambda: it is a form the compiler rewrites to one loop, so it
allocates nothing, is legal under `@budget(alloc_per_call = 0)`, and
needs no closure concept at all. The sequence-value question that made
"add closures, then add iterators" look like a language-sized change
turned out to be self-inflicted.

- **wasm32: a package's `[ffi] csrc` is compiled and linked** (#213).
  `link_wasm` was called without the build options, so `csrc_files`
  never reached the wasm path: every `@ffi("c")` symbol a package
  defined in C became an undefined `env` import, `--allow-undefined`
  swallowed it, and the JS loader stubbed unknown imports with
  `() => 0`. The build reported success and every call returned 0
  forever. Two constraints follow from the wasm build being
  freestanding: a translation unit that includes system headers will
  not compile (a build error naming the file, rather than a missing
  symbol), and `[ffi] link = [...]` is rejected outright because wasm
  has no dynamic linker. Note Hale's `Int` is 64-bit, so the C must
  declare `long long` — a mismatch links and then traps with a wasm
  `signature_mismatch`, which is louder than the native ABI's silent
  truncation and far louder than the stub it replaces.

- **`@budget(stack_bytes)`: the spec now states what the estimate
  rests on and what it does not cover** (#326). The entry previously
  asserted that frames "over-approximate, so the bound is safe to
  assert on" — the claim under question, stated without evidence. It
  now gives the actual argument (Hale arena-allocates arrays, structs
  and string buffers, so almost nothing but scalars is on the stack,
  which is why an 8-bytes-per-local unit is close to right here and
  would be wrong by orders of magnitude in C; and inlining removes
  call levels, the term the model spends most of its budget on), and
  states the limitation (register spills are invisible to any
  source-level model — at `-O3 -march=native` a spilled AVX-512
  register is 64 bytes against a model whose unit is 8). The bound is
  structural, over program shape, not a machine-level guarantee.
  The load-bearing premise is now pinned by tests.

- **`std::regex`** (#353). A linear-time Thompson NFA:
  `matches` (full match), `find` (leftmost byte offset, -1 if absent)
  and `valid`. The engine class was forced rather than chosen —
  backtracking buys backreferences and lookaround at the cost of an
  exponential worst case, and you cannot bound a handler that runs
  one, so it is incompatible with `@budget` and `@hot`. Supported:
  literals, `.`, `*`, `+`, `?`, `|`, grouping, character classes with
  ranges and negation, `\` escapes. No backreferences, no lookaround.
  Classified PURE — the match path allocates nothing beyond fixed
  state lists sized from the pattern, so it is usable from a
  `@deterministic @no_syscall` fn.

- **`@form(set)`** (#353). Specified and deferred in
  `decisions.md` with the trigger "revisit if a workload needs it";
  the trigger fired. Reuses the hashmap slot and the whole
  `lotus_hashmap_*` runtime, sync disciplines included, so
  `@form(set, sync = striped)` works for free. Differs only in the
  synthesized surface — `insert` / `contains` / `remove` / `len` /
  `is_empty` — which exists to keep the value off the call site:
  membership through a hashmap means writing `get(k) or false`
  everywhere.

- **Recognized element chains** (#353, cluster B).
  `xs.filter(it > 2).count()` and `xs.filter(...).into(target)` are
  rewritten to ONE loop by a post-parse pass, so typecheck and codegen
  both see an ordinary `while` and neither learns about chains. A
  chain is not a value being built — nothing is produced at any stage,
  so there is no sequence value, no owner for it, and no arena
  question. Three consequences fall out: it allocates NOTHING (so a
  chain is legal under `@budget(alloc_per_call = 0)`, unlike a design
  that returns a new collection); it is eager, so a predicate's
  effects are attributed to the predicate's own source position rather
  than to the terminal; and it needs no lambdas at all, because the
  predicate is an argument position rather than a value — `it` is
  bound per element by the desugar. Stages fuse: two filters are one
  pass.

- **A diverging `or` fallback no longer needs a substitute** (#353).
  `v.get(i) or { break; }` was rejected with "fallback type `()` does
  not match success type `Int`" — but `break` never yields, so there
  is no value whose type could match, and the rule asked callers to
  invent a substitute provably never used. `break` / `continue` /
  `return` / `fail` / `terminate` in tail position are now accepted.
  Conservative: only an UNCONDITIONAL divergence counts, since a block
  that can fall through genuinely does need a value.

- **UTF-8 code-point decoding** (#353). `std::str::cp_count`,
  `cp_at` (by byte offset) and `cp_size`. Hale's `String` stays
  byte-oriented; these let a caller walk code points deliberately
  rather than pretending bytes are characters. Normalization, case
  folding beyond ASCII, grapheme segmentation and locale collation are
  each a separate commitment with megabytes of tables against a wasm
  target, and are deliberately NOT provided — half-shipping them is
  worse than not shipping them. Invalid UTF-8 yields -1 rather than
  U+FFFD, so corruption cannot be mistaken for content.
- **`std::str::join`** (#353). The pair to `split_into`, and note it
  RETURNS where split writes: a String is already a value in Hale, so
  joining never meets the sequence-value question that forces split's
  shape. One arena allocation sized in a first pass, rather than
  repeated concatenation, so the cost is a single countable
  allocation.
- **`std::str::split_into`** (#353). Splitting a string is the most
  common operation in service code and Hale had no way to do it. It
  writes into a caller-supplied `@form(vec)` rather than returning a
  sequence, following `text::tokenize_words_into` — because Hale
  cannot return one: arrays are fixed-size types and growable
  collections exist only as locus-owned forms. That shape is also the
  allocation-visible one: the caller owns the storage, so the cost
  lands in the caller's budget instead of hiding behind a return
  value, and a `@hot` handler can reuse one vec across calls. Empty
  fields are preserved — `"a,,b,"` is four fields.

- **`std::time::parse_iso8601`** (#353). `std::time` was
  monotonic/now/sleep/time_from_unix and nothing else, so a service
  could emit a timestamp and had no way to read one back. Formatting
  turned out not to be missing — `time_from_unix` already returns
  ISO-8601 text, which is why `println` on a `Time` renders a date —
  so only the inverse was added, as `fallible(ParseError)` with a
  `can_parse_iso8601` probe, matching `str::parse_int`. UTC only: a
  timezone database is megabytes against the wasm target, and local
  time reads `TZ` and would therefore be `env`-effectful rather than
  pure. A trailing offset is rejected rather than ignored, so a
  local-time string is never silently read as UTC.

- **An indirect call no longer voids every certificate** (#353).
  A call through a function-typed parameter reached the graph as
  `Callee::Unresolved(param_name)` — indistinguishable from an unknown
  free fn, which contributed nothing. So `@no_syscall` on a fn whose
  body is `return f(v);` typechecked while the program performed the
  syscall, and `@budget(alloc_per_call = 0)` leaked identically. Every
  certificate the language offers ran through that hole. The edge is
  now marked `indirect` at construction (the enclosing fn's parameter
  list is in hand there, exactly as it is for `recv_ty`), and an
  indirect call is treated as "may do anything" rather than "does
  nothing". Deliberately conservative: exact resolution is possible
  given the closed world, but a certificate that is wrong in the safe
  direction beats one that is wrong in the other.
- **`hale check` rejects an unknown `std::` namespace** (#353).
  `std::totally::fake()` passed the checker and was caught only by
  codegen, so a typo'd or imagined stdlib call was invisible to
  `check`, to the CI gate and to the LSP — the editor would confirm
  made-up code as valid. Offers the nearest real namespace.
- **`std::str::contains` / `starts_with` / `ends_with`** (#353).
  `lotus_str_contains` and `lotus_str_starts_with` had been in the
  runtime for a long time — carrying `memory(read)` so LICM can hoist
  them — but neither was reachable from Hale. `ends_with` is new. The
  trio was previously unusable as a set: two of the three questions
  were askable and the third had to be hand-rolled.

- **An undeclared effect class is now an error** (#345). Interning
  happened on an `effect NAME;` declaration and on a bare reference in
  `@effects(...)` alike, so a misspelling minted a brand-new class that
  nothing carries — `@effects(none: { monye })` typechecked clean on a
  fn that called a declared `money` source. Same failure as the mask
  overflow from the other side: there the class had no bit, here it has
  no carriers, and both yield a certificate quietly true of nothing.
  The diagnostic offers the nearest declared name.
- **User effect classes travel over the bus** (#345). `causes:` infers
  each subscriber's effects from `frontier::infer_effects`, which
  unioned a leaf's `carries` only when something CALLED it — a fn's own
  `is: {…}` was invisible to its own set, so a subscriber declaring
  `is: {money}` contributed nothing to the publisher's causal set. The
  identical shape with a built-in class reported the violation
  correctly, which is why spot-checks missed it. `@effects(causes: {…})`
  also never learned to intern user classes, so the diagnostic's own
  advice led to a parse error — the feature was unreachable from both
  ends. The docs, spec and published article all claimed this worked.
- **The causal diagnostic and the manifest name user classes** (#345).
  `render_effects` knows only built-ins, so a user class rendered as
  nothing: `can transitively cause  through the bus`.

- **User effect classes resolve across a seed boundary** (#345). Was
  single-seed at v1: `EffectClass::User(i)` indexes the *declaring*
  seed's intern table, and every seed interns from zero, so two seeds
  each declaring one class both used `User(0)` for different names —
  concatenating their items aliased them onto one bit. Rejecting
  cross-seed names avoided that but made a class unusable across the
  boundary it most wants to cross: `money` holds everywhere the money
  goes, and the money goes through `lib/`. The merge now unions the
  name tables and remaps each seed's indices before merging its items.
- **`hale check` on a directory no longer aliases effect classes**
  (#345). `merge_programs` concatenated items while discarding every
  input's `effect_names`, so a `@effects(none: {money})` in one file
  was checked against another file's class 0. It reported `quote`
  reaching `pii` for an assertion that named `money`.

- **An effect-class overflow no longer fails open** (#345). `class_mask`
  saturated to `PURE` past the mask ceiling, and `PURE` means "reaches
  nothing" — so `@effects(none: {overflowed})` silently CERTIFIED a fn
  that called a declared source of that class. The analysis failed open
  in the one direction it must not; every other incompleteness here
  fails closed. `EffectSet` widens u32 → u64 (54 user classes, was 22)
  and declaring past the ceiling is now an error at the `effect NAME;`
  line, where there is a span to point at.
- **The effects manifest names user classes** (#345). The committed
  baseline rendered every user class as `<user effect>`, so two
  distinct classes produced the same line and a real change could diff
  to nothing — in the artifact whose diff *is* the review.
- **A corpus fixture covers the effect-annotation surface** (#345).
  `74-effect-contracts` exercises `@effects`, the `@no_*` sugar,
  `@deterministic`, `@budget`, `@no_panic`, `@phase_effects` and
  `effect NAME;`. The tree-sitter grammar could not parse any of them
  for weeks after they shipped and nothing caught it: the corpus gate
  scans the fixture directory, and no fixture used an effect
  annotation.

- **A user effect-class violation names the class you declared**
  (#345). `EffectClass::as_str` returns a `&'static str`, so a
  `User(i)` — an index into the seed's intern table — had no static
  name to answer with and returned `<user effect>`. Every diagnostic
  that reached for it printed that placeholder, discarding the one
  thing the feature exists to carry: the report now says ``must not
  reach `money` `` where it said ``must not reach `<user effect>` ``.
- **Spec and docs catch up with the effect surface**. `depends:`
  (#330) and user-declared classes (#345) shipped without reaching
  `spec/verification.md`, `spec/tokens.md`, or the `docs/src/effects.md`
  chapter, contrary to the same-commit rule in `CLAUDE.md`. Both are
  now specified, including the boundaries: `depends:` closes over the
  bus graph only, and user classes are single-seed with 22 available.
- **User-declared effect classes** (#345). A program can name its own
  effect classes and have the compiler propagate them:

  ```hale
  effect money;

  @effects(is: {money})
  fn charge(cents: Int) -> Int { ... }

  @effects(none: {money})
  fn price(n: Int) -> Decimal { ... }   // violates if it reaches charge
  ```

  Grounded exactly like a built-in: attached to a leaf and propagated
  by the same engine, with the same witness paths. The compiler owns
  propagation; the program owns classification — the split the stdlib
  registry already has, with a different owner.

  Classes are interned as indices so `EffectClass` stays `Copy`, and
  occupy the free bits above the ten built-ins (22 available in the
  `u32`). **Single-seed at v1**: merging the per-seed tables needs
  index remapping across the merged AST, so a cross-seed class name
  does not resolve.

- **`mode` bodies are walked by the effect analysis** (completeness
  sweep). A `mode` member was never collected into the callgraph, so
  its callees were invisible and `@no_syscall` certified a path
  straight through one. Modes are invoked like methods, so they key
  the same way.

  Found by sweeping every shape a certified fn can reach an effect
  through, now pinned as a standing test: direct stdlib call, free fn,
  handle, `self.` method, interface slot, absent frontier row, `@ffi`
  leaf, two-locus chain, recursive cycle, mode, bus subscriber, and a
  `sync`-bearing form — plus a control that a genuinely pure path
  still certifies.

- **A direct call into a `sync`-bearing form is attributed too**
  (#341). The attribution fired at a locus *holding* such a form but
  not at a call straight into it — backwards, since the direct call is
  the one plainly taking the lock. Synthesized form methods have no
  summary entry, so they arrive unresolved with only a bare name; the
  receiver's type now rides on the call edge, where it was already
  computed and then discarded.

  The reason `block` is attributed at all is worth stating, because
  it is a semantic commitment: **placement is not static.** Once
  placement can be swapped at runtime, whether a mutex ever contends
  is undecidable at compile time, so a certificate reading "never
  blocks, we are single-pool today" would be invalidated by a later
  swap. Conservative is the only sound reading. A form with **no**
  `sync` discipline takes no lock and stays certifiable — pinned by a
  control test.

- **`@shared` is now an effect surface** (#340). Shipping `@shared`
  (#333) sanctioned cross-pool sharing, which made three contracts
  false inside code the compiler had blessed — worse than before, when
  the sharing was accidental and merely warned about. All three now
  hold:

  - **`@no_block`** catches reaching a shared locus. A `sync = …` form
    is a lock and acquiring it waits on another thread, which is what
    `block` means; certifying it as non-blocking was a false hot-path
    certificate.
  - **`@deterministic`** catches a shared read. Another pool can change
    the value between two calls with identical arguments, so the result
    is not a function of the inputs — the same distinction the docs
    draw between `monotonic_ns()` and `time_from_unix(n)`.
  - **`depends:`** reports a `@shared` field as an input channel it
    cannot close over, rather than claiming a completeness the message
    graph cannot give it.

  The class label is approximate for the determinism group — a shared
  read is not literally a clock read, and it wants its own effect
  class. Reporting it under the classes `@deterministic` forbids is
  deliberate in the meantime: an imprecise label on a true finding
  beats a silent false certificate. The witness text says what it
  actually is.

- **Cross-pool aliasing is checked precisely, and `@shared` is gone.**
  The hazard was never "is this shared" — a locus whose mutable state
  lives entirely behind `sync`-bearing forms is safe to reach from
  several pools, because the form orders the accesses. The hazard is
  **unsynchronized** mutable state reachable from two threads, and
  that is directly checkable: a method assigning `self.<field>`, or a
  field whose form carries no `sync` discipline.

  So the report now names the actual problem — *"holds unsynchronized
  mutable state: field `histograms` is a `@form(...)` with no `sync`
  discipline"* — instead of flagging the sharing, and it is silent on
  a properly synchronized registry with no annotation needed. The
  `@shared` annotation added earlier in this cycle is removed: it
  existed to suppress a diagnostic that was too blunt, which was the
  wrong fix.

  The effect attribution that hung off it is inferred from structure
  instead. A locus holding a `sync`-bearing form can take that lock,
  so `@no_block`, `@deterministic` and `depends:` account for it —
  without an annotation, because whether the lock exists is a property
  of the form's own declaration rather than of anyone's intent or of
  how a consumer wires up placement.

- **Aliasing one locus into two differently-placed towers is now
  reported** (#334, #333). F.31 keeps a locus's methods on one pool's
  thread, but reasons per *field declaration*: each holder correctly
  concludes it owns its own field, and nothing related the two
  declarations back to the single object they both name. Two pinned
  workers each doing 100k increments on one shared locus produced
  ~140k of 200k with `hale check` reporting `ok`.

  A **warning, not an error**, deliberately. The sanctioned way to
  share across pools is a `@form(..., sync = ...)` locus, and a plain
  locus whose mutable state sits entirely behind such fields is a
  legitimate design — two applications in a downstream fleet do
  exactly that, with the reasoning written above their placement
  block. Distinguishing those from a real race needs a declared
  shared-locus surface; until that exists, reporting without failing
  the build is the honest position.

  Scoped to the static params-init tower of the main locus, which is
  the domain placement already operates on. Instances created
  dynamically inherit their creator's pool and are not this shape.

- **One topic now has one identity across a seed boundary** (#334,
  closes #332). A qualified topic reference (`relay::Recalled`) kept
  its qualified form while the declaring seed's own `topic Recalled`
  was mangled to `__lib_lib_relay_main_Recalled`, and desugaring
  resolved the two through different paths — the qualified one via
  `BusSubject::canonical()`, which is *syntactic* and returns the last
  path segment. One topic became two subjects in the bus graph.

  Everything downstream of that split is fixed together: a library
  locus subscribing to its own topic now receives an importing
  application's publish (its handler previously never fired); the
  library's subscription is no longer reported dead; and `depends:`
  follows a republisher across a seed, which was explicitly a lower
  bound when that feature shipped.

  The fix canonicalizes qualified topic references in the same pass
  and against the same rename table that already canonicalized
  qualified *type* paths — the bus arm there destructured `{ ty, .. }`
  and never visited the subject.

  **Behaviour change worth noting:** qualified topics now participate
  in orphan detection, which they never did. An unqualified topic in
  the same shape has always warned; five smoke-test binaries in a
  downstream fleet gain "published but has no subscriber" warnings
  that are true of those programs. Warnings only — exit codes are
  unchanged.

- **`hale check` now compares types at call boundaries** (#335). It
  compared types at assignment sites and never at calls, so a
  wrong-typed argument or return reached codegen and surfaced as
  `unsupported in codegen v0: fn \`take\` arg 0 type mismatch` — a
  plain type error wearing a backend limitation's clothes. Arguments
  are now checked for free fns, locus methods, `self.` calls,
  interface-slot calls and builtins, and return types are checked in
  non-fallible fns (only fallible bodies had a check).

  This matters because `hale check` is the documented oracle: AGENTS.md
  tells coding models to iterate against it until it prints `ok`, so
  `ok` has to mean the program compiles.

  Three legal coercions are preserved and pinned by tests, because a
  first cut broke two of them: `Int` → `Float` widening at a call
  (legal at a call, still rejected at an assignment); a satisfying
  locus passed to an interface-typed parameter (nominal comparison
  would reject it, so the structural check owns that case); and
  `StringView` → `String` / `BytesView` → `Bytes` at read-position
  arg sites (F.30b, epoch-checked unpack).

- **`@effects(depends: {…})` — the backward dual of `causes:`** (#330).
  `causes:` exists because a call graph stops at a publish and the bus
  graph continues. Nothing walked it the other way, so an independence
  claim between two parts of a bus graph was unenforceable: a
  dependence routed through one republishing intermediary is invisible
  in every declaration on the depending locus, whose `bus {}` block
  names only the innocent subject it directly subscribes to.

  A complete declaration, like `publish:` and `causes:` — every subject
  that can transitively reach any of the locus's handlers must be
  named, and the violation names the path:

  ```
  declared dependency set violated: `StatedCarry` can transitively
  depend on `SumLookup` through the bus, which its
  `@effects(depends: …)` does not declare. Path: subject `SumLookup` ->
  `Launderer` -> subject `Recalled` -> `StatedCarry`.
  ```

  **Locus-level**, because dependence enters through subscriptions and
  those are declared per-locus; a fn-level `depends:` is a parse error
  rather than a silent no-op. **Opt-in**, on measured grounds: across a
  real application (428 topics, 114 loci), transitivity adds nothing
  beyond the `bus {}` block for 87% of loci, so a mandatory form would
  be redundant far more often than informative.

- **`@budget(alloc_per_call = 0)` now counts string concatenation.** It
  didn't, so a function doing `"x" + a + "y"` — **34 heap allocations**,
  measured — passed a zero-allocation certificate clean. That is a
  fail-open in a contract, which is worse than no contract: it reads as
  proof. Detection is deliberately narrow, requiring an operand to be
  *provably* a String (a literal, or a name whose declared type is
  `String`), because flagging every `i + 1` is the cry-wolf failure the
  allocation pass exists to avoid — which is why this was originally
  deferred to "a type-aware stage". Integer arithmetic is untouched,
  pinned by a control test.

- **String/byte scanners and predicates are now pure reads for LLVM**
  (#322, follow-on). Seven more runtime symbols join the audited
  `memory(read) nounwind willreturn` list: `lotus_str_eq`,
  `lotus_str_starts_with`, `lotus_str_contains`, `lotus_str_index_of`,
  `lotus_bytes_find_byte`, `lotus_bytes_find_byte_raw`,
  `lotus_bytes_at_raw` — all `strcmp` / `strncmp` / `strstr` / `memchr`
  / const-index over `const` pointers, which is the shape the HTTP and
  JSON byte scanners are built from. Two that look identical by name
  are excluded: `lotus_bytes_read_uint` and `_raw` take an
  `int64_t *oob` and write `*oob = 1` on an out-of-bounds read.

- **Indexed byte accessors are now pure reads for LLVM** (#322).
  `lotus_str_len` / `lotus_bytes_len` / `lotus_bytes_data` have carried
  `memory(read) nounwind willreturn` since 2026-07-01 so LICM can hoist
  a length read out of a loop; their indexed siblings
  `lotus_bytes_at` / `lotus_str_byte_at` were missed. A loop-invariant
  `std::bytes::at(b, i)` therefore stayed *inside* the loop body while
  the identically-shaped `len` call in the same program was hoisted to
  `entry:` and its loop folded away — the only difference was the
  attribute. Synthetic upper bound: 1e9 loop-invariant reads, 0.78s →
  0.00s, identical output.

  The exclusions are the substance of the change and are pinned by
  tests. Container accessors do **not** qualify: `lotus_vec_len` /
  `lotus_hashmap_len` / `lotus_ring_buffer_len` read
  concurrently-mutable state, and hoisting a poll loop's length read
  out of the loop is a hang rather than a slowdown;
  `lotus_vec_get` / `lotus_hashmap_get` write through an
  out-parameter; `lotus_lru_get` writes a recency tick on read. Only
  accessors over immutable values (Bytes, String) are eligible.

- **`LOTUS_LTO=thin` selects ThinLTO.** `LOTUS_LTO` previously accepted
  only `1`/`true` and always meant monolithic LTO; it now takes `thin`
  as well, and an unrecognized value is off rather than an error.
  Measured median-of-15, after establishing each bench's noise floor on
  an unchanged binary: `json_parse` (noise 7.3%) **thin -10.9%** vs full
  -6.0%; `locus_instantiation` (noise 10.8%) thin -8.2% vs full -8.6%.
  So thin is the flavor to reach for when you want LTO.

  Still **off by default**, and that isn't changing: either mode takes
  ~1.35-1.43s to link a bench that links in 80ms without LTO, a ~17x
  dev-loop tax. ThinLTO's usual link-time advantage barely shows here
  (1337ms vs 1427ms) because a Hale program is one module plus ~5
  runtime TUs — there is almost nothing to parallelize. Its win is
  cross-module import quality, not build time.

- **Locus birth/dissolve observation probes are branch-gated (#328).**
  They were unconditional opaque calls, so every locus birth and every
  dissolve in every program paid for observation nobody had turned on
  — not because the call is slow, but because LLVM cannot see through
  it and must assume it clobbers memory, which stops optimization
  across the whole instantiation path. They now sit behind the same
  `lotus_obs_live` check the bus publish/deliver probes already used.

  Measured on `locus_instantiation` (100k births, bench precision
  ±1.2%): **20.74 → 17.78 ns per locus**, recovering 76% of a
  regression bisected to v0.11.10 (+7.2%) and v0.11.12 (+15.0%), both
  observation releases. The gated build matches a build with the
  probes deleted outright (17.85 ns), so the dormant cost is now
  essentially zero with observation fully intact — all 18 obs tests,
  including birth/dissolve attribution and late-attach, still pass.

  A residual +5.6% vs v0.11.9 is NOT the probes and is not explained
  yet; #328 stays open for it.

- **A library's `@effects(publish: {…})` contract now survives being
  imported.** Subjects reach the analysis as the import resolver's
  mangled symbol (`__lib_lib_relay_main_Recalled`) while the annotation
  holds the source text (`Recalled`), and the comparison was exact
  string equality — so a publish contract written in a library became
  unsatisfiable the moment anyone imported it. The failure pointed the
  worst way: the library passed `hale check` standalone and failed only
  in the consumer's build, naming a symbol the library author never
  wrote and could not predict, because the mangled name embeds the
  **importer's chosen alias**. An unqualified topic in an effect set now
  matches the trailing segment of a merged symbol; a qualified one still
  matches exactly.

---

## v0.12.0 — the effect system becomes trustworthy (2026-07-30)

Minor bump. `@effects` shipped in v0.11.23, but it could be walked
around in four different ways — and this release is mostly the work of
finding that out and closing them. A contract that reads as verified
and isn't is worse than no contract, so the headline is not a new
feature: it is that the existing one now means what it says.

**The four holes, all found downstream and all closed:**

1. **Calls through a handle were invisible.** `reader.slurp()` was an
   unresolved edge, so `@no_syscall` passed over real I/O. Since
   locus-with-methods is how Hale does I/O, the contracts were largely
   decorative outside free-fn code — and the shape they missed is the
   one the violation diagnostic recommends as the fix.
2. **Seed boundaries stopped the analysis.** `hale check` never
   followed `import`, so a contract violated one seed away was silent.
   Then `hale build` still didn't enforce it after `check` did.
3. **Interface-typed slots resolved to nothing**, an interface having
   no body — so any contract reaching a plug-in implementation through
   a slot was vacuous.
4. **Absent frontier rows failed open.** An unclassified registry entry
   violated every assertion, but a `std::` path with *no row at all*
   contributed nothing, so an unregistered namespace read as pure.

**Also in the effect system:** `println` is syscall-class (writing to
a stream is a `write(2)`); a typo'd `@phase_effects` phase and a
repeated `@budget` dimension are errors instead of silent no-ops; a
publish set can name a qualified topic, so the "only this binary may
publish X" contract is finally expressible; and `hale doc --stdlib`
publishes every function's effect classes, generated from the same
registry the checker queries so the catalogue cannot drift from the
enforcement.

**Why none of this was caught here:** every in-tree effect test
declared its types, topics and loci inline in one seed. The shapes the
corpus never exercised were the only shapes a real multi-seed codebase
has. That is now fixed structurally — cross-seed fixtures are in-tree,
and `crates/hale-corpus` exposes the ~1.2k Hale programs embedded in
test string literals (3× more Hale than the on-disk corpus) to every
corpus-wide property.

**Test-suite and tooling work** landed alongside: a collision-proof
build-path harness (the suite no longer needs `--test-threads=1`,
because the hazard is gone rather than avoided), a committed effect
baseline the CI gate actually checks, the compiler testing itself in
its own language via `hale test`, and observation counters fixed for
payloads carrying a `String` or `Bytes`.

- **Observation counters were missing for any payload containing a
  variable-size field.** `lotus_bus_dispatch_static` probed
  `BUS_PUBLISH` and `BUS_DELIVER` inside its `if (flat)` branch, which
  returns — so a payload carrying a `String` or `Bytes` counted
  nothing and its topic never entered the observation manifest at all.
  Deliveries were correct and cross-process NET edges paired with real
  latencies; only the probes were absent, which is what made it look
  like a counter bug rather than a missing call.
  The class is variable-size storage, not one type (reproduces for
  `String` and `Bytes` alike). Worth recording how it was found: it was
  first reported — and first investigated here — as a *cross-seed*
  bug, because in the reporting codebase every shared topic also
  carried a String, so the two properties were perfectly correlated.
  A cross-seed topic with a scalars-only payload counts correctly and
  is what exonerated the seed boundary. The in-tree fixture now runs
  that six-way differential.
- **`hale build` enforces cross-seed effect assertions, like `hale
  check` already did.** Only the check path carried the import rename
  table, so a contract violated one seed away compiled, linked and
  shipped. A downstream fleet gates on `build` across 109 binaries —
  "it built" must not be weaker than "it checked" on a contract the
  compiler already knows how to evaluate. All four analysis paths
  (`build`, `run` file, `run` dir, test-compile) now pass the table.
- **An app's effects manifest describes the app, not its imports.**
  Once `check` resolved imports, every imported fn emitted a row under
  its merged symbol — one downstream fleet's committed baseline went
  from 1,319 rows to 8,021, and 131 of one app's 151 rows were mangled
  names. That defeats the artifact: an effect regression is meant to
  be a one-line diff in review. Merged symbols are excluded; a
  library's rows come from checking the library, and what an import
  contributes here is already folded into the caller's `does={…}`.
- **Every diagnostic renders the alias spelling**, not only effect
  witnesses. The no-locus-return rule was naming
  `__lib_lib_a_b_OrderBook.query_bulk` — a symbol appearing nowhere in
  the user's program.
- **A framework keyword is legal as a struct-literal field name.**
  `tier` was declarable, readable and assignable, but `Row { tier: 1 }`
  failed with `expected ;, got LBrace` pointing at `Row {` and never
  naming `tier`. `parse_struct_init` already accepted these keywords;
  the struct-literal *lookahead* did not, so the literal fell through
  to "expression followed by a block". The two now agree.
- **Advisories from an imported seed are not reported on the
  importer.** Making `check` resolve imports is what exposes
  cross-seed errors — and it also drags every advisory lint in every
  imported seed into the target's output. Checking one downstream app
  began reporting 47 hot-path warnings from library code, and since
  `hale verify` gates on ANY finding, 10 of 12 apps that passed it
  started failing. A gate that goes red for library internals you
  cannot edit from there is a gate people switch off. Advisories are
  now reported where they are actionable — when that seed is the
  check target — and **errors are never filtered**, wherever they
  originate.
- **Observation shm segments no longer leak on SIGKILL (downstream handoff).** A clean exit unlinks the segment and its
  registration via `atexit`; a SIGKILLed process by definition runs no
  handler. A downstream fleet measured **442 stale segments, 245 MB of host
  tmpfs** from one fleet run, because `docker stop` never reaches
  `dissolve` and their compose bind-mounts `/dev/shm`. A dead emitter
  cannot clean up after itself, so the next observed process to start
  now sweeps segments and registrations belonging to dead pids. It
  skips anything alive — blinding a running observer would be far
  worse than leaving a file behind. (Our own suite had accumulated 69
  of these on a dev box, so this was never only a downstream problem.)
- **A cross-seed observation fixture is in-tree.** A downstream handoff reported
  `CT_PUBLISHED` always 0 for a topic declared in an imported seed,
  and correctly identified why we could not see it: every in-tree obs
  test declares its topics inline. The bug did **not** reproduce at
  their measured tree in either shape tried — in-process with a local
  subscriber, and transport-bound with none — so this ships as a
  standing guard with an inline control rather than a fix, and the
  open question goes back to them with what was ruled out.
- **Effect assertions now resolve through an F.20 interface-typed
  slot (downstream handoff).** `self.sink.emit()` where `sink`'s
  declared type is an interface resolved to nothing — an interface
  has no body — so every effect behind the slot was invisible. The
  concrete locus in the slot's default is what actually runs, and the
  witness now reads `certified -> Manifest::reach ->
  LoudEmitter::emit`. This is the plug-in-implementation design:
  consumers see
  only the abstract type, so a contract reaching a venue surface
  through a slot was vacuous.
- **A publish set can name a qualified topic (downstream handoff).**
  `@effects(publish: {t::SharedTopic})` was a parse error, so the
  contract could only name app-local topics — and the contract worth
  having most, "this binary is the only one permitted to publish X",
  was the one it could not state. Two halves had to agree: the parser
  now accepts `alias::Name` in an effects set, and the publish SITE
  records a qualified subject instead of writing it off as a computed
  one (which had made every shared-topic publish unprovable).
- **Effect assertions were silently vacuous across a seed boundary
  (downstream handoff), and `hale check` rejected cross-seed types it
  could not see (P2). Same root cause.** `hale check` collected only
  the target directory's own `.hl` files and never followed
  `import` — so an imported seed's bodies were absent from the
  program the analysis walked, and a cross-seed payload type rendered
  as `?`. Separately, a call written `alias::name` reaches the
  callgraph as a qualified path while the imported decl was merged
  under a mangled symbol, so even with bodies present the two never
  met. Codegen had the rename table all along; the analysis phases
  did not. `check` now resolves imports the way `build` and `run` do,
  and `Bundle` carries the table so the callgraph links across the
  boundary. Diagnostics render the alias spelling (`p::far_syscall`),
  never the merged symbol.
- Worth stating plainly why this survived: **every in-tree effect
  test declares its types, topics and loci inline in one seed.** The
  one shape the corpus never exercised is the only shape a real
  multi-seed codebase has. A cross-seed fixture now lives in-tree.
- **A repeated `@budget` dimension silently kept the last value.**
  `@budget(alloc_per_call = 0, alloc_per_call = 5)` enforced **5** —
  you wrote a zero-alloc certificate and got a ceiling of five, with
  nothing said. Rejected now: whichever way precedence fell would be
  a guess, and the annotation is simply ambiguous. Distinct
  dimensions in one clause are untouched.
- **A typo'd `@phase_effects` phase was silently ignored.**
  `@phase_effects(disolve: {})` typechecked clean and checked
  nothing — you declared a contract and got no contract and no
  diagnostic. It is now an error naming the bad phase and listing
  what the locus actually has. The six lifecycle names stay legal
  whether the hook is written out or not, so the canonical
  `@phase_effects(birth: {alloc}, run: {})` line still works on a
  locus with only `params`.
- **Documented the annotation parameters that were only implied.**
  The book stated that an omitted phase is unconstrained but left
  `run: {}` — the opposite meaning, and the load-bearing one — to be
  inferred from an example. Both are now spelled out in a table,
  along with what a phase name may be, and the gotcha that a
  publishing handler needs `{publish, alloc}` because building the
  payload allocates.
- **The effect catalogue is published, and generated.**
  `hale doc --stdlib` now prints each function's effect classes beside
  its signature (283 of them; the 57 without are locus/type paths that
  legitimately have no row), and `--json` carries an `effects` field.
  The registry has held an `EffectSet` per fn since #265 and the doc
  generator was already walking those entries to print signatures
  while ignoring the column next to them — so the classification the
  checker enforces was invisible to anyone reading the docs. Derived,
  not transcribed: a hand-written table of 300+ rows would drift, and
  this repo has been bitten by exactly that three times now.
- The book gains a **per-class reference** — what each of the ten
  classes covers, why `println` is a `syscall`, and the distinction
  that makes `@deterministic` useful rather than merely restrictive
  (`time_from_unix(n)` is pure, `monotonic_ns()` is not). Every
  example in it was verified against the compiler.
- **The book documents the effect system.**
  `docs/src/verification.md` — the chapter named Verification — had
  zero mentions of effects; the whole surface lived in
  `systems/performance.md`, which is a placement problem as much as a
  coverage one, since `@no_syscall` and `@phase_effects` are
  correctness contracts. Two surfaces were in the spec and nowhere in
  the book at all: the **effects manifest**
  (`--dump-effects-manifest` / `--check-effects-manifest`, the CI
  gate) and **bus causality** (`@effects(causes: …)`). Both are now
  taught, with real compiler output rather than paraphrase, and the
  hot-path angle stays cross-linked to Performance instead of
  duplicated.
- **Effect assertions were blind to calls made through a handle —
  fixed (GH #265 soundness).** `@no_syscall` and the rest resolved
  free fns, `self.m()`, and `std::ns::fn(…)` path calls, but a call
  on a *value* (`reader.slurp()`, `resolver.get(…)`) was reduced to
  an unresolved edge carrying only the bare method name. The
  callgraph never reached the body, the effect contributed nothing,
  and the assertion passed. Since locus-with-methods is the
  idiomatic way to do I/O in Hale, this made the contracts largely
  decorative outside free-fn code — and the shape it missed is the
  same one the violation diagnostic recommends as the fix. Moving an
  effect behind a locus you still call does not make it unreachable.
  The analysis now resolves the receiver's declared type (including
  from the struct literal, `let r = Reader { … }`, which is the
  common shape) and walks into the method body.
- **The Hale-source stdlib is visible to the analyzer
  (new `crates/hale-stdlib`).** Part of the standard library is
  written in Hale, and those `.hl` modules lived in a `const` inside
  `hale-codegen` — *downstream* of `hale-types`, so the effect
  analysis structurally could not read them. They are now their own
  upstream crate that both the compiler and the analyzer consume, so
  the effects of `std::cli::Resolver`, `std::log::Logger`,
  `std::io::file::File` and friends are **inferred from their
  bodies** rather than hand-transcribed into a table that drifts.
  Witness paths through them render in the public spelling
  (`std::cli::Resolver::get`), not the internal mangled name.
- **An absent frontier row now fails closed, like an unclassified
  one.** These were asymmetric: an unclassified registry entry
  violated every assertion, but a `std::` path with *no row at all*
  short-circuited to "no effect", so an entire unregistered
  namespace read as pure. Absent and unknown are the same claim, and
  neither can be certified. (`std::ts`/`std::shm` were the instance
  fixed in v0.11.24; this is the class.)
- **`println` / `print` / `eprintln` / `eprint` are syscall-class.**
  They are language builtins, not `std::` paths, so they sat outside
  the frontier entirely — while the diagnostic emitted for
  `std::io::fs::*` described the syscall class as covering "stdio".
  Writing to a stream is a `write(2)`: it can block, and a hot-path
  certificate that permits it is not certifying what it claims.
- **The registry/dispatch parity test knows all three lowering
  structures.** Its first cut scraped only `match` arms and passed
  partly by accident: `PATH_RENAMES` rows are also `["std", …]`
  literals and were being counted as arms. Renames are now counted
  deliberately — they *are* a lowering — and a new
  `rename_targets_exist` check asserts every rename points at a name
  the Hale-source stdlib actually declares, which the accidental
  version could not do.

  hazard is gone rather than avoided.** ~131 codegen test files wrote
  their compiled binary to a temp path with no uniquifier — most of
  them `temp_dir()/lotus_test_{name}`, a template eleven files shared
  verbatim (nine more shared `lotus_{name}`). Nothing made those
  distinct; the suite passed only because the `name` arguments
  happened not to overlap. One `build_and_run("basic", …)` in the
  wrong file and two tests write and exec the same path.
  `harness::unique_bin` (pid + process-local counter) now supplies
  every build-artifact path, and `harness_paths_are_unique.rs` fails
  the build if a test rolls its own. `harness::free_port()` replaces
  the hand-maintained 57xxx/47xxx port registry spread across 159
  files (`9876` was already used six times).
- **Two docs disagreed about that hazard and neither was right.**
  `CLAUDE.md` mandated the serial flag because of it; `tests.yml`
  claimed nextest's process-per-test made the shared paths safe.
  Process isolation is not filesystem isolation — two processes
  writing one path are *more* concurrent than two threads, not less.
  Both are corrected, and the guidance is now "run it in parallel"
  because that is finally true.
- ~115 copies of `build_and_run` (104 textually distinct) had drifted
  almost entirely in where they put the binary; at least three had
  independently rediscovered the pid+counter fix and left a comment
  about it. That is a missing invariant, not a missing convention.

- **The test corpus was 3× bigger than anything tested it
  (`crates/hale-corpus`).** Every corpus-wide property — `fmt`
  idempotence, effect totality, the parse sweep — walked the on-disk
  fixtures and stdlib: 7,032 lines. The suite also carries **1,391
  Hale programs embedded in Rust string literals, 21,621 lines**,
  invisible to all of them. That is where the interesting code is:
  fixtures are written to be tidy examples, embedded programs are
  written to hit feature intersections and regressions. One provider
  now yields both, and the properties consume it.
- **Two new whole-corpus properties**: the analysis never panics on
  a parseable program (an ICE is never the right answer to a bad
  program), and it is deterministic run-to-run (non-determinism is
  what makes an effects manifest diff-noisy, and a noisy gate gets
  switched off).
- **Frontier completeness is now asked from the corpus side.** The
  old phrasing — "no reachable stdlib call is UNCLASSIFIED" — could
  only see paths that had a registry row, so an absent namespace was
  invisible to the check meant to guarantee coverage. Asking instead
  "every `std::` namespace the corpus calls must be registered"
  closes that, and immediately found `std::io::mirror` (the
  `MirrorRing` primitives) unclassified. Now registered.
- **The `#265` effect gate finally guards something.**
  `--check-effects-manifest` shipped as a CI gate exercised on two
  toy inputs, with no baseline committed for this repo.
  `.effects-baseline/corpus.effects` is now that baseline — the
  inferred effect set of every function in every in-tree example —
  with `scripts/effects-baseline.sh` to regenerate and a test that
  fails on drift.
- **The manifest covers lifecycle hooks.** It listed free fns and
  locus `fn`s only, so for most programs it emitted a single line:
  in Hale the work lives in `birth` / `run` / `dissolve` and in bus
  handlers. A fingerprint blind to `run()` cannot notice a handler
  that starts doing filesystem I/O, which is the regression the gate
  exists to catch. 157 effect rows across 86 programs, up from ~86
  near-empty ones.
- The registry/dispatch parity check understands **whole-namespace
  dispatch arms** (`["std", "io", "mirror", op]`), which the literal
  scraper counted as zero coverage.


- **`err.kind` did not compile.** Reading a stdlib error payload in
  an `or` block — `parse_int(s) or { println(err.kind); -1 }`, a
  shape `docs/src/everyday/http.md` and `spec/decisions.md` both
  show — failed with `no field 'kind' on 'ParseError'`. The stdlib
  error types (IoError, ParseError, CryptoError, …) were injected
  into scope only when a program used `@form` machinery, which has
  nothing to do with reading an error field. Now injected
  unconditionally; a user declaration still wins.
- **The compiler now tests itself in its own language.** `hale test`
  shipped in the same binary as `hale build`, and the repo contained
  four `*_test.hl` files — all of them fixtures for testing the
  runner. `tests/hale/` is the real suite, run by `hale test` and
  wired into the workspace run. The first two files replace nine
  Rust tests in `stdlib_str.rs`.
- The move is not cosmetic. The expectation stops being transcribed
  (and gets stricter: `assert_eq_int(n, 42)` rejects what
  `stdout.contains("a=42")` accepts, since that also passes on
  `a=421`), and the program gets **typechecked** —
  `build_executable`, which every Rust codegen test calls, parses
  and lowers but never runs the checker. Converting the first test
  is what surfaced the `err.kind` bug above.
- **Measured that gap:** 8.5% of the ~1,000 programs embedded in
  codegen tests do not pass `hale check` while compiling and running
  fine. Some is deliberate (a codegen test may lower a shape the
  checker rejects), so this ships a guard on the part that should
  hold unconditionally — the on-disk example corpus typechecks
  clean — rather than a blanket assertion.

---

## v0.11.24 — stdlib registry/dispatch parity enforced; effect-classification hole closed (2026-07-29)

- **Registry/dispatch parity is enforced (R2 completion), and it
  found real drift.** The R2 refactor made `stdlib_surface` the
  single table for the stdlib surface, but the *lowering* stayed in
  hand-written `["std", ns, fn]` match arms with nothing forcing the
  two to agree — the four-parallel-structures problem, only
  half-solved. `stdlib_registry_parity.rs` now asserts mutual
  coverage in both directions (with a non-vacuity guard, and
  prefix-pattern arms like `bytes::read_*` understood rather than
  hand-listed). What it caught:
  - **`std::ts` and `std::shm` were absent from the registry
    entirely** — real namespaces, called from stdlib `.hl`, typing
    as `Ty::Unknown` (no arity/fallibility checking) **and escaping
    effect classification**, which would have let a `@no_syscall`
    fn call them unchallenged. Now registered and classified.
  - **`std::io::fs::list_dir` and `std::str::can_parse_decimal`
    were in the typecheck surface with no lowering** — they passed
    `hale check` and then failed at codegen with
    `unsupported in codegen v0`. (The spec already admitted
    `list_dir` was "listed in older notes but not dispatched".)
    Both removed; they now fail cleanly at typecheck with a
    did-you-mean.
  - `std::io::udp::Reader` was missing from `LOCUS_PATHS`.

## v0.11.23 — effect assertions (GH #265, complete) + the #265/#262 refactor substrate (2026-07-29)

- **GH #265: effect assertions — one surface, one engine, one
  classified frontier.** `@budget`'s discipline generalized from
  allocation *count* to effect *classes*, delivered as a system
  rather than a family of flags.
  - **The general form**: `@effects(none: {syscall, block, time,
    entropy, env, ffi, publish, spawn, recursion})` and
    `@effects(publish: {Topic, …})` (the allowed publish set —
    exact, because the topic set is closed). The `@no_syscall` /
    `@no_block` / `@no_ffi` / `@no_publish` / `@no_spawn` /
    `@no_recursion` / `@deterministic` family is **documented
    sugar**, desugared at parse time so the checker has one shape to
    interpret and a flag can never drift from the general form. The
    general form also expresses contracts the sugar can't name —
    `@effects(none: {time})` forbids the clock while allowing
    jitter.
  - **The frontier is classified**: all 327 stdlib registry entries
    carry an `EffectSet`, zero unclassified residue (pinned by a
    test). Reading an effect source is distinguished from operating
    on a supplied value — `time_from_unix(n)` is deterministic,
    `monotonic_ns()` is not. An unclassified entry violates every
    assertion by construction, so incompleteness can't silently
    pass.
  - **Syntactic effects**: `publish` and `spawn` are carried by
    `Topic <- v` and `Child { … }`, not by any call, so the summary
    now records effect *sites* alongside allocation sites.
  - **Diagnostics carry the witness path** — the call chain from the
    asserting root to the offending leaf, which `@budget`'s fixpoint
    structurally could not produce.
  - **Placement-implied contracts — the assertion you don't write.**
    A handler on a `cooperative(pool = X) where async_io` locus that
    reaches a blocking call stalls every other locus on that pool;
    the placement already declared the intent, so the compiler warns
    with **no annotation at all**, naming the chain and both fixes.
    Writing `@no_block` upgrades it to an enforced error and
    suppresses the advisory. This is the class of bug that shipped
    as a downstream latency mystery (a sleeping handler holding an
    engine pool), now visible at compile time.
  - Docs: `spec/verification.md` rewritten as one systematic entry,
    `spec/tokens.md` gains an annotation inventory,
    `spec/styleguide.md`'s enforcement ladder gains the `@effects`
    and placement-implied tiers, `AGENTS.md` updated, and
    `docs/src/systems/performance.md` documents the surface.

- **GH #265 COMPLETE — the effect-assertion system, all seven build
  steps.** The remaining phases land together on the substrate the
  earlier ones established:
  - **Quantitative budgets** (step 5): `@budget(stack_bytes = N,
    block_points = N, publish = N, fanout = N)`, composable in one
    clause with `alloc_per_call`. `stack_bytes` is a DAG longest-path
    over estimated frames (acyclicity is the precondition — recursion
    reports unbounded); `fanout` counts transitive subscriber
    deliveries off the bus graph, the amplification property no
    per-fn count reveals; `@budget(publish = 1)` **is** the
    exactly-once-reply contract the issue sketched as `@replies`,
    falling out as a count rather than a bespoke analysis.
  - **Phase-indexed effects** (step 6): `@phase_effects(birth:
    {alloc}, run: {})` on a locus — the DO-178 "no dynamic memory
    after initialization" discipline stated directly rather than
    assembled from two unrelated flags. `alloc` became a first-class
    `EffectClass` (site-measured, like publish/spawn) so a phase can
    name it.
  - **`@no_panic`**: disposition coverage — explicit `violate`, an
    `or raise` that propagates rather than handles, or a trapping
    index. Deliberately not an effect class: it is a syntactic
    property of a body, not a query over the frontier.
  - **The conformance loop** (step 7): compile programs carrying
    assertions, run them, and sample the runtime's own counters
    around the certified call. A fn certified `@no_syscall` that
    performs a syscall is a **caught soundness bug in the analysis
    itself** — the defect class that expectation-based testing
    structurally cannot find. A negative control proves the oracle
    detects effects when they genuinely happen, so the checks can
    never pass vacuously.
  - **The `.hale.effects` manifest** (step 7): declared contracts in
    a stable sorted format alongside `.hale.topo`, so an effect
    regression shows up as a one-line diff in review.

  34 effect tests across four suites; spec, book, and the annotation
  inventory updated.

- **GH #265 frontier items — the deferred set, delivered.**
  - **Cross-actor causality** (`@effects(causes: {…})`): the call
    graph stops at a publish, the bus graph continues. Publishing to
    a subject whose subscriber writes a file *causes* a syscall, and
    the diagnostic names the path (`Api::handle -> subject Orders ->
    Audit::on_order`). Checkable only because Hale's message graph is
    declared over a closed topic set.
  - **Supervision coverage** (`@supervised`): every locus in a
    subtree must have a failure policy in scope; uncovered loci are
    named. A tree walk over the declared ownership tree.
  - **Coarse secret taint** (`@secret` params): a secret must not
    reach a bus publish or a log/file sink. Parameter-granular by
    design — the honest reach, and enough to catch a key in a log
    line.
  - **Inferred effect sets + symbolic cost**: `infer_effects`
    computes any fn's transitive effect set with no declaration
    (feeding causality and the manifest's inferred column);
    `cost_expression` renders a structural `O(n^k)` estimate —
    explicitly not WCET.

- **GH #265: the effect manifest is wired end-to-end.**
  `hale check --dump-effects-manifest` emits the behavioural
  fingerprint — declared contracts **plus inferred effect sets**
  (`does={syscall,publish}`) for every fn, stable-sorted;
  `--check-effects-manifest <baseline>` diffs against a committed
  copy and fails the build on change, naming the fn and the gained
  effect. That catches what annotations can't: a handler that
  quietly starts doing filesystem I/O is a one-line review diff even
  though nothing was annotated. Plus a **corpus-wide conformance
  sweep** asserting, across every in-tree `.hl` program, that no
  reachable stdlib call is unclassified, that inference is
  deterministic, and that declared contracts hold.

## v0.11.22 — iris handoff-8: adapter ingest lit + the refactor batch (2026-07-29)

- **The adapter ingest path carries the full observation trio**
  (iris handoff-8 P21 — "the last dark ingestion path"). The
  Hale-owned-wire ingest (`std::bus::__local_dispatch`) emitted no
  NET_DELIVER and its fanout no per-target BUS_DELIVER — a
  dynamically-subscribed plane was invisible (zero `net<`, zero
  dlv) while the same segment's statically-configured listens
  paired fully. The inbound wrapper now peels the magic-guarded
  obs wire header when present (which also FIXES headered
  datagrams reaching a Hale adapter — they previously failed
  deserialization), emits NET_DELIVER echoing the wire
  (origin, seq), and plain `dispatch_wire` gains the per-target
  BUS_DELIVER its keyed sibling got in v0.11.18. Pinned by a
  two-process producer→adapter-consumer test asserting paired
  net records, attributed deliveries, and published == 0.

- iris handoff-8 P20 (remote-only publishes show CT_PUBLISHED=0)
  **did not reproduce** on any of four flavors (adapter binding,
  udp config, framed transport, keyed) — the counter is correct on
  all; likely a carry from the pre-v0.11.15 keyed-probe-gap era.
  The keyed+udp shape is pinned as a regression test.

- **Refactor batch (R1/R2/R3/R4/R6) — the substrate for #265 and
  #262.** Behavior-neutral; full workspace suite + the dispatch
  bench gate every piece.
  - *R1*: `hale-types::callgraph` — the shared, witness-path-
    preserving call-graph engine (extracted verbatim from
    `budget_check`'s DFS, which is its first ported customer with
    byte-identical diagnostics); `witness_path` renders the
    `root -> mid -> leaf [alloc]` chains #265's diagnostics
    specify. `PurityKey` unified onto `alloc_summary::FnKey`.
  - *R2*: `stdlib_surface` is now the stdlib registry — structured
    per-fn entries carrying an `EffectSet` column (UNCLASSIFIED
    until #265 classifies the frontier) with `effects_for(path)`
    as the query hook.
  - *R3*: the deployment arrangement (placement, NUMA nodes,
    pools, async_io set, pinned/pool locus types) is one
    `DeploymentPlan` value on Cx instead of seven loose fields —
    #262's seed artifact.
  - *R4*: `lotus_bus_post_entry` — the one per-entry
    mailbox/coop_pool/queue post, replacing 9 of 11 hand-copies
    across dispatch flavors (the "fixed one flavor, missed
    siblings" class from P5..P17); the two exceptions (_st
    fast path, direct same-thread call) are annotated. Dispatch
    bench unchanged (~173µs dormant).
  - *R6*: one obs-segment reader for the three test files that
    each carried a drifting copy of the PROTOCOL v0.1 decode; the
    protocol.h-vendored decodes live in exactly one place.

## v0.11.21 — iris handoff-7: BUS w1 packed per PROTOCOL §8 (2026-07-29)

- **BUS record w1 packed per PROTOCOL §8** (iris handoff-7 — the
  one-liner). `BUS_PUBLISH`/`BUS_DELIVER` emitted `locus` in bits
  0..19 with seq shifted high; the protocol (and every consumer)
  puts **locus in bits 44..63, seq low**. Attribution has been
  computed correctly since handoff-6 and packed unreadably —
  consumers decoded `w1 >> 44` and read the top of a small seq
  → 0. Both probes now pack `locus:20 << 44 | seq:44`, and the
  contract tests vendor protocol.h's decode
  (`obs_bus_locus = w1 >> 44`) instead of the emitter's own
  layout, closing the self-consistent-but-wrong loophole that
  kept them green.

## v0.11.20 — iris handoff-6: constructor-resolved obs gate, marked adapter inbound (2026-07-29)

iris handoff-6 (P17/P19 — attribution in the field).

- **The obs gate flag is resolved in a constructor** (P19). The
  fn-entry hoist of `lotus_obs_live` claimed the flag was final
  before any user publish; it was set at the FIRST PROBE (a locus
  birth inside main's body), so a publish lowered into `fn main`
  itself snapshotted a stale dormant flag forever. `LOTUS_OBS` is
  now read in a `__attribute__((constructor))` before `main` — the
  flag is genuinely process-constant, which is the exact property
  the hoist's soundness requires. The field's ordering shape
  (publishers deep in steady-state loops, observer attaching much
  later) is pinned in `obs_fleet_contract.rs`.

- **The adapter inbound path no longer stamps `locus=0` publishes**
  (the fleet's remaining attribution zero). `std::bus::
  __local_dispatch` — the Hale-owned-wire ingest adapters use —
  called the UNMARKED `lotus_bus_dispatch_wire`: every inbound
  message recorded a spurious unattributed BUS_PUBLISH and
  inflated the published counter (measured: 2 genuine publishes +
  2 adapter relays = `pub=4` on v0.11.19; now exactly 2, all
  attributed). It now lowers to `lotus_bus_dispatch_wire_inbound`,
  which brackets the dispatch with the P15 redispatch marking —
  deliveries deliver, publishes publish.

## v0.11.19 — Crumb batch-5: sleep parks on async_io, stdlib builtin-namespace README (2026-07-28)

Crumb batch-5 (UPSTREAM5.md).

- **`std::time::sleep` parks on `async_io` pools** (batch-5
  item 1). Sleep blocked the pool's single worker in nanosleep, so
  N sleeping coros serialized (400/800/1200ms instead of all
  waking at ~400ms) and one sleeping handler held the pool against
  unrelated requests (a JS `await sleep(400)` turned an unrelated
  `GET /` into 329ms) — invisible until concurrency made it
  latency. On an async_io pool sleep now parks the coroutine on a
  deadline (timer-only park, no fd; the drain loop's existing
  deadline sweep services it), yielding the worker for the full
  duration. Off async pools the classic chunked-nanosleep +
  per-slice bus drain is unchanged. All three repro waiters wake
  at ~401ms; regression-locked.

- **`runtime/stdlib/README.md`** (batch-5 item 2): the stdlib
  directory now documents the namespaces that exist only as
  compiler builtins (`std::crypto`, `std::str`, `std::math`,
  `std::time`, `std::text::base64`, …) and have no `.hl` file —
  the exact search that finds every other module found nothing for
  them, which twice read downstream as "doesn't exist".

## v0.11.18 — Crumb batch-4 + iris handoff-5: teardown join order, direct-flavor obs, replay heartbeat, json keys (2026-07-28)

Crumb batch-4 (UPSTREAM4.md) + iris handoff-5, delivered together.

- **A bus subscription on `main` no longer inverts the teardown
  join order** (Crumb 4-1). The GH #253 delivery contract held on
  the eager path but not the deferred one: a `subscribe` on the
  main locus made it long-lived → deferred, and the deferred flush
  tore the parent down (cascading its subscriber fields' dissolves)
  BEFORE joining its own pinned children — every in-flight result
  silently dropped, exit 0. A deferred parent's own pinned entries
  are now re-ordered after its own frame entry so the reverse-order
  flush joins + drains them while every subscriber field is alive
  (identical semantics to the eager path). Regression-locked with
  both shapes.

- **The fully-devirtualized direct dispatch now carries obs
  probes** (iris P17c). The single-quiet-subscriber same-thread
  flavor (baked-handler bucket walk + the multi-handler C sibling)
  emitted no probes at all — its subjects never registered,
  counted, or produced BUS records; a fleet path on this flavor was
  invisible to observation. Both direct flavors now publish once +
  deliver per matched target with full locus attribution. Dormant
  cost is BETTER than before: the `lotus_obs_live` gate is now
  checked once per function entry (sound — the flag is final
  before any user publish can run), LLVM hoists the branch, and
  the dormant `bus_dispatch` bench lands at ~173µs (vs the 193.8µs
  baseline and v0.11.17's 192µs).

- **Observer attach no longer needs probe traffic** (iris P18).
  The 0→1 birth replay was driven from inside probes, so a
  probe-quiet process (main parked in a read loop, pinned raw-fd
  readers, direct-flavor hot paths) never replayed its loci —
  segment registered, zero records, "silent" to the consumer. A
  detached heartbeat thread (spawned only under `LOTUS_OBS=1`)
  drives the replay check every 250ms, bounding replay latency
  after attach at ~250ms with no probe traffic at all.

- **`std::json::find_field_raw` matches key positions, not key
  text** (Crumb 4-2). The old lookup was
  `index_of(json, "\"name\"")` — an earlier string VALUE repeating
  a later key's name shadowed that key (on a real npm packument,
  12 of 35 version keys were invisible). Rebuilt on the single-pass
  object cursor: top-level members only, depth-aware, string-safe;
  the documented re-feed-the-substring chaining contract is now
  actually enforced.

- **`std::json::obj_key_string(it, json) -> String`** (Crumb 4-4):
  the key-side sibling of `obj_value_string` for unknown-key
  iteration (a packument's `versions`, a `dependencies` map),
  including the escape decoding that hand-slicing
  `key_start..key_end` silently skips.

- Crumb 4-3 (hash functions) closed as already-shipped:
  `std::crypto::sha1/sha256/sha512/hmac_sha256/hmac_sha512/crc32`
  have existed since v0.8.0 (spec § std::crypto; book
  `everyday/crypto.md`); an npm `sha512-<base64>` integrity check
  is `std::text::base64::encode(std::crypto::sha512(tarball))`.

## v0.11.17 — publish hot path back to baseline (obs note branch-gated) (2026-07-28)

- **perf: publish hot path back to baseline — the obs
  publisher-attribution note is now branch-gated.** v0.11.13's
  iris P10 fix made `lower_send` emit an UNCONDITIONAL
  `lotus_obs_note_publisher` call (a call + TLS store) before
  every `<-`, violating the "dormant = one predictable branch"
  observation cost contract: ~0.8ns on a ~1.9ns devirtualized
  publish, +39% on the `bus_dispatch` microbench and +34% on
  `stream_aggregator`, shipped unnoticed in v0.11.13–v0.11.16.
  Codegen now branches on `lotus_obs_note_publisher_wanted` (an
  i32 the obs TU sets when `LOTUS_OBS` resolves enabled), so an
  unobserved publish pays one predictable load+branch — LLVM
  hoists the check and the dormant publish loop is
  instruction-identical to the pre-v0.11.13 one. Bench restored:
  `bus_dispatch` 268→192µs, `stream_aggregator` ~600→~440µs
  (baselines met). Attribution under `LOTUS_OBS=1` is unchanged
  (the flag is set by the first probe — always a locus birth —
  before any publish).

## v0.11.16 — Crumb batch-3: main-return teardown fix, takeover_raw, Duration scalars (2026-07-28)

Crumb batch-3 handoff (UPSTREAM3.md): two design asks, one
codegen bug with a second symptom, one paper cut.

- **`fn main`'s `return f()` no longer tears the runtime down
  before calling `f`** (batch-3 items 3+4). `lower_return`'s
  in_main path emitted the full teardown — cooperative-pool
  shutdown, dissolve flush, arena destroy, bus-queue destroy —
  BEFORE lowering the return expression. A main written as
  `return cmd_run();`, where `cmd_run` instantiates the main
  locus, executed the whole program in a torn-down world:
  `lotus_coop_pool_lookup` returned NULL (subscribers registered
  pool-less; pool-placed children's `run()` forced onto the
  synchronous inline path — item 4's surprise), and the first
  bus enqueue wrote into the freed queue (item 3's SIGSEGV; in
  small heaps a silent drop, which is why minimal repros looked
  green). The return value is spec-enforced `Int`, so evaluation
  is now hoisted before teardown with no lifetime hazard.
  Regression test asserts on delivery output, not just exit
  status.

- **`std::http` raw takeover — `Response { takeover_raw: true }`**
  (batch-3 item 1). The Server writes NOTHING — no status line,
  no headers — and fd ownership transfers exactly as with
  `takeover`. The deferred-response shape: the handler returns
  before the answer exists (a promise resolved later, a bus
  reply), and whoever ends up owning the fd writes the entire
  response, status line included, via the raw-fd surface. Also
  covers CONNECT tunnels and server-initiated protocols.
  `status`/`headers`/`body` are ignored; same recv-timeout and
  fd-leak caveats as `takeover`; takes precedence if both set.

- **Duration scalar arithmetic** (batch-3 item 5). `Int *
  Duration` (either order) scales the interval; `Duration / Int`
  divides it — so a runtime-computed delay is `ms * 1ms` instead
  of an O(ms/100) tiered sleep loop. `Duration * Duration` (and
  `/`, `%`) is now rejected with a real diagnostic pointing at
  the scalar forms (it previously died on a spanless codegen
  catch-all).

- **One worker per named cooperative pool is now a spec-level
  promise** (batch-3 item 2). `spec/runtime.md` § `where
  async_io`: every named cooperative pool has exactly one OS
  worker thread for the program's lifetime; all `run()`s, bus
  handlers, and coro resumes for the pool's loci execute on that
  thread (coros never migrate). Thread-affine C libraries (JS
  engines, SQLite serialized mode, GUI toolkits) placed on a
  named pool are entered from one thread by construction, and
  citable as such.

## v0.11.15 — iris observation edge-emission fixes (topic id, publish counter, wire opt-in) (2026-07-28)

Three regressions the iris fleet caught in the v0.11.14 field test
(handoff 4), plus the acceptance test that would have caught them.

- **NET records carry the topic id (P14).** `NET_SEND` /
  `NET_DELIVER` hardcoded their record id field to 0. For those
  record kinds the id field IS the topic id — the consumer's join
  key onto the fused topic row — so no NET event could be
  associated with any topic and cross-process edges were
  structurally impossible regardless of `(origin, seq)`
  correctness. The probes now resolve the id from the subject (in
  hand at every emit site); the per-binding counter line still
  keys off the binding id.

- **The published counter is no longer attribution-gated (P15).**
  handoff-3's consume-once publisher-TLS gated both the record AND
  the published *counter* behind a TLS that a keyed or otherwise
  unattributed publish never delivered to the probe — zeroing the
  fleet's published counters (and every `BUS_PUBLISH`). Counters
  are the dormant-mode contract and must count every genuine
  publish. Inbound wire re-dispatch is now excluded by NEGATIVE
  marking (the reader brackets its re-dispatch and the probe
  consumes the mark) instead of by requiring a positive TLS;
  genuine publishes are the unmarked default and always count,
  with best-effort locus attribution. The **keyed** dispatch
  flavors, which had no publish OR deliver probe at all (a routed
  market-data-style feed recorded zero of both), now emit both.

- **`LOTUS_OBS` never alters the wire; edges opt in with
  `LOTUS_OBS_WIRE=1` (P16).** The `(origin, seq)` edge header is a
  wire-format change a pre-header receiver cannot parse — an
  observed sender silently dropped every datagram at a stale peer,
  partitioning a mixed-version fleet invisibly. The UDP
  self-describing header and the framed-transport origin word now
  ride the wire ONLY under `LOTUS_OBS_WIRE=1`; with `LOTUS_OBS`
  alone the wire is byte-for-byte identical to an unobserved run.
  Cross-process edges require `LOTUS_OBS_WIRE=1` fleet-wide;
  counters and local records need only `LOTUS_OBS=1`.

- **Field-shaped acceptance test.** `obs_fleet_contract.rs` runs
  three processes over a real UDP multicast group and asserts the
  full consumer contract in one pass — nonzero publish AND deliver
  counters, NET records with a nonzero topic id + origin,
  cross-process `(origin, seq)` pairs, and `BUS_PUBLISH` attributed
  to a real birth instance — plus a keyed-publish probe test and a
  pristine-wire test (an unobserving receiver still receives from a
  `LOTUS_OBS=1` sender). Prior obs tests were 2-process loopback
  unicast, which is why the multicast/keyed/wire gaps slipped.

## v0.11.14 — iris observation NET seq pairing (edges), transport-branch parity (2026-07-28)

- **Native observation: NET (origin,seq) on the transport branch
  too (iris handoff-3 field re-test).** The handoff-3 fix landed
  on the raw-udp `sendto` branch, but a fleet whose bindings flow
  through `lotus_transport_send` still stamped origin 0 + a local
  receive counter (the field's still-zero edges). The framed
  transport wire header now carries `origin:16 | seq:48` (was
  seq-only), and both the fanout NET_SEND and the reader
  NET_DELIVER emit the wire `(origin, seq)` instead of `(0, local
  ctr)` — verified cross-segment over a framed unix transport
  (`obs_net_seq.rs`). The parity audit that found this is in the
  commit; the adapter branch (user transport loci) still has no
  NET probe and is a separate gap. Non-framed transports carry no
  wire seq and fall back to the local count.

- **Native observation: NET seq semantics + publish attribution
  (iris handoff 3).** The last cross-process-edge blocker.
  `NET_SEND`/`NET_DELIVER` now carry `origin:16 | seq:48` where
  origin+seq are the **sender's** identity+counter, echoed
  verbatim by the receiver from a self-describing 16-byte UDP
  wire header — so a send pairs with its delivers on
  `(origin, seq)` even when several senders multicast one
  subject (the receiver-local delivery count summed across
  senders was the zero-edges cause; P11). Origin is a nonzero
  per-process id (P12, was `unknown:0`). The header is prepended
  only when the sender is observed, so unobserved runs and
  non-Hale peers are byte-for-byte unchanged. And `BUS_PUBLISH`
  is attributed only for genuine local publishes (consume-once
  TLS); the reader thread's inbound re-dispatch no longer stamps
  a spurious `locus=0` publish record, so per-locus pub/dlv is
  nonzero in the field (P13). Cross-segment pairing pinned by
  `obs_net_seq.rs` (two real processes over loopback UDP);
  `obs_emission.rs` gains an exact
  publish-locus-equals-birth-instance assertion.

## v0.11.13 — C→Hale re-entry, iris observation field-hardening, Crumb bug fixes (2026-07-27)

- **Native observation emission: field-report hardening (iris
  handoff 2).** Six fixes from a ~16-binary production fleet run:
  NET_SEND now fires on the **UDP multicast fanout** (it was
  `continue`-skipped before the stream probe, so multicast
  publishers emitted no send-side records and the cross-process
  seq matcher rendered zero edges); LOCUS_BIRTH carries real
  **parentage** (emitted before field-default init so a child
  finds its parent registered — every tree was rendering flat)
  and **pinned children register on the spawning thread**
  (previously a pinned reader that parked before its first probe
  emitted nothing); BUS_PUBLISH **stamps the publishing locus**
  (per-locus perimeter pulses); topic **shape_hash is subject +
  canonical payload structure**, never the declaring type's local
  name, so two binaries sharing a subject fuse into one manifest
  row; and rings **re-emit EPOCH every 1024 records** so a
  high-rate ring that wraps its anchor stops reconstructing ~2^64
  ns timestamps. `obs_emission.rs` gains ring-walking assertions
  for parentage + attribution.

- **Direct `std::io::tcp::listen_socket` path-call fixed (Crumb
  batch-2 item 2).** The direct user-code lowering truncated the
  port to i32 against the C primitive's declared `(ptr, i16)`
  signature — a debug-info verifier failure, and silently
  mismatched IR without debug info. The stdlib's own `Listener`
  path was unaffected (it truncates correctly), which is why
  only direct calls tripped. Now i16 on both paths; regression
  test in `tcp_raw_fd_freefns.rs`.

- **C→Hale re-entry: `@export fn` emits a native C-ABI symbol
  (Crumb batch-2 item 1 — the port's critical path).** The same
  annotation wasm entry-inversion uses now works on native: the
  exported fn's literal name becomes an unmangled C-callable
  symbol (FFI-portable marshalling both ways), and codegen
  publishes the call-site arena in the caller-arena TLS around
  every `@ffi` call so a callback fired during an in-flight call
  re-enters with a live context — bus publishes and eager locus
  instantiation from inside the callback work (CI-proven).
  Same-thread contract at v1: foreign-thread/idle entry aborts
  with a pointed diagnostic; typecheck rejects non-portable
  types, defaults, and fallible exports. spec/ffi.md § "C→Hale
  re-entry". This is what lets QuickJS host functions land in
  Hale — `serve()`/`fetch()` from JS backed by `std::http` and
  the bus.

## v0.11.12 — the iris handoff: native observation emission, vec.set retire, verify modeling (2026-07-27)

- **Native observation emission (iris handoff P4).** `LOTUS_OBS=1`
  makes any hale binary publish an iris-protocol observation
  segment and emit records from the runtime's own choke points —
  BUS_PUBLISH/BUS_DELIVER at dispatch, NET_SEND/NET_DELIVER with
  per-binding seqs at the transport layer, LOCUS_BIRTH/DISSOLVE/
  RESTART from the lifecycle paths — lighting up a whole deployed
  stack with zero app changes. Dormant = one branch per probe;
  observed-but-unattached = counters only; SPSC ring per emitting
  thread; live-locus birth replay on observer attach (late-attach
  tree reconstruction). Verified end-to-end against iris's own
  `peek` consumer (pub=dlv 1:1, manifest-resolved names, births
  incl. pinned loci). spec/runtime.md § "Native observation
  emission"; `obs_emission.rs` pins the segment contract +
  dormant default.

- **`@form(vec).set` no longer leaks or slows down (iris handoff
  P1).** Vec elements are pointer-storage; `set` deep-copied the
  new element into the form owner's program-lifetime arena but
  never retired the REPLACED one — ~33 B leaked per set, and the
  growing arena made the per-set containment walk progressively
  slower (the reported ~1µs/set, ~1000× `get`; ~1.4 MB/s in a
  ~1M sets/s observer). Replaced elements (and their
  non-surviving String fields, hashmap retire-cell discipline)
  now retire straight onto the arena's reuse freelist and the
  deep-copy alloc consults it: the iris repro went from 2.06 s /
  70 MB to 0.01 s / 7.8 MB flat over 2M sets, ASan-clean.
  Single-owner caveat spec'd in forms.md: a `.get` value is
  invalidated by a later `set` to the same slot.

- **`hale verify` unbounded-allocation analysis: three
  false-positive shapes fixed (iris handoff P2).** (1) Loop
  ceilings const-fold — `while i < NET_SLOTS * WINDOW` over
  top-level consts ranks bounded. (2) Eager per-iteration
  children are modeled: a bare-statement `Cycle { ... };`
  dissolves at the statement, so neither the instantiation site
  nor the child's own self-stores accumulate (let-bound and
  subscription-bearing instantiations, and loci containing
  `while true`, keep the conservative verdict) — the analysis
  no longer flags the very idiom its advisory recommends.
  (3) A vec `.set` is no longer an accumulation channel (see the
  P1 retire). Both advisory texts now name `@unbounded` on the
  enclosing fn/hook as the acknowledge mechanism for
  domain-bounded shapes. fuse-hl: 26 findings → 0, with the
  let-bound/while-true counterparts still flagged.

- **Docs: takeover send timeouts.** `std::io::tcp::
  set_send_timeout(fd, d)` already shipped but the takeover
  chapter never mentioned it — a stalled SSE/WS peer blocks
  `send` forever without it (iris handoff P3). The chapter now
  says so next to the recv-timeout note.

## v0.11.11 — or wait + bounded topics (the backpressure contract), std::compress + std::tar, teardown delivery contract (2026-07-27)

- **Bounded topics + consumer shed bounds (GH #255 phase 2).**
  Topic-level `bounded(N); on_full: fail;` makes publishes
  refusal-fallible: every send site carries a disposition —
  `or raise` (synchronous BusFull refusal), `or discard`
  (at-capacity registrations shed the newcomer, counted), or
  `or wait` (park until the drain frees space — queue-full is
  the second wake source for the phase-1 disposition, no new
  surface). Subscriber-level `bounded(N, drop_new|drop_old)` is
  that consumer's private cap; `drop_old` keeps the newest N
  (ring semantics — right for reload/telemetry events), and a
  consumer bound below the topic bound sheds privately before
  refusal ever fires (min governs). v1 scope: main-queue
  subscribers only — pool queues and pinned mailboxes are
  already bounded MPSC rings with producer-blocking
  backpressure (GH #125), so declared bounds there are
  typecheck-rejected with that explanation; err-payload
  dispositions on full-fail topics land in a follow-up slice.
  Self-checking corpus fixture `73-bounded-bus`; contracts
  pinned by `bus_bounded_topics.rs`.

- **`or wait` — park a publish through the loss window
  (GH #255 phase 1).** A send to a transport-bound topic can
  attach `or wait`: instead of the counted `dropped_lost` drop
  while a connect binding is lost/reconnecting, the publisher
  parks until the app's `on_failure` → `restart (t)` re-arms
  the binding, then publishes onto the live link. A
  delivery-mode modifier, not error handling — the send stays
  infallible; `wait` is its own disposition kind, rejected on
  unbound topics ("nothing to wait for"), on fail-policy keyed
  publishes, and in expression position. Main-thread waiters
  pump their own queue drain (loss dispatch + ticks) while
  parked; a structurally unsatisfiable wait raises — failed
  reconnect takes the existing structural exit, and main
  teardown wakes parked waiters into `BusWaitAborted` before
  the pinned joins (a parked publisher can never hang
  teardown). Per-binding `waits` counter joins the #236 dump.
  Phase 2 (bounded topics, designed on the issue) feeds
  queue-full into this same disposition later.

- **`std::compress` + `std::tar` — compression and archives
  (GH #254).** One-shot over `Bytes`, all `fallible(IoError)`:
  `compress::gzip`/`gunzip` (zlib, gzip container; gunzip
  auto-detects bare zlib too), `compress::zstd`/`unzstd`
  (libzstd, **dlopen'd at first use** — no link-time dependency;
  machines without it get a clean `not_found`), and a ustar
  `std::tar` (indexed read: `entries`/`entry_name`/`entry_size`/
  `entry_type`/`entry_data`; append-style write:
  `pack`/`pack_dir`/`finish`). Corrupt input fails
  `kind="invalid"`; decompression is guarded at 1 GiB one-shot
  (zip-bomb protection). Plus the companion the whole pipeline
  needed: **`std::io::fs::write_bytes(path, b)`**, the
  binary-safe file write (`write_file` truncates String content
  at the first NUL). Hale-built `.tar.gz` output is accepted by
  system `tar`/`gzip` (pinned by `compress_tar.rs`). This was
  the distance between hale-bun's parody registry and the real
  npm protocol; it also unblocks HTTP `Content-Encoding` work.

- **Teardown no longer drops pinned workers' final publishes
  (GH #253, hale-bun handoff item 2).** A parent whose `run()`
  returned immediately used to dissolve eagerly — cascading its
  subscriber fields' teardown and destroying the arena holding
  its pinned children's self structs — before those pinned
  threads were joined, so events they published in their last
  moments were silently dropped in ANY declaration order (the
  hale-bun install-fanout shape). A dissolving parent now joins
  its own pinned children (mailbox shutdown → join → bus drain)
  before the field cascade, and the fn-exit flush joins
  subscription-less pinned entries before any cooperative
  teardown. The delivery contract — what is now guaranteed and
  what still drops (publish after the last subscriber dissolved:
  coordinate completion explicitly) — is spec'd in
  spec/runtime.md and docs/src/services/bus.md. Self-checking
  corpus fixture `72-teardown-publish-delivery`. The fixture
  also flushed out a pre-existing devirt soundness bug (caught
  by the static/dynamic differential in CI): the direct-call
  gate never checked PUBLISHER placement, so a pinned publisher
  could run a same-thread subscriber's quiet handler directly on
  its own thread — two such publishers ran it concurrently and
  lost `self.x + 1` updates. Direct-call eligibility now
  requires every publisher same-thread; off-thread publishers
  stay on the serializing enqueue path.

- **Conditional instantiation of a deferred-dissolve locus fixed
  (hale-bun handoff item 1).** `if c { App { }; }` with a
  placement-bearing child died at build time with an LLVM
  dominance failure ("Instruction does not dominate all uses",
  debug-info verifier) — and without debug info the same broken
  teardown IR was emitted silently: the fn-exit dissolve flush
  referenced pointers defined inside the branch, so the
  not-taken path tore down garbage. Deferred-dissolve entries
  now spill their self pointer to a NULL-initialized entry-block
  slot; the flush loads it and skips teardown entirely when the
  instantiation never ran. Corpus fixture
  `71-conditional-instantiation` pins both branches.

- **Unknown qualified names diagnosed as unknown names (hale-bun
  handoff item 3).** A qualified struct literal whose path
  resolves to nothing (e.g. `std::process::Output` — real name
  `ProcessOutput`) used to error "qualified-name struct literal
  in expression position", which reads as a positional
  restriction (expression position is fully supported for names
  that resolve). It now says `unknown qualified name` with a
  did-you-mean — substring match against siblings under the
  same prefix first, nearest-name second, else a listing of
  what the namespace provides.

- **`std::process` ENOENT hint for shell-split argv (hale-bun
  handoff item 4).** `run("echo hello world")` execs a binary
  literally named that, and the resulting `not_found` read as
  "echo isn't on PATH" — the first mistake every new user makes
  against the newline-separated argv convention. When run/spawn
  fails ENOENT and argv[0] contains a space, the IoError's
  `path` label now names the real mistake.

- **`std::io::tcp::send_fd(fd, b: Bytes)` — public raw-fd send
  (hale-bun handoff item 4b).** The write-side takeover
  companion to `close_fd` / `recv_into`: a handler that keeps a
  taken-over `Request.conn_fd` previously had only the internal
  `__send_bytes` to write with. Same contract as
  `Stream.send_bytes` (Unit success, `fallible(IoError)`).

## v0.11.10 — the publish contract + loss supervision, macOS unix transport, SPSC observation ring, diagnostics overhaul (2026-07-22)

- **Diamond imports fixed (GH #249, iris friction F.10).** A lib
  reached a second time — by the entry and by another lib, or by
  two libs — now registers the second importer's alias against
  the shared mangled names (seed-rename cache keyed by canonical
  lib path). Previously the resolver's visited-set dedup skipped
  the registration, so `alias::Name` references in the second
  importer leaked into codegen as "qualified type not in stdlib
  path-renames table" / "unknown type name in signature" while
  `hale check` passed — with which alias broke depending on
  import order. This was the bug gating iris's reuse of its
  spike rendering libs.

- **SPSC observation ring as a lotus primitive (GH #244).**
  `lotus_spsc_*` + `std::ring::__spsc_*`: a single-producer
  16-byte-slot ring over caller-provided memory — monotonic
  release-published head, overwrite-oldest (never blocks a
  producer on readers), producer-side drop accounting, external-
  reader-safe snapshot reads with overrun accounting. Layout is
  a stable documented contract (spec/runtime.md) intended for
  verbatim adoption by the iris observer protocol; concurrent
  contract test + GenMC model included. Convergence note: the
  driver test empirically refuted the pre-freeze protocol
  sketch's overrun boundary (`< h2 - ring_slots`) — an in-flight
  producer write already clobbers slot `h2 - ring_slots` before
  publication, so the live window is `(h - ring_slots, h]`.
- **Diagnostics: caret snippets, did-you-mean, span-carrying
  codegen errors (GH #241).** Every rendered diagnostic shows
  the offending source line with a caret underline; the
  no-field diagnostic suggests the nearest name from the
  receiver's own surface; printing a struct/locus and
  abs/min/max on non-numerics are now spanned check-phase
  errors (previously spanless codegen deaths); and
  `CodegenError::UnsupportedAt` lets any codegen raise carry a
  location rendered like a check diagnostic.

- **Test failures report per-assertion progress; runtime C
  warnings no longer leak (GH #230 items 1+3).** A failing
  multi-assert test file now prints `(N earlier assertion(s)
  passed)` under the ASSERTION FAILED diagnostic — the pass path
  stays silent, so the exit-0-and-quiet contract is untouched.
  And the emitted clang invocation compiles the runtime TU with
  `-w` (Hale users can't act on lotus_arena.c diagnostics);
  compiler developers re-enable with `HALE_CC_WARNINGS=1`.
  Item 2 (decimal display) resolved per the design call:
  declared precision isn't stored in the Decimal repr, so
  default printing keeps trimming — the new
  `std::decimal::format(d, places)` renders exactly `places`
  fraction digits (0..=9, round half-up) for money-style
  fixed display.

- **Per-binding transport telemetry counters (GH #236 item 2).**
  Every remote binding maintains relaxed-atomic counters at the
  transport choke points — messages/bytes sent and delivered,
  send failures, `dropped_lost` (publishes made while a connect
  binding was in the lost/reconnecting window), listener
  re-arms, reconnects, and `seq_gaps`. `LOTUS_BUS_COUNTERS_DUMP=1`
  prints one line per binding at teardown; this is the substrate
  for the iris observer. (Entry restored — it was dropped in a
  changelog merge during the release cycle; the feature shipped
  in v0.11.10 as PR #237.)
- **macOS unix-transport support via framed SOCK_STREAM + wire
  sequence numbers (GH #231 transport half, GH #236 item 1).**
  Darwin has no AF_UNIX `SOCK_SEQPACKET`, so the substrate unix
  transport now has a framed byte-stream mode — per-message
  `[u64 len][u64 seq]` header, boundaries preserved by the
  transport instead of the kernel — selected by default on
  macOS (`#ifdef __APPLE__`) and forcible on Linux with
  `LOTUS_UNIX_STREAM=1` (set it for every process on the
  socket; the two wire formats don't interoperate — a
  mismatched peer is detected via the length sanity cap and
  refused loudly). The "build a monolith, deploy a distributed
  system" flow now works on macOS. The seq stamp is #236's
  loss-computability primitive: per-connection monotonic,
  starting at 1, reset per accepted peer; the receiver counts
  gaps (`seq_gaps` in the `LOTUS_BUS_COUNTERS_DUMP=1` line).
  Linux SEQPACKET default unchanged. Homebrew
  libunwind/OpenSSL static-linking for the prebuilt toolchain
  remains open on #231; the install page's platform matrix is
  now honest about both carve-outs.- **Per-binding transport telemetry counters (GH #236 item 2).**
  Every remote binding now maintains relaxed-atomic counters at
  the transport choke points — messages/bytes sent and
  delivered, send failures, `dropped_lost` (publishes made while
  a connect binding was in the lost/reconnecting window — the
  drops GH #233's contract makes deliberate, now countable),
  listener re-arms, and reconnects. `LOTUS_BUS_COUNTERS_DUMP=1`
  prints one line per binding at teardown (operator/test
  surface); no in-process consumer yet — this is the substrate
  for the iris observer. Sequence numbers (item 1) ride #231's
  framing rework per the sequencing recorded on the issue.
- **Connection loss is structural; `restart` reconnects
  (GH #233 steps 3–4, closes #233).** A send failure on a
  source-declared connect binding now marks the binding lost
  (publishes during the window are dropped, never falsely
  "delivered") and routes a synthetic `link_lost`
  `ClosureViolation` through the main locus's `on_failure` at
  the next queue drain. Declare
  `on_failure(t: std::bus::UnixTransport, err: ClosureViolation)
  { restart (t); }` on `main` to reconnect — the runtime re-runs
  the connect-with-retry and publishing resumes (the new
  public name `std::bus::UnixTransport` names the connect-side
  substrate transport locus). Without a handler (or when
  reconnect fails), the process exits non-zero with a
  diagnostic naming the subject — completing the publish
  contract: the broker never accepts what it cannot deliver, at
  boot (#227) or mid-run. `LOTUS_BUS_CONFIG` routes sit outside
  the supervision tree and keep logged-only send failures.
- **Substrate unix transports are loci; listen bindings re-arm
  (GH #233 steps 1–2).** A `bindings { T: unix(...) }` entry now
  desugars to a stdlib transport locus
  (`__StdBusUnixListenTransport` / `__StdBusUnixConnectTransport`)
  instantiated as a cooperative child at the main prelude —
  converging with the adapter path, per F.37's
  transports-as-loci direction. birth() realizes synchronously
  on the boot path (behavior of GH #227 preserved verbatim);
  dissolve() interrupts, joins, and reclaims. The listen serve
  loop now **re-arms on peer EOF** — it closes the dead
  connection and accepts the next peer instead of silently going
  deaf for the rest of the process — so rolling restarts of the
  connect-side binary just work (`LOTUS_BUS_CONFIG` unix
  listeners share the same loop and re-arm too). The hot path is
  unchanged: publish fanout still writes the C remote table
  directly (locus for flow, C for bytes). Loss-is-structural +
  restart-as-reconnect are GH #233 steps 3–4.
- **Bus binding failure is now structural (F.37, GH #227).** A
  `bindings { }` entry or `LOTUS_BUS_CONFIG` route whose
  transport cannot be realized — socket/bind/listen/addr
  failure, connect-retry timeout, unparseable route — now fails
  the declaring locus's birth: structural diagnostic on stderr
  naming the subject + non-zero exit at boot. Previously the
  runtime perror'd and ran on with a dead table entry, so every
  publish "succeeded" while fanout silently dropped the
  messages (the failure mode an external reviewer hit on macOS
  via SEQPACKET's `Protocol not supported`). Listener-side
  realization (unix `socket+bind+listen`, udp parse+bind+group
  join) moved from the reader thread to the synchronous boot
  path, so failures can't die invisibly on a detached thread —
  only blocking accept/recv stays threaded, preserving the
  no-hang-at-boot property. Bonus fix: teardown now shuts down
  the listener fd, so a subscriber whose peer never connected
  exits cleanly instead of hanging in `pthread_join` at
  dissolve. Per-send transient errors on lossy transports (udp
  `sendto`) stay logged-not-fatal, per the new normative publish
  contract in `spec/semantics.md`. Regression tests inject
  failures platform-independently (ENAMETOOLONG / ENOENT), per
  the issue's ask — no macOS hardware needed.
- **Toolchain reorg: `hale mcp`, `crates/hale-lsp`,
  tree-sitter-hale.** (1) `hale mcp` — a Model Context Protocol
  server in the binary (stdio, newline-delimited JSON-RPC):
  14 tools — the toolchain surface self-execs this very binary so
  the tools and the CLI they describe cannot version-skew (the
  drift that killed the separate Node server), the bus-graph/
  placement/enforcement/alloc-summary analyses call hale-lsp
  directly, and `hale_docs_search` greps the language spec
  embedded at build time (864 KB — an installed hale grounds
  language rules with no checkout). `HALE_MCP_ROOT` sandboxes
  path arguments. The Node hale-mcp is retired. (2) The LSP moved
  to its own workspace crate (`crates/hale-lsp`) — same binary,
  same surface, cleaner boundary. (3) The tree-sitter grammar
  moved out of pond to
  [hale-lang/tree-sitter-hale](https://github.com/hale-lang/tree-sitter-hale)
  (full history) with corpus-sync CI: every push parses the hale
  fixture corpus; the 11 known grammar gaps are enumerated in its
  issue #1 and XFAIL'd, so green means "no NEW drift".
- **Stdlib doc migration complete at decl level.** Every public
  `.hl`-backed declaration in the rename table — 73 more across 19
  files (http Server/Router/Client + both Request/Response pairs,
  io_tcp Stream/Listener, udp Reader, json Builder + span types,
  process Child + the full fn family, file, term, text sinks,
  yaml, cli, iter, tagged, mirror_ring, lang, name, source, bus
  Adapter) — now carries `///` docs, so `hale doc --stdlib`
  renders a fully-documented reference for the entire locus/type
  surface (`--stdlib` also gained a Type arm for the renamed
  type decls). Remaining doc-less entries are the
  signature-table-only C-primitive fns, which need a doc field in
  the FnSig table (separate arc). Method-level docs exist where
  the surface demanded them (metrics); broad method-doc coverage
  is incremental from here.
- **`std::http` connection takeover — the Upgrade surface.**
  `Request.conn_fd` carries the live fd into the handler;
  `Response { takeover: true }` writes only the status line + the
  response's own headers (101 gets its `Switching Protocols`
  phrase) and returns without closing the connection — the new
  `Stream.release_fd()` primitive disarms the per-connection scope
  close and hands ownership to the handler. Status-agnostic
  (WebSocket 101, CONNECT 200). Verified over a real socket:
  101 + upgrade headers, then raw bytes echoed on the same
  connection (`http_upgrade.rs`). This closes the gap WebSocket
  promotion was blocked on; the 5s recv timeout stays armed until
  the new owner clears it.
- **`hale verify` — the Layer-2 discipline gate** (the last
  planned CLI row besides `bench -compare`). Identical analysis
  surface to `hale check` (typecheck + the advisory analyses:
  unbounded-alloc survey, hot-path lint, placement/starvation,
  accept-without-release, bus checks) but ANY finding exits 1 —
  `check` stays the fast advisory oracle, `verify` is what CI
  runs. `--json` and the check flags carry over. Tests:
  `hale-cli/tests/verify.rs`.
- **`hale bench` — the Layer-3 runner** (spec/testing.md's planned
  row, now real). `*_bench.hl` discovery; zero-param `bench_*` free
  fns; a synthesized driver self-calibrates Go-style (batch ×10
  until ≥100 ms) and reports ns/op + allocs/op
  (`std::diag::heap_alloc_count` deltas). Release-profile compile
  with the same `[ffi]` pickup as build/test; `-run` filter;
  `--json` records. Baselines-with-bands and `-compare` stay
  planned. Tests: `hale-cli/tests/bench.rs`.
- **`hale doc --stdlib` + first stdlib doc comments.** The `std::`
  surface renders as a generated API reference: public paths from
  the rename table, decl shapes + `///` docs from the bundled
  stdlib source (mangled param types demangled; internal-typed
  params hidden), and the signature table fills in the
  C-primitive-backed free fns with no `.hl` decl. First
  namespaces migrated to `///` docs: std::metrics (full surface),
  std::log (Logger + all three sinks), std::bytes::BytesBuilder.
  spec/stdlib.md stays the contract; the generated reference is
  the browsable companion.
- **DWARF struct members (debug stage 4).** User struct types are
  emitted as real `DW_TAG_structure_type`s with named members at
  their LLVM layout offsets — `p *rec` in gdb prints
  `{key = "alpha!", n = 41, f = 2.5, sub = <ptr>}` instead of an
  opaque address. Members map shallowly (scalars + String/Bytes
  precise; nested struct members as typed opaque pointers — no
  recursion, so mutually-referential shapes can't loop). readelf
  regression extended.
- **LSP v5: formatting, document symbols, `hale/enforcement`.**
  documentFormattingProvider wraps the `hale fmt` core (one
  whole-document edit; null on an unlexable buffer);
  documentSymbol returns the hierarchical outline (locus → params
  fields + methods); the hale-only `hale/enforcement` request maps
  every user fn/method to its `@hot` / `@budget` / `fallible` /
  `@unbounded` contract. Protocol test `lsp_v5_...`.
- CI: `hale fmt --check` gates the repo's own `.hl` surface in the
  tests workflow (styleguide §5's fmt tier).

## v0.11.9 — hale fmt + hale doc, LSP completion, DWARF variables, hale test @ffi (2026-07-18)

- **`hale doc` — API-reference generator + `///` doc-comment
  convention.** `///` lines directly above a declaration attach to
  it (decorators may sit between); `hale doc [file | dir]` renders
  every public top-level declaration — fns, loci with params and
  documented methods, types, topics, interfaces, consts — with
  signatures and doc text, as Markdown (stdout or `-o`) or `--json`
  records for tooling. `__`-prefixed names and `main` are skipped.
  Doc text recovers positionally, so no lexer/AST change. Spec:
  tokens.md comment section + testing.md tool row/section. Tests:
  `hale-cli/tests/doc.rs`.
- **DWARF variable info (debug story stage 3).** Emission moves
  from LineTablesOnly to Full: fn/method parameters and let-bound
  locals carry `dbg.declare` with real DWARF types — `Int`/`Float`/
  `Bool`/`Decimal`/`Time`/`Duration` as proper base types, `String`
  as `char*` (gdb prints the text, not an address), struct values
  as named typed pointers, everything else ABI-derived. gdb goes
  from "stop on a .hl line" to `info args` / `info locals` /
  `print msg` with real values. Param declares attach when the
  body's first statement creates the subprogram (they're collected
  at the prologue); `<optimized out>` after a variable's last use
  remains normal optimizer behavior — `--dev` keeps more of the
  frame. Structural regression via readelf in
  `hale-cli/tests/debug_info.rs`; `LOTUS_NO_DEBUGINFO=1` still
  opts out entirely.
- **`hale test` links `@ffi` libs.** The test runner's per-file
  compile now runs the same Stage-2 `hale.toml [ffi]` csrc/link
  pickup `hale build` does, so tests importing FFI-bearing libs
  (pond/sqlite and everything on it) compile and link instead of
  dying with undefined references — closing the open pond FRICTION
  entry ("hale test cannot link @ffi libs", 2026-07-04) and the
  one place the runner contradicted the three-gates verification
  story. Test binaries also build with the dev profile now (they
  rebuild every run; nothing in the exit-code contract times).
  Regression: `hale-cli/tests/test_ffi_pickup.rs`; validated
  against pond's real sqlite/jobs/migrations tests (previously 5
  link failures, now green).
- **LSP v4: completion.** `textDocument/completion` (trigger
  characters `.` and `:`): after `self.` — the enclosing locus's
  params (with types) and user-declared methods (with signatures);
  after `std::…::` — the stdlib surface namespace-by-namespace
  (free fns carry `fn(params) -> ret fallible(E)` detail from the
  signature table, locus paths and child namespaces listed); bare
  words — the seed's top-level symbols (fns/loci/types/topics/
  interfaces/consts), keywords, and primitive type names. Context
  detection reads the raw text left of the cursor, so it works
  mid-keystroke when the buffer doesn't parse; the symbol side
  falls back to the on-disk seed in that case. Same
  no-index/no-state design as v1-v3. Protocol test:
  `lsp_v4_completion`.
- **`hale fmt` — the canonical formatter** (spec/testing.md's
  "(planned)" slot, now real). Zero config, Go-style: a
  token-stream formatter that preserves the author's line breaks
  and normalizes indentation (4-space, bracket-stack), inter-token
  spacing (canonical pair rules incl. unary/binary `-`
  disambiguation, tight generics, the spaced `: serves` colon),
  blank lines (max one), and comment placement. `hale fmt [paths]`
  writes in place; `--check` is the CI gate (exit 1 + offender
  list); `--diff` previews; `--stdin` filters for editors. Safety:
  output is re-lexed and must produce a byte-identical semantic
  token stream or the file is left untouched — a formatter bug
  can't change what the compiler sees; unlexable files are skipped
  loudly. Idempotence + gate anchored over every fixture and
  stdlib source (`fmt_corpus.rs`); CLI contract covered in
  `hale-cli/tests/fmt.rs`. The repo's own `.hl` surface (fixtures,
  stdlib, README/play examples) is reformatted to canonical form
  in the same change — the full suite runs green on the formatted
  corpus.

## v0.11.8 — std::metrics + log sinks, LSP v3, cell single-owner, lld links (2026-07-18)

- **Build: link via lld when installed.** The non-LTO link now
  probes once for `ld.lld` and passes `-fuse-ld=lld` (Linux;
  `HALE_NO_LLD=1` opts out; silent fallback otherwise). The default
  bfd linker spent ~120 ms per build scanning the ~27 MB
  tree-sitter shim staticlib — measured 148 ms vs 26 ms on the
  identical link line. Dev builds drop from ~100 to ~55 ms (hello)
  and ~159 to ~119 ms (Server+metrics app); release links speed up
  identically. The staged dev-mode prebuilt-stdlib-object cache was
  re-scoped and deferred on fresh measurements — post-DCE its
  remaining win (~50-65 ms) no longer justifies split-module
  emission (stdlib lowering bakes app-derived bus-devirt state);
  rationale + numbers in notes/build-latency-and-lsp.md.
- **Runtime: @form cell single-owner + Bytes grow-path retirement.**
  Two anchor-retirement residuals closed. (1) A hashmap `set` / lru
  `put` now walks a stack snapshot of the value struct and clones
  String/Bytes leaves through force-copy variants
  (`lotus_*_clone_cell_owned`) — previously the same-arena clone
  skip let a cell share a blob with the self-storage struct it was
  set from (`m.set(self.rec)`), so an in-place field overwrite
  mutated the cell silently and a retire on either side could
  dangle the other. Statics still pass through and cross-arena
  values clone as before; the cost lands only on get-then-set
  round-trips, where the freelist recycles the replaced
  generation. (2) `self.X = <bigger Bytes>` grow now retires the
  abandoned blob instead of orphaning it, and Bytes allocation
  consults the retire freelist through alignment-aware pops
  (align-1 String and align-8 Bytes blocks share one list; a
  candidate must satisfy the request's alignment). Caveat carried
  over from the String side: a shrink collapses recorded capacity,
  so an oscillating field can't self-serve its own grows — the
  reclaim pays off through other same-arena allocations. Tests:
  `hashmap_cell_alias.rs` (deterministic mutation-visibility
  repro) + fixture `70-cell-single-owner` (mixed String/Bytes
  churn, ASan-clean under the corpus oracle); spec/memory.md §5/§7
  updated.
- **`std::metrics` — Prometheus metrics, promoted from pond/metrics.**
  `Registry` (namespace prefix; **owns its storage** as param-default
  children, so `Registry { namespace: "app" }` is the whole
  construction and a Registry returned from a builder fn keeps its
  series alive), idempotent factory free fns `counter` / `gauge` /
  `histogram` returning hot-path handles that reference the storage
  slots directly (resolve at boot, cache as a field — S12), labels
  helpers, text-exposition `render()`, and `Endpoint` — a
  `std::http::Handler` that turns any `std::http::Server` into a
  `/metrics` scrape target (`Content-Type: text/plain;
  version=0.0.4`). Histogram bounds are a space-separated ascending
  String parsed once at registration (max 32 buckets), replacing
  pond's math-lib Matrix signature; buckets render cumulatively with
  the implicit `+Inf` plus `_sum` / `_count`. The metric map is
  `sync = serialized` for the scrape-pool-reads-while-handlers-write
  topology. Covered direct + over-TCP in
  `crates/hale-codegen/tests/stdlib_metrics.rs`; new docs chapter
  (`docs/src/everyday/metrics.md`); pond copy frozen.
- **`std::log::FileSink` + `std::log::ConsoleSink` — promoted from
  pond/logfmt.** FileSink appends every `log.**` event to `path` and
  rotates by size (`max_size_bytes`, `keep_files`; atomic
  `rename(2)` chain shifts, oldest evicted), capturing I/O failures
  in the `last_error_kind/errno/path` triple; it also wears the
  `std::text::Sink` shape (`write`/`line`/`newline`). ConsoleSink
  renders dim HH:MM:SS + colored width-5 level badge + dim path +
  message with the WARN/ERROR stderr lane split; color is AUTO
  (stderr tty probe; `FORCE_COLOR`/`CLICOLOR_FORCE` override,
  `NO_COLOR` always wins, `color: false` = never). pond's OtlpSink
  stays pond-tier. Test:
  `crates/hale-codegen/tests/stdlib_log_sinks.rs`.
- **LSP v3: definition, references, `hale/placement`,
  `hale/allocSummary`** (committed 2026-07-17 as `92fb0c6`).
  Goto-definition and find-references over the same seed re-analysis
  the rest of the server uses; `hale/placement` answers the
  pool/placement table the checker computes; `hale/allocSummary`
  surfaces the per-fn allocation survey.

## v0.11.7 — LSP v2: hover with contracts + `hale/busGraph` (2026-07-17)

- **Hover.** `textDocument/hover` resolves the token at position and
  answers with the signature *plus the contracts no generic language
  server carries*: a fn's `fallible(E)` with the addressing hint and
  its enforcement status (`@hot` — lint-as-errors; `@budget(
  alloc_per_call = N)` — compiler-enforced ceiling) read from the
  declaration; a topic's payload, subject, and `keyed_by` routing
  field; a locus's params, accepted child type, and bus surface; a
  type's full field/variant listing; an interface's methods with the
  structural-satisfaction note; `self.<field>` resolved through the
  enclosing locus; and `std::` paths through the stdlib signature
  table. Same design as v1: no index — every request re-analyzes the
  seed through the ~10 ms front-end.
- **`hale/busGraph`** — a hale-only custom request returning the
  seed's whole message topology: per subject, its publishers (locus +
  payload), subscribers (locus + handler + placement + payload), and
  the static-dispatch verdict with its honest ineligibility reason.
  "Who subscribes to this topic?" becomes one protocol call instead
  of a grep session — aimed squarely at coding-agent harnesses.
- Protocol test extended (`lsp_v2_hover_and_bus_graph`); README and
  the first-run guide describe the new surface. Known polish item: a
  user fn whose fallible payload names a stdlib-injected error type
  (e.g. `IoError`) hovers that payload as `?` — the fallibility
  itself still shows. Staged for v3: goto-definition/references,
  `hale/placement`, `hale/allocSummary`.

## v0.11.6 — `hale lsp` (2026-07-17)

- **`hale lsp` — a stdio Language Server, v1: diagnostics.** Point
  any LSP-speaking editor or coding-agent harness at it and the full
  `hale check` surface arrives as you type: parse/type errors at
  error severity, the advisory analyses (unbounded-allocation
  survey, hot-path lint, placement/starvation, accept-without-
  release) as warnings, each with real ranges (UTF-16 columns) and
  the diagnostic kind as the LSP code. The design leans on the
  front-end being ~free (`hale check` ≈ 10 ms whole-program): every
  didOpen/didChange/didSave re-parses and re-checks the changed
  file's whole seed (its directory, per the F.19 model) with the
  editor's unsaved buffer winning over disk — no incremental
  analysis, no index, no warm-up, no configuration. Diagnostics
  publish for every file in the seed so stale squiggles clear
  without bookkeeping; a parse error gates the typecheck so
  mid-keystroke syntax holes don't cascade phantom type errors.
  Protocol lifecycle covered end-to-end in
  `crates/hale-cli/tests/lsp.rs` (initialize → error → fix-clears →
  warnings → parse error → clean shutdown). v2 (staged in
  `notes/build-latency-and-lsp.md`): hover with type + fallibility
  + enforcement status, go-to-definition/references, and the
  hale-only custom methods — `hale/busGraph`, `hale/placement`,
  `hale/allocSummary` — the checker already computes.
- README + first-run guide teach the one-command integration;
  `hale check --json` remains the minimal scripted alternative.

## v0.11.5 — std::process::try_wait + signal (2026-07-17)

The subprocess arc: the one missing lifecycle primitive for
supervising daemons, plus arbitrary signals — promoted from
pond/subprocess's surface.

- **`std::process::try_wait(c) -> Int fallible(IoError)`** —
  non-blocking reap via `waitpid(WNOHANG)`. Returns `-2` while the
  child is still running (the same retryable-sentinel shape
  `recv_into` uses — poll again on your next tick), the exit code
  (`0..255`) on a normal exit, or `-1` when killed by a signal (the
  child is reaped in both terminal cases). An already-reaped child
  surfaces `kind="not_found"` (ECHILD) through the error channel.
  This closes the styleguide's "daemons can't non-blocking-reap
  children" gap: a supervisor's periodic `tick()` polls `try_wait`
  per child without ever parking its pool, where the only prior
  option was a blocking `wait` or short-timeout sleeps. The
  supervisor idiom is documented in the operations chapter.
- **`std::process::signal(c, sig) -> () fallible(IoError)`** —
  send an arbitrary POSIX signal to the child's pid (15 = TERM,
  1 = HUP for config reloads, 10/12 = USR1/USR2, …). Promoted from
  pond/subprocess's `Process.signal`; the fixed TERM→KILL
  escalation remains `kill`'s job. ESRCH surfaces
  `kind="not_found"` — usually benign post-exit (`or discard`).
- Both honor the manual-`Child` convention `wait` established:
  `pid <= 0` answers "already exited with code 0" / no-ops.
- Deliberately NOT promoted: pond's `Process` bus-streaming locus —
  its stdout/stderr streaming side is a documented placeholder in
  pond (`run()` is a no-op pending non-blocking line-drain
  primitives), and the stdlib ships behavior, not intentions. The
  vendored lib carries a pointer at the new surface.

Coverage: `process_try_wait.rs` (poll-to-exit without blocking,
TERM observed as signal-kill, double-reap through the error
channel, sentinel conventions). Full workspace suite green (296
test binaries).

## v0.11.4 — std::http grows a Router and a client (2026-07-17)

Two pond libraries promoted into the stdlib — the batteries every HTTP
program reached for: path routing on the server side, and outbound
requests on the client side. Both arrived production-proven (pond's
vendored copies are frozen with pointers here).

### std::http::Router (promoted from pond/router)

- **Path routing + middleware as a stdlib battery.** Register
  `METHOD /path/:capture` patterns against handler loci
  (`add(method, pattern, h)`; first match wins, method matching is
  case-insensitive at register time), wrap the chain in
  before/after `Middleware` (onion order, `use(m)`), and mount the
  Router straight on a Server — it satisfies `std::http::Handler`
  structurally, so `Server { handler: router }` just works. Route
  handlers implement the new `RouteHandler` contract
  (`handle(ctx: Context) -> Response`); `Context` bundles the parsed
  request with extracted params — `path_param(ctx.params, "name")`
  for `:name` captures, `query_param(ctx.params, "k")` for `?k=v`
  pairs (`""` when absent; not URL-decoded at v1). Unmatched
  requests hit an overridable `not_found` handler. Promotion
  simplifications vs the vendored original: handlers return
  `std::http::Response` directly (the local Response type + boundary
  conversion were a vendored-lib aliasing workaround), and in-file
  declaration order retires the alphabetical file-naming hack the
  vendored copy needed for its storage loci.

### std::http client (promoted from pond/http/client)

- **Outbound HTTP/1.1 for both schemes.** One-shot free fns —
  `get(url)` / `post(url, body, content_type)` / `request(req)`, all
  `fallible(HttpError)`, `Connection: close` — plus the pooled
  `Client` locus: retry-with-backoff, configurable user-agent/body
  cap, and opt-in `keep_alive: true` that switches to framed reads
  (Content-Length or `Transfer-Encoding: chunked`, chunk extensions
  included) over a per-host connection pool. Fd reuse is
  regression-proven: two keep-alive requests ride one accepted
  connection in the test's server-side accept count. Client-side
  types are deliberately distinct from the server side —
  `ClientRequest` targets a `Url`; `ClientResponse` carries a
  **Bytes** body so binary content (embedded NULs) survives —
  and `parse_url` decomposes scheme/host/port/path. Placement
  caveat carried in docs + spec: https rides `std::io::tls`, whose
  recv blocks the worker thread (no async_io park yet) — keep
  https-calling loci off `async_io` pools. Not in v1: redirects,
  proxies, compression, URL-decoding.
- **Form-design finding recorded:** the connection pool deliberately
  is NOT `@form(lru_cache)` — an fd-owning cache needs an eviction
  hook (the evicted fd must be closed, not dropped) and
  take-semantics (ownership transfers out on hit), neither of which
  the form offers. Logged in-code and in `spec/stdlib.md` as
  feedback for a future forms arc.

Docs: the everyday HTTP chapter gained a Routing section and its
"calling out" section now teaches the stdlib client; `spec/stdlib.md`
carries both contracts. New coverage: `http_router.rs`,
`http_client.rs`, corpus fixture `69-http-router`.

## v0.11.3 — five language gaps + the unified styleguide (2026-07-17)

A gap-closing release driven by a survey of five production Hale
codebases: the recurring footguns and missing surface they converged on,
closed in one arc, then the styleguide rewritten around what now exists.

### Memory: plain self-field stores retire + single-owner value semantics

- **`self.<field> = Struct { ... }` replaces now reclaim.** The anchor
  retirement shipped for `@form(hashmap)` cells extends to plain
  self-field stores: a whole-value replace memcpys the struct bytes in
  place and *retires* each replaced String clone at the enclosing
  method's activation boundary (`lotus_str_field_replace_fixup`), so
  the clones recycle on the next store. Previously each replace orphaned
  the old clones in the locus lifetime arena forever — the leak every
  production codebase mitigated by hand with in-place scalar mutation
  and construction-position idioms. Validated: 1M whole-struct replaces
  (two fresh String clones each) hold RSS exactly flat; 200k mixed
  alias/RMW/grow churn clean under ASan+UBSan. Direct `self.f = String`
  reassignment also retires its abandoned buffer on the grow path.
  String leaves only at v0.1 — structs carrying `Bytes` / nested
  compound fields keep the prior behavior, and stores looping directly
  inside `run()` (no activation boundary) still accumulate.
- **Found en route, fixed: same-arena stores could alias two fields to
  one buffer.** The clone same-arena skip let `self.g = self.f` (on the
  non-fitting path) and struct literals embedding a `self.<field>` read
  *share* the source slot's buffer — the source's next in-place
  overwrite silently mutated the other field (broken value semantics,
  reproducible on prior releases with concat-built strings), and
  retirement would have upgraded that to use-after-free. Every
  self-storage store path now enforces single ownership: a same-arena
  incoming pointer that isn't the slot's own old pointer is
  force-copied. Fresh values, statics, and RMW round-trips keep the
  zero-copy paths. Regression suite `self_field_alias.rs` + corpus
  fixture `66-self-field-retire`.
- **The unbounded-alloc survey learned retirement.** A whole-field
  replace of an all-scalar/String struct in a method is no longer
  reported as unbounded accumulation (it provably reclaims); the
  conservative verdict stays for Bytes/nested-compound fields,
  `run()`-loop-direct stores, and scratchless owners.

### Bus: String routing keys

- **`keyed_by` accepts `String` fields; `where key == self.<String>`
  works.** The registry stores the subscriber key's FNV-1a-64 hash plus
  its own copy of the string (capture-by-value, per the existing key
  stability rule — required, since the subscriber's field may be
  reassigned and its old buffer retired); the publish site passes the
  payload field's hash, and only a hash match pays a full string
  compare — a mismatched key still costs one integer compare per entry.
  No dispatch ABI change; remote fanout stays unkeyed (no key material
  crosses a process boundary). Name-keyed fan-out (rooms, symbols,
  topics) now routes on the bus instead of filtering in handlers — the
  README chat room drops its `if m.room == self.name` line for
  `keyed_by room` + `where key == self.name`. `StringView` / `Bytes`
  keys stay rejected. Validated: exact-count routing over 50k keyed
  publishes, ASan-clean; capture-by-value semantics regression-tested
  against retirement.

### Language: `match` in expression position

- **`let x = match n { 0 -> 10, _ -> 20, };` now compiles.** The form
  parsed and typechecked but had no codegen (`Unsupported("expression
  form ...")`) — the docs' control-flow chapter already showed it. The
  lowering shares the statement form's full pattern machinery (literal /
  binding / wildcard / tuple / enum-constructor patterns, guards, block
  arm bodies) and phi-merges arm values, mirroring if-expressions.
  Typecheck now types the expression as the join of its arm types with
  a proper spanned mismatch diagnostic (statement-position arms remain
  heterogeneous-legal); F.18 exhaustiveness applies in both positions.
  The one reachable no-match case (every arm guarded, all guards false)
  yields the result type's zero value — defined, never poison. Zero new
  syntax. (`else if`, String-scrutinee match, and enum payload
  destructuring all predate this release — a survey finding was that
  production code ladders `} else { if` simply because the features
  were younger than the code.)

### Checks: `@hot` certification + handler-context lint + accept/release

- **`@hot fn` — hot-path certification.** Promotes the hot-path
  allocation lint's findings inside that fn to hard errors and enables
  two stricter perf hints: `.snapshot()`/`.finish()` in a loop (prefer
  the zero-copy `.view()`), and whole-struct self-field replace
  (reclaimed now, but in-place scalar mutation is still
  allocation-free). Stacks with the counted contract:
  `@hot @budget(alloc_per_call = 0) fn send(...)`.
- **The hot-path lint understands bus handlers.** A locus /
  `BytesBuilder` instantiated *anywhere* in a bus handler (not just a
  loop) warns — a handler runs per message, so that's a fresh arena
  per frame (~4.5 KB/frame measured downstream). Plain methods at
  depth 0 stay silent.
- **`accept` without `release` on a daemon warns.** Every accepted
  child of a release-less parent is resident until the parent
  dissolves; when the parent's `run()` loops forever that's unbounded
  growth. Deliberately narrow daemon signal (a literal `while true`),
  so run-to-exit programs accepting bounded batches stay silent. Zero
  new diagnostics across the 81-example corpus.

### Coverage: raw-fd TCP free fns

- The historically-reported native crash in `std::io::tcp::
  __send_bytes` / `__recv_bytes` does **not** reproduce on ≥ v0.10.0 —
  verified across pinned / classic cooperative / async_io placements,
  direct and wrapper-fn call shapes, under ASan, and against rebuilt
  v0.10.0 and pre-#215 binaries. The surface previously had zero native
  run coverage (the corpus oracle skips server fixtures); it is now
  regression-tested end-to-end (`tcp_raw_fd_freefns.rs`). Reminder:
  these free fns return `0` = success / `-1` = error, not a byte count.

### Docs: the unified styleguide

- **`spec/styleguide.md` rewritten** as the single author-facing guide:
  Foundations (the one-page memory model — the highest-leverage page a
  `.hl` author can read), the seven-shape catalog (new: the `@form`
  collection with a domain facade), correctness rules C1–C7 and speed
  rules S1–S12 each tagged with their enforcement status, the compiler
  enforcement ladder (default warns → `@hot` → `@budget`), and a
  de-staled gaps list. `agents/memory-patterns.md` folded in (now a
  pointer stub); README chat room rewritten to the keyed form;
  `docs/src/services/patterns.md` gains the reused-buffer connection,
  pre-render/fan-out, and event-driven ingest compositions. All guide
  examples compile-verified.

## v0.11.2 — recycle small replaced hashmap clones (2026-07-17)

- **Runtime: small `@form(hashmap)` replaced-value clones now recycle.**
  Anchor retirement reclaims a hashmap slot's replaced String clones at
  the activation boundary, but its reuse freelist stored the free node
  *inside* the dead block (16 bytes), so blocks under 16 bytes couldn't
  carry it and were dropped at flush — a short replaced value or key (a
  `"12.3"`, a `"sig.4"`) never recycled. On a continuously-churned
  recorded-state map (one keyed replace per delivered frame) that leaked
  ~50–128 B/frame, linear, no plateau — measured downstream at ~128
  B/frame on a long-lived subscriber connection. Fix: blocks under 16
  bytes recycle **out-of-band** via their shell node (nothing is written
  into the block, so it is sound at any size), and `lotus_str_clone`
  drops its 16-byte allocation floor so the recorded size equals the
  block size. A prior attempt to floor the *retire* size to 16 corrupted
  genuinely-small blocks (SEGV at high churn); the out-of-band approach
  avoids that entirely. Validated: a 1M-set churn of sub-16-byte values
  over a bounded key set stays at the RSS floor (was tens of MB), flat
  across 5 consecutive 30k-frame runs, clean under ASan+UBSan; the
  ≥16-byte in-band path is unchanged. See `notes/anchor-retirement.md`.

## v0.11.1 — Linux ARM64 release binary (2026-07-16)

- **Release: Linux ARM64 binary.** Releases now ship an
  `aarch64-unknown-linux-gnu` tarball (built on a native
  `ubuntu-24.04-arm` runner) alongside the x86_64 Linux and macOS arm64
  binaries — for aarch64 Linux servers (AWS Graviton / EKS arm64 nodes,
  Ampere). Toolchain packaging only; no compiler/runtime changes.

## v0.11.0 — substrate hardening + hot-path enforcement (2026-07-16)

Two arcs. First, a downstream service built on 0.10.0 filed a batch of
substrate findings — this release hardens the runtime against all of
them (async_io recv parking, cross-thread bus reclaim, exclusive binds,
fallible `Stream`, TLS socket-upgrade, and more). Second, a push to make
the compiler *enforce* the allocation-free hot path rather than leave it
to folklore — a lint, an opt-in `@budget` contract, coro-stack pooling,
and an ergonomic zero-copy UDP ingest handle.

### Hot-path enforcement

- **New stdlib: `std::io::udp::Reader` — the event-driven ingest
  handle.** `std::io::udp::Reader { addr, port, cap }` bundles a bound
  socket + a single reused `BytesBuilder`; `next() -> BytesView
  fallible(IoError)` parks on EPOLLIN on a `where async_io` pool
  (kernel-woken, no busy-poll, no timeout quantum) and returns a
  zero-copy view of each datagram aliasing the reused buffer. It's the
  hand-rolled "bind + `BytesBuilder` + `recv_into` + `.view()`" fast
  path baked into one handle so the allocation-free, event-driven shape
  is the default you reach for. Binds lazily on the first `next()` (so
  a bind failure propagates through the fallible channel); `dissolve()`
  closes the socket. Validated RSS-flat over 40k datagrams; unlike the
  allocating `recv` it copies no per-datagram payload.

- **Runtime: coro pooling on `async_io` pools.** Bus dispatch to an
  `async_io` subscriber previously `malloc`'d a fresh coroutine +
  64 KiB stack per delivery and freed it on completion. The pool now
  keeps a bounded per-worker free-list (cap 64) of completed coro
  slots and reuses them — a warm fan-out skips the per-dispatch stack
  malloc/free entirely. Measured **~640 vs ~729 ns/dispatch (~12%)**
  on a 300k-message single-subscriber flood, stable run-to-run. The
  free-list is worker-thread-local (no lock), drained at pool
  teardown; steady-state RSS retains up to 64 × 64 KiB (~4 MiB) per
  async pool. Correctness validated under ASan+UBSan+LSan (a
  20k-dispatch flood and the full corpus oracle). Transparent — no
  surface change.

- **New default-on advisory: hot-path allocation lint.** `hale check`
  now warns on two loop-scoped anti-patterns: a **locus** or a
  `std::bytes::BytesBuilder` instantiated inside a loop (a fresh arena
  / heap buffer every iteration — hoist it to a reused field), and an
  **allocating `recv`** (`recv` / `recv_bytes` / `recv_with_source`)
  in a loop (use `recv_into` with a reused `BytesBuilder`). Both
  accumulate in the method scratch until the enclosing method returns,
  and a `run()` read loop never returns. A plain value struct/type
  literal isn't flagged, and an instantiation outside a loop isn't —
  only the unambiguous per-iteration case. Warning, never a build
  failure.
- **New opt-in contract: `@budget(alloc_per_call = N)`.** The dual of
  `@unbounded` — an explicit per-call allocation ceiling on a `fn`
  (free or method), enforced as a **hard error**. The compiler counts
  the arena allocations it can see (literals, `@form` inserts) —
  transitively through resolved callees — plus the known-allocating
  `recv` family, and errors if the fn allocates more than `N` per
  call; a loop-nested allocation, a call to an allocating fn in a
  loop, or recursion is unbounded per call. `N = 0` is the zero-alloc
  certificate for a per-datagram handler or decode helper. fn-only;
  mutually exclusive with `@unbounded`. A violation reports the
  measured count and pinpoints every offending allocation with the
  fast-path fix. Reuses the item-1 (`--dump-alloc-summary`) allocation
  summary + call graph. See `spec/verification.md`,
  `docs/src/systems/performance.md`.

### Substrate hardening (downstream-handoff fixes)

Eight substrate findings from a downstream service built on hale
0.10.0; six fixed here, two filed as issues.

- **`recv_into` now parks on `async_io` pools (timed park).**
  `std::io::tcp/udp/tls` `recv_into` / `recv_stamped_into` (and
  `recv_bytes`) on a `where async_io` pool park the coroutine on
  epoll until the fd is readable or the fd's `set_recv_timeout`
  deadline expires — `-2` again means "deadline expired" on every
  pool type, never an instant would-block. Fixes pond/websocket's
  liveness machinery tearing down every idle connection on
  async_io pools. Two contract alignments: `recv_bytes` now honors
  `set_recv_timeout` on async_io (it parked indefinitely before),
  and `udp::recv_into` returns `-2` retryable on timeout (was `-1`
  fatal).
- **`std::http::Server` reassembles split-written requests.** The
  per-connection loop reads to the header terminator, then to
  `Content-Length` body bytes, so python-urllib-style clients
  (headers and body in separate segments) work. New guards: 1 MiB
  request cap (413 on declared overflow) and a 5s recv timeout.
- **New warning: cooperative pool starvation.** Two or more
  statically non-returning `run()` bodies on one cooperative pool
  (including fields with no placement entry and the main locus's
  own `run()`) warn naming every offender — the second-born
  `run()` never starts, and the failure was silent.
- **`self.<scalar>` in nested-literal param defaults works.**
  `conn: Ws = Ws { conn_fd: self.fd }` now resolves `self`
  lexically (the declaring locus) even when the instantiation
  happens inside another locus's method body; call-site overrides
  keep resolving to the caller (F.4). A default reading a
  later-declared sibling is now a compile error instead of an
  uninitialized read.
- **Unbounded-alloc lint: `fail`/`return` payloads in loops no
  longer flag.** Both diverge — the payload allocates at most once
  per invocation. Removes the false-positive class on strict
  parsers (`fail E { … }` inside `while`).
- **Parser: reserved keywords in binding position are named.**
  `let accept = …` now says ``expected variable name, but `accept`
  is a reserved lifecycle keyword in Hale — pick another name``.
- **BREAKING: `Stream.send` / `send_bytes` / `recv` / `recv_bytes`
  are `fallible(IoError)`** (#209, finding 5). Every call site
  must address the error (`or raise` / `or discard` / `or
  <fallback>` / `or handler(err)`). send/send_bytes succeed with
  Unit (the old Int was only ever a 0/-1 status). recv/recv_bytes
  fail **only on genuine I/O errors** — EOF and a
  `set_recv_timeout` expiry still return empty, so liveness loops
  keep their shape. `IoError` is now declared in the stdlib seed
  and can be constructed / `fail`ed from user code. Bonus:
  `Stream.recv` joins the async_io timed park (its siblings got
  it in the recv_into fix above). Migration for sentinel-checking
  callers: `let n = s.send(x); if n < 0 {…}` becomes
  `s.send(x) or handler(err);`.
- **Fixed: SIGSEGV under cross-pool ingest load** (downstream
  handoff 2026-07-15) — three layered runtime bugs:
  (1) the global cooperative queue now drains **only on its owner
  thread** — a pinned publisher's scope-exit flush used to execute
  main-pool subscribers' handlers on the publisher's thread,
  concurrently with main's drains (two threads in one locus);
  (2) `lotus_arena_retire_str` records the honest blob size —
  the old 16-byte floor let the freelist flush write a 16-byte
  node over smaller same-arena-skipped concat/slice blobs
  (heap corruption at high `indexed_by` churn, even
  single-threaded);
  (3) non-flat bus payloads for **cross-thread** subscribers are
  now enqueued as wire bytes and deserialized into the
  subscriber's arena on its OWNER thread at drain — dispatch used
  to deserialize into foreign arenas on the publisher's thread
  (TSan-verified race). Same-thread publishes keep the
  deserialize-at-dispatch fast path. See spec/runtime.md
  § Owner-executed handlers.
- **Fixed: P0 memory leak on cross-thread bus dispatch to a
  parked `async_io` subscriber** (downstream handoff 2026-07-15).
  The owner-routed wire-cell path above deserialized each
  delivery's payload straight into the subscriber's locus arena —
  fine for a subscriber that dissolves, but a per-delivery leak on
  a long-lived one whose `run()` is parked forever (the canonical
  accept/recv server loop): the arena never dissolves, so every
  message's String/Bytes fields accumulated unboundedly (~320 MiB
  over 20k 16-KiB deliveries; flat afterward). Each wire cell now
  deserializes into a per-delivery subregion destroyed the instant
  the handler returns. Retention patterns are unchanged —
  `self.saved = msg` still deep-copies into the locus arena. Only
  the leaking cross-thread wire path is affected; same-thread and
  main-pool delivery were never impacted. See spec/memory.md
  § Cross-thread wire cell per-delivery reclaim.
- **Fixed: N readers can share one `async_io` pool** (downstream
  handoff 2026-07-15, item 3). The Bytes-returning
  `std::io::udp::recv` / `recv_with_source` did a blocking
  `recvfrom`, pinning the single pool worker inside the syscall —
  so a second reader locus's `run()` queued behind it on the same
  pool never started (with no recv timeout, never at all; the
  drain otherwise hung at shutdown). They now park on EPOLLIN like
  the tcp/tls siblings, bounded by the socket's `set_recv_timeout`
  deadline (or indefinitely when unset), yielding the worker so
  every reader parked on its own socket is serviced concurrently.
  Also fixes a latent use-after-free the concurrency exposed: a
  coro's caller-arena (where its stdlib allocations land) is now
  snapshotted across a park and restored on resume, so a resumed
  reader no longer allocates through an arena a sibling coro tore
  down while it was parked. See spec/runtime.md § `where async_io`
  and spec/stdlib.md `std::io::udp`.
- **BREAKING: TCP listeners bind exclusively** (downstream handoff
  2026-07-15, item 4). `std::io::tcp::listen_socket` (and the
  `Listener` / `http::Server` that use it) no longer set
  `SO_REUSEPORT` — only `SO_REUSEADDR`, which still covers the
  restart-within-`TIME_WAIT` case. `SO_REUSEPORT` let two live
  processes both bind the same host:port and have the kernel
  round-robin connections between them, so a second server booted
  by accident got no error and clients were silently split-brained
  across two divergent-state processes. A second live bind now
  fails with `EADDRINUSE`, matching Go/Rust. Only affects the
  accidental-dual-bind case; a single server is unchanged.
  Intentional multi-process port sharing would need an explicit
  opt-in (none today). See spec/stdlib.md `std::io::tcp`.
- **Fixed: unaddressed fallible `Stream` call is a clean error, not
  an LLVM ICE** (downstream handoff 2026-07-15, item 5). After #209
  made `Stream.send` / `send_bytes` / `recv` / `recv_bytes`
  `fallible(IoError)`, a call site that omitted the `or` clause (a
  bare statement or a plain value-binding) reached codegen's
  non-fallible method-call lowering and emitted a call to the
  fallible callee with the wrong arity — surfacing only as `module
  verification failed … Incorrect number of arguments passed to
  called function`. The typechecker can't catch it because a
  `std::io::tcp::Stream` literal types as `Unknown` there (stdlib
  handle loci aren't in the type table), so codegen now rejects the
  call by name: `error not addressed: \`std::io::tcp::Stream.send_bytes\`
  is fallible — handle its error with an \`or\` clause`. A
  typecheck-time diagnostic would need stdlib handle loci in the
  type table (a larger follow-on); this removes the ICE, which was
  the defect.
- **Fixed: two `@form` instances on two pools no longer need twin
  types** (downstream handoff 2026-07-15, item 5). The F.31
  cross-pool-method check pinned a `@form` (or any) locus **type**
  to one pool (first placement seen), so two loci that each held
  their own field of that type on different pools false-flagged
  every owner but the first with a "cross-pool method call" error —
  forcing byte-identical twin types as a workaround. The receiver's
  pool is now inferred per **instance** at the call site: the
  enclosing locus's own placement of the field, else the field
  co-locates with its owner. Two separate `self.<field>` maps, each
  touched only by its owner's pool, are single-threaded and no
  longer flagged (they never needed a sync discipline). A genuine
  cross-pool access — a form field explicitly placed off its owner —
  still flags and still carries the sync-discipline hint.
- Filed as an issue: implicit error propagation on tail-position
  `return` (finding 8).

---

## v0.10.0 — topology-aware placement + perspectives (live redeploy)

- **Topology-aware placement (Phase 1).** Describe the host machine
  and map loci onto its NUMA/cache/core hierarchy, memory co-located
  to the thread. `pinned(cores = A..B | A..=B | {a, b, c})` sets a
  thread's affinity mask to a core *set* (a range carves out an
  isolation domain); a `topology { }` block declares the
  socket → NUMA node → L3 domain → core hierarchy with
  `pinned(node = N)` / `pinned(l3 = name)`; a node-pinned locus
  allocates its *arena* on that node via a raw `mbind` (no libnuma
  dependency) — the thread+memory co-location payoff; and
  `replicas = K` fans a locus into K single-threaded instances, one
  per core in the range (parallelism as more single-threaded units,
  so the lock-free / devirtualization invariants survive). Linux-only
  optimization; degrades to advisory no-ops on macOS/other. Opt-in —
  existing placement lowers byte-identically.

- **Perspectives — live redeploy (Phase 2–3).** A perspective is now
  a first-class, live-rebindable handle to a *contract*: program
  against a stable ABI (`serves`) reached through a single swappable
  slot, and `reperspective` swaps the implementation behind it at
  pointer-flip cost — no restart, no global pause. Bus
  subscribe/publish edges are part of the swappable contract and
  re-point across a swap; a layout-identity swap repoints code at the
  existing arena (zero data movement), while a changed footprint runs
  a `migrate`.

- **macOS (Apple Silicon) support — phase 1.** The runtime builds and
  runs on macOS 14. `async_io` is gated behind a clear compile
  diagnostic pending a kqueue backend, and Linux-only socket options
  (`SO_PRIORITY` / `IP_PKTINFO`) + CPU affinity degrade to no-ops.
  Prebuilt, reproducible self-contained Linux releases ship via
  Docker.

- **`@form(lru_cache)`** — a bounded LRU cache form.

- **`hale test`** — discover + run `*_test.hl` (see
  [`spec/testing.md`](./spec/testing.md)).

- **Anchor-retirement freelist double-free fixed.** A String-keyed
  `@form(hashmap)` whose value struct carries the `indexed_by` field
  aliases one clone as both the map key and that field; it was
  retired twice, self-linking the reuse freelist and crashing under
  multi-key churn. Retirement now dedups within the call; block reuse
  is preserved.

- **DI verifier fix — synthesized fn-exit epilogues now carry a
  !dbg location.** A fallible fn that dissolves a local locus at
  scope exit emitted the dissolve-cascade calls with no !dbg while
  the fn carried a DISubprogram — the DWARF verifier rejected the
  whole module ("inlinable function call in a function with debug
  info must have a !dbg location"). First reproducer: pond
  http/client's round_trip_oneshot (keepalive) dissolving its local
  HttpConn, which broke a downstream app build. The epilogue
  emitters now pin the LLVM-sanctioned synthetic location (line 0
  in the fn's scope) when the per-statement location was cleared,
  and unset it on completion so it can't leak into the next
  function ("!dbg attachment points at wrong subprogram").

- **Anchor retirement — the TP-3 leak class is fixed for
  @form(hashmap).** Overwriting or removing a map row used to orphan
  the old cell's String clones in the locus arena forever (the
  audit's biggest true-positive class: 53 corpus sites; a downstream
  service's marks/on_mark shape leaked per market-data frame). Now: sync=none
  string-celled maps carry a String-field offset descriptor
  (installed at instantiation from TargetData layout); set/remove
  retire the replaced clones (pointer-difference guarded, so the
  RMW key-reuse idiom and grow-rebuild stay no-ops); retired blobs
  flush to a size-classed freelist at the USER activation boundary
  (sound by the method-scratch argument — bytes stay intact while
  any legal holder can exist); `lotus_str_clone` reuses flushed
  blocks (16-byte floor so every clone can carry a freelist node).
  Steady-state churn (4M sets, 16 keys, fresh strings per set):
  4.8 MB flat RSS, was 207 MB. Synced maps, vec cells, and compound
  self-store retire are staged in notes/anchor-retirement.md.

- **Batched @form(hashmap) iteration — walk_large 0.30 → 0.82 vs
  Rust.** `for e in m.entries` now fills a 64-entry stack batch per
  C call instead of one call per element: plain (sync = none,
  single-pool) maps take a POINTER-mode batch (zero copies — the
  loop var references slot storage directly; sound because unsynced
  maps have no concurrent writers and mutation-during-iteration is
  already contractually unsupported), synced maps copy values out
  under one lock/epoch per batch. 100k-entry walk: 301 µs → 109 µs
  (and 5.3× ahead of the hand-written C comparator). The journey
  from the original key_at walk: 1.31 ms → 109 µs, 12×.

- **Typecheck: fallible stdlib calls rejected as direct `or`
  handlers.** `x() or std::io::fs::read_file(p)` compiled but
  silently yielded the un-addressed sret value ("" / 0) when the
  handler ITSELF failed, instead of propagating — found while
  compile-testing doc examples. Now a typecheck error with the
  exact rewrite ("write `or (std::io::fs::read_file(p) or raise)`
  so its own failure has a path") until the codegen handler
  classifier covers stdlib paths. Zero hits across pond + downstream
  apps + examples.

- **Aliasing stage 2 (tier 1) — `noalias self` on provably
  non-reentrant locus methods.** Rust's `&mut`-style guarantee,
  earned from Hale's own invariants: a method in the elidable
  fixpoint (non-allocating ⇒ cannot publish, and its callees never
  drain the cooperative queue) with all-scalar params cannot be
  re-entered through the bus registry nor handed an aliasing
  pointer — so `self` is `noalias` and field loads can stay in
  registers across calls. MODES join the elidable fixpoint under
  their synthetic names (bulk/harmonic/resolution — the brain-tower
  pull surface — qualify, and sibling `self.bulk()` calls now
  classify non-allocating for scratch elision too). Contract pinned
  by IR tests (positive + both unsound channels stay unmarked).

- **Builds are 2.3–5.8× faster: dead-stdlib elimination before the
  backend.** Every module carries the full merged stdlib; it was
  being O3-optimized and machine-emitted on every build, used or
  not (224 ms of a 462 ms trivial build). Defined fns except `main`
  are now internalized and a leading `globaldce` strips the
  unreferenced stdlib before the pipeline runs. Trivial builds
  462 → 80 ms; the largest app 1.2 s → 526 ms. Plus:
  `HALE_TIME=1` prints per-phase wall times; `hale build --dev`
  (or HALE_DEV=1) selects an O1 pipeline for latency-critical
  loops; `hale check --json` emits NDJSON diagnostics on stdout
  (file/line/col/severity/kind/message) — with `hale check` at
  ~10 ms on the largest apps, this is the LSP groundwork: an
  editor save-hook needs nothing more. The staged rest (prebuilt
  stdlib object, `hale lsp`, per-seed caching) is in
  notes/build-latency-and-lsp.md.

- **Unbounded-allocation warnings are DEFAULT-ON.** (M3 stage 5
  complete — Riley's flip call after the full-corpus audit.) Every
  `hale check`/`build` now surveys the whole program; run-to-exit
  programs (a `main` with no `run` loop and no bus handler) warn
  nothing, `@unbounded fn` stays the carve-out, and
  `--no-warn-unbounded-alloc` is the opt-out (the old
  `--warn-unbounded-alloc` spelling is accepted-and-ignored).
  Warnings never fail the build. Expect real findings on the
  downstream daemons: the audit confirmed 103 true accumulation
  sites across them and the pond libraries — that visibility is the
  point of the flip.

- **M3 stage 5 (part 2) — run-to-exit programs don't warn; a
  tempting loop-bound extension rejected by the empirical model.**
  A program whose bundle has a `main` but no `run` loop and no bus
  handler is run-to-exit — per the tool's own philosophy it owes no
  memory-bound proof, so smoke binaries and scripts no longer warn
  (the model still ranks their sites; only the diagnostic surface
  is gated). Libs checked standalone (no `main`) keep ALL warnings —
  per-dir consumer checks don't re-bundle vendored libs, so the lib
  check is where pond/websocket's real per-message leaks surface.
  Also documented in-code: ranking runtime-invariant loop ceilings
  (len()/params) as bounded was implemented and REVERTED — the
  RSS-validated test is the authority that a param-ceiling loop in
  a scratchless frame accumulates linearly in the input (3M iters ≈
  190 MB), which is exactly what unbounded means here. Warning
  totals across the corpus: 402 (pre-audit) → ~160, all audited
  true positives preserved. Default-on remains blocked at ~36%
  residual FP (accepted D/E-lib/F limitations + one-shot-shaped
  app code) — the flip is now a policy call, not an engineering
  gap.

- **M3 stage 5 (part 1) — unbounded-alloc analysis: audited + three
  gap fixes.** A fresh-context audit triaged all 402
  `--warn-unbounded-alloc` warnings across pond + downstream apps +
  examples: 103 true (26%) — including live production leaks (a
  downstream service's `marks.set` per md frame, pond websocket's
  `last_message.kind` per message; the per-set anchor-clone class is
  filed as a downstream runtime issue) — and 299 false (74%). Three
  classifier gaps fixed:
  (A) `Returned` values consumed inside a member fn's per-call
  scratch no longer flag — only returns consumed by a scratch-less
  long-lived frame (`main`/`run`/free-fn chains therefrom) accumulate;
  (B) in-loop `Local`s in scratch-ful frames are bounded per
  activation (reclaimed at method exit) — EXCEPT inside a literal
  `while true`, where the exit never comes;
  (C) whole-value `self.field = Struct{...}` replaces whose inits
  are all scalar/static-literal are in-place memcpys, not arena
  growth (a single fresh heap subfield re-flags — that's the
  anchor-clone leak).
  Result: ~402 → ~165 warnings with every audited true positive
  preserved (downstream-app counts audit-exact);
  bounded[T; N] eviction loops no longer warn. Remaining for
  default-on: len()/param loop-bound recognition (the ~35% residual
  FP is main-reached runtime-bounded loops) and the accepted E/F
  limitations (one-shot binaries, return-then-publish aliasing).

- **Typecheck M3 stage 3 (tranche 2) — generic STRUCT literals +
  monomorph unification.** `Box_Int { ... }` literals now resolve
  against the generic template with the type args substituted:
  wrong-typed fields, unknown fields, and missing fields are caught
  at typecheck; field READS on monomorph values type as the
  substituted field (`b.value` on a `Box_Int` is `Int`). And
  `Box<Int>` type-exprs now resolve to the mangled monomorph name
  (previously the bare `Box`), so a `Box<Int>`-typed field and a
  `Box_Int` literal unify — and a `Box_String` literal in a
  `Box<Int>` slot is a caught mismatch. This also FIXES generic
  structs being unusable through the CLI: `hale check` rejected
  every mangled-monomorph literal as "unknown type", so only
  codegen unit tests (which skip the checker) could use them.

- **Typecheck M3 stage 3 (tranche 1) — generic fn call validation.**
  Call sites of generic fn templates are now checked at typecheck
  with source spans — the Ty-level mirror of codegen's m62
  inference: arity ("takes 3 arguments, got 2"), binding conflicts
  ("parameter `T` bound to both `Int` and `String` by this call's
  arguments"), unpinned generics ("cannot infer `T` from this
  call"), and args vs SUBSTITUTED param types. The call types as
  the substituted return (fallible payloads substituted too), so a
  generic call's result participates in downstream checking instead
  of passing through as Unknown. Permissive exactly where inference
  is blind (Unknown args, generic-arg'd nested shapes). Tranche 2:
  generic STRUCT literal field validation. Also fixed en route: a
  DWARF location leak at the mid-statement generic-synthesis site
  (the caller's active location poisoned the synthesized fn's entry
  allocas — "!dbg attachment points at wrong subprogram" — on any
  debug-info build using generics).

- **bounded[T; N]: `set(f, i, x)` + `truncate(f, n)` intrinsics.**
  `set` overwrites a live slot (fallible IndexError, arena-anchors
  pointer-shaped elements like push); `truncate` clamps the count
  down (never grows; returns the new count). Together they make the
  drop-front/FIFO idiom expressible — shift live slots left with
  set, then truncate — which unblocked migrating
  pond/agent/conversation's history eviction off its TSV walker.

- **`bounded[T; N]` — fixed-capacity counted collections in types.**
  Types can now hold a real bounded collection instead of the
  delimited-string workaround: `type Recent { vals: bounded[Int;
  32]; }` lays out inline as `{ i64 len, [N x T] }` (capacity is
  part of the type — K made value-level per F.22). The operations
  are grammar INTRINSICS, not methods, so the types-are-pure-data
  axiom holds: `push(f, x)` (fallible `CapacityError { cap, count }`
  when full — displacement policy lives in the caller's `or` arm),
  `at(f, i)` (fallible IndexError), `count(f)`, `clear(f)`, and
  `for x in f` iterates the live slots. Fields auto-initialize
  EMPTY — literal init and whole-field assignment are rejected
  (the intrinsics are the only mutation surface). Works in `type`
  fields and locus `params`; whole-struct copies carry elements and
  count by construction; scalar-element bounded is flat under
  `zero_copy`. v1 covers scalar elements (Int/Float/Bool/Decimal/
  Duration) AND pointer-shaped elements — `bounded[String; N]`,
  `bounded[Bytes; N]`, `bounded[SomeStruct; N]` (stage 1, same
  day): push arena-anchors each element into the receiver's owning
  arena (a scratch-built String pushed from another fn survives —
  the same-arena gates make re-anchoring idempotent, no realloc
  storms), and whole-struct copies anchor live slots with a runtime
  [0, len) loop. `type RouteParams { keys: bounded[String; 16];
  ... }` replaces the pond TSV idiom directly. On the bus:
  scalar-element bounded travels as flat bytes; pointer-element
  bounded cross-process is post-v1 polish (focused reject).

- **Typecheck M3 stage 2, tranche 2 — signatures for the I/O
  namespaces + dual-mode fallible semantics.** 60 more rows:
  io::fs/file/tcp/tls/udp, process child management, text
  predicates, term/diag/os. Two semantic fixes the corpus forced:
  (1) stdlib fallible path-calls are DUAL-MODE at codegen — with
  `or` they use the fallible ABI, bare they're the legacy direct
  form with per-fn returns (read_file → the String, write_file →
  an Int status) — so bare calls now stay permissive (Unknown)
  while `or` positions get precise success/payload types from the
  table (the Or arm consults it directly); (2) a statement-position
  `call() or handler(err);` discards its value, so the fallback/
  handler-return type no longer needs to match the success type
  (a common production pattern). Handle args at the
  path-call level are plain Int fds. Still excluded-not-guessed:
  all std::json / std::http rows and process stdio (routed through
  Hale-stdlib __ fns — no codegen-level ground truth), the 7
  spec'd-but-unimplemented std::io::tls fns, tcp
  set_recv/send_timeout, io::file::write_line, io::fs::list_dir.
  Gate: zero new errors across pond, downstream apps, and examples; the
  three bring-up hits (a downstream app's refdata, pond logfmt, io-demo) were
  exactly the two semantic gaps above — all three now pass.

- **Typecheck M3 stage 2 — stdlib signatures for the scalar-heavy
  namespaces.** 118 functions across std::math/time/env/decimal/
  process(scalar)/str/io::stdin/io::stdout/bytes/crypto/
  text::base64/rand now have full signature rows: arity and arg
  types are enforced, and calls return their REAL type instead of
  the permissive Unknown — `std::math::sqrt("four")`,
  `std::math::pow(2.0)`, and `std::time::sleep(100)` (Int where
  Duration is required) are now typecheck errors with spans.
  Fallible rows return `Ty::Fallible`, so `parse_int(s) or ""`
  is caught (`or` substitute checked against the Int success type).
  The table's coercions mirror what each lowering actually does
  (verified per-fn): math sitofp-coerces Int args, every String
  position accepts StringView, readers accept the whole Bytes
  family. Uncertain rows are names-only, not guessed —
  str::builder_* (opaque handles) and can_parse_decimal (in the
  spec, NOT in the dispatch — spec bug, flagged). io::fs/tcp/tls/
  udp/file are the string-heavy tranche 2. Gate: zero new type
  errors across pond, downstream apps, and the example corpus (the two hits
  found were verified pre-existing at the unmodified baseline).

- **Typecheck M3 stage 4 — expose-side contract validity + exposed-mode
  syntax.** Every `expose` entry must now bind against something real
  on the declaring locus — a params field, a mode, or a `fn` member —
  at a matching type. Previously `expose no_such_field: Int;` and
  `expose value: String;` over an Int field compiled silently (codegen
  treats contract members as pure declaration, so typecheck is the
  only enforcement point) and a consuming parent type-checked against
  fiction. The consume-side checks (missing expose, type mismatch,
  consume-without-accept) already existed. Also: mode keywords are now
  admitted in contract-name position (`expose bulk: Float;`), making
  the spec's exposed-mode pull rule (semantics.md — a parent may call
  a child's mode iff contract-exposed) expressible for the first time;
  the exposed type is checked against the mode's declared return.
  Gate: zero errors across pond, downstream apps, and the example corpus (51
  real contract lines, including pond websocket).

- **Typecheck M3 stage 1 — stdlib typo detection.** A call to an
  unknown function in a TABLED `std::` namespace is now a typecheck
  error with a did-you-mean (`std::str::parse_itn` → "did you mean
  `std::str::parse_int`?"). The table covers 26 namespaces
  (mechanically extracted from the codegen dispatch's
  `["std", ...]` patterns, unioned with spec/stdlib.md); namespaces
  with non-literal dispatch (io::sockopt, io::mirror, shm, ts) stay
  permissive, so table incompleteness degrades to the old Unknown
  behavior, never to a false error. Gate: zero new errors across
  pond, downstream apps, and the full example corpus. This is the first slice
  of the M3 plan (notes/typecheck-m3.md); signatures (killing the
  Unknown returns) are stage 2.

- **@form iteration surface — `for e in m.entries` / `for x in
  v.items`.** Hashmap iteration lowers to a cluster-aware
  slot-cursor walk (`lotus_hashmap_iter_next`): O(cap) for a full
  walk, where the index-based `key_at`/`entry_at` pair rescans from
  slot 0 per element (O(cap×len) — the quadratic behavior that put
  form_hashmap_walk_large 13× behind Rust). Vec iteration is a fully
  inline buf walk with zero per-element calls. Loop var is a copy
  (hashmap) / reference-to-cell (vec struct cells); mutation during
  iteration is unsupported; break/continue work. Measured on
  walk_large (100k entries): 1.22 ms → 0.30 ms — 4× faster and now
  1.9× ahead of the hand-written C comparator; Rust's SwissTable
  iterator still leads 3.4× (one C call per element remains — a
  batched iterator is the follow-on). Ring iteration deferred.

- **Fn-call protocol at C shape — exit-drain elision + fn-pointer
  classifier refinement.** Two changes driven by the first Rust/C bench
  comparators (fn_call/fn_modular ratio was 0.40 vs all three):
  (1) a proven-non-allocating body cannot have published (payload
  copies allocate), so its scope-exit flush skips the per-call
  `lotus_bus_queue_drain` when the deferred-dissolve frame is also
  empty — fn exit is NOT a spec-required yield point (handler exits,
  lifecycle transitions, `yield`, and `sleep` still drain). A
  minimal free fn drops from `push+lea+load+call drain+pop+ret` to
  `lea; ret` — literally C's shape. BEHAVIOR NOTE: a cooperative
  compute-only loop that relied on helper-call exits as its delivery
  points never had that guarantee by spec and now won't get it —
  use `yield;` (that's what it's for).
  (2) a call through a fn-pointer PARAM with a numeric-scalar return
  no longer marks the caller allocating: the callee scratches off the
  threaded caller arena and a scalar return leaves nothing behind —
  callback-style code (`fn outer(x: Int, g: fn(Int) -> Int)`) stays
  elidable instead of paying subregion+drain+destroy per call.
  Measured (opaque-pointer bench variants, ratio vs clang -O3 C):
  fn_call 0.40 → 0.77, fn_modular 0.40 → 0.98 (15.77 ms vs C's
  15.4 ms — parity). The bench .hl files now call through
  pid-selected opaque fn pointers (Hale has no noinline surface; the
  direct-call versions inline + fold to nothing post-elision).

- **Fallible `or` handlers — `call() or handler(err)` now accepts a
  handler that is itself `fallible(E2)`.** The handler's success value
  substitutes; its failure propagates through the ENCLOSING fn's error
  path (implicit `or raise` — sugar for the already-legal nested form
  `call() or (handler(err) or raise)`). E2 must be assignable to the
  enclosing fn's fallible payload; targeted diagnostics otherwise
  ("handler's failure has nowhere to go" / "propagated payload must
  match"). Free-fn, imported-path, and locus-member handlers are
  classified; `@form` synthesized methods and stdlib path-calls still
  need the explicit nested spelling. This closes the pond stash-bridge
  idiom: `jobs::Queue`'s DbError→JobError conversion no longer needs
  private stash fields, removing its non-reentrancy hazard.

- **DWARF debug info — `hale build` binaries now carry line tables for
  Hale code and full debug info for the runtime.** Every statement gets
  a file:line location (emission kind LineTablesOnly, DWARF 5); the
  lotus runtime TUs compile with `-g`. gdb sets breakpoints on `.hl`
  lines, backtraces show `FxL.at () at inlarr.hl:7` with inline frames,
  addr2line resolves Hale addresses, and ASAN reports carry real
  file:line through both Hale and runtime frames. Zero runtime cost —
  frame pointers are deliberately NOT forced (measured +22% on
  bus_dispatch from `-fno-omit-frame-pointer` on the runtime's
  dispatch fast paths); profile with `perf record --call-graph dwarf`.
  Opt out with `LOTUS_NO_DEBUGINFO=1`. Stdlib and synthesized `__*`
  helper bodies carry no line info (their spans live in other
  coordinate spaces); `__lib_*` cross-seed imports keep theirs. The
  module is verified whenever debug info is enabled, so a codegen
  location bug surfaces as a readable error (dumped to a .ll file)
  instead of a backend abort. Implementation notes: statement
  locations are managed by a save/restore stack that never restores a
  location across a function boundary (mid-expression fn synthesis),
  and `alloca_in_entry`'s `position_before` — which silently ADOPTS
  the target instruction's empty location per LLVM's SetInsertPoint
  semantics — re-asserts the statement location after repositioning.
  Inkwell's `get_current_debug_location` is avoided entirely (its
  legacy value-based API materializes an empty MDNode for "none",
  which then verifier-fails as `!dbg !{}`).

- **Inline fixed arrays — scalar `[T; N]` fields are now laid out inline
  in their containing struct.** Previously every array field lowered to
  an out-of-line arena pointer, so a "flat" struct with an array field
  was secretly `{…, ptr}`: `is_flat_shapeable` said flat, the shm slot
  carried a dangling pointer cross-process (the bench xproc segfault),
  and every whole-value replace persisted a fresh copy in the locus
  arena. Scalar-element arrays (Int/Float/Bool/Decimal/Duration) are now
  `[N x T]` in the struct body; the array's SSA value is unchanged (a
  ptr to storage — field reads yield the slot address, field writes
  memcpy elements). Covers user types, locus params, struct literals,
  locus params-init, self-field reads/indexed assigns, the lvalue
  walker, deep-copy/anchor walks, and the m70 wire codec.
  `is_flat_shapeable` accepts scalar arrays again to match; non-scalar
  element arrays keep the out-of-line layout and stay rejected under
  `zero_copy`. Verified cross-process: the idiomatic
  `type Blob { tag: Int; data: [Int; 511]; }` round-trips a 4 KB payload
  over `shm_ring … where zero_copy` with a correct checksum — no more
  512 hand-spelled scalar fields. Whole-value scalar-array replace
  (`self.recent = […]`) no longer leaks a persisted copy per assign
  (~35 MB over 3M trips removed; the RHS literal's scratch growth in a
  single long activation remains and is still flagged by
  `--warn-unbounded-alloc`).

- **Accept'd-child struct recycling — churn daemons no longer grow by
  sizeof(child struct) per child.** Interest-based ownership (v0.9.2)
  allocates an accept'd/bubbled child's locus struct in the owner's
  arena so `owner.__children` reads stay valid cross-lifecycle — but
  arena allocations are never individually freed, so a churn shape
  (one flow child per connection/message) leaked ~100–200 B per child
  *forever*, O(total children ever) instead of the O(peak alive) the
  F.3 free-list contract promises. Reclaim (flow run-completion,
  `terminate;`, parent cascade) now pushes the dead struct onto an
  intrusive per-owner free-list (`lotus_child_struct_release`);
  instantiation pops a size-matched block before bump-allocating
  (`lotus_child_struct_alloc`). Covers both subregion-owning children
  and arena-elidable (empty-lifecycle) children. Measured: accept-churn
  at K=4M flat at 5.5 MB maxrss (was 443 MB). Resident children (no
  `release(c)` on the parent) still accumulate until parent dissolve —
  that's the documented flow-vs-resident semantics, not a leak.
- **Owner-arena child structs now allocated 16-byte aligned** (was 8):
  an accept'd child with a `Decimal` param could take a `movaps` trap —
  same genre as the 2026-05-20 arena-alignment fix.
- **Cross-seed locus-field whole-reassignment now takes the WS1#4
  lifecycle path.** `self.conn = wsx::Conn { … }` (qualified/imported
  RHS type) previously fell through the `segments.len() == 1` gate to
  the plain value lowering — the field ended up pointing at a
  method-scoped stack temp, the exact dangle WS1#4 exists to prevent
  (its cross-seed test only survived by benign garbage). Qualified
  paths now resolve through the import-rename table, same as
  statement-position instantiation.

## v0.9.2 — interest-based ownership (accept bubbling)

- **`accept()` now collects descendants, not just direct children — a locus
  bubbles to its nearest accepting ancestor.** When a locus `I{}` is instantiated
  somewhere its *direct* enclosing locus does not `accept(I)`, it now stitches to
  the nearest enclosing ancestor that does (innermost-wins), instead of falling
  through to a transient throwaway. A top-level `World` can `accept(Ship)` and
  collect every `Ship` spawned anywhere beneath it — past intermediaries that
  don't care about Ships — with no manual registration. It's the structural dual
  of the bus: where the bus is ephemeral *messaging*, this is ephemeral
  *ownership* (a live projection the ancestor iterates and reclaims).
  **Backward-compatible by construction:** innermost-wins picks the direct parent
  whenever it accepts, so no existing parent↔child relationship changes; the
  feature only *adds* an owner where a child was previously transient (the whole
  corpus is byte-identical with the feature on vs off). Ownership stays opt-in via
  `accept` — an `I{}` with no accepting ancestor is a transient locus, never an
  error. Resolution is fully static (no polymorphic instantiation → the
  closed-world graph fixes every owner edge at compile time; no runtime ancestor
  walk). Three tiers, each proven inert on shipped code and ASan-clean:
  - **Same-tower, singleton owner** — the owner (a `main locus` / `@export`) is a
    compile-time constant; bubbling lowers to direct pointer wiring + a projection
    append + the existing reclaim cascade. Zero runtime cost over direct parenting.
  - **Same-tower, multiple owner instances** — the owner pointer is threaded down
    the birth chain via hidden per-locus fields, giving **instance isolation**:
    two `World`s each collect only the entities in their own subtree.
  - **Cross-pool** — a consumer on a worker pool spawning into a main-thread
    registry. The child is born on the owner's thread via an async handoff over the
    bus queue (reusing the lock-free post+wake), so teardown stays the owner's
    same-thread cascade — no cross-thread reclaim. Necessarily **async
    fire-and-forget**: a cross-pool `I{}` may only be a bare statement; using the
    instance as a value is a compile error.
  `LOTUS_NO_OWNERSHIP_BUBBLE=1` disables the whole mechanism (used as the
  backward-compat differential).

## v0.9.1 — pinned-Decimal bus-payload alignment fix

- **Fixed a segfault when a pinned bus subscriber stores or does arithmetic on a
  received `Decimal`.** A `Decimal` (an inline `i128`, align-16) delivered to a
  *pinned* subscriber landed in an 8-aligned mailbox payload cell, so an aligned
  SSE access (`vmovaps`) `#GP`-trapped — silent UB on ordinary type-correct code
  in the hot path of any bus consumer carrying money. Root cause:
  `lotus_bus_cell_t.payload_inline` had only the cell's natural align 8 (its
  widest member is a pointer), and the pinned drain hands the handler
  `&cell.payload_inline` directly — whereas a cooperative drain copies into a
  16-aligned scratch, which is why only the *pinned* path crashed. (It looked
  flaky because at `-O3` LLVM scalarizes individual i128 *field* ops into
  misalignment-tolerant paired 64-bit moves, so only a whole-struct payload copy
  reliably tripped the aligned `vmovaps`.) Fix: force the mailbox cell to 16-byte
  alignment (one struct attribute makes every cell copy 16-aligned uniformly), and
  bump the two nested-struct wire-deserialize allocations from 8 to 16 (a latent
  trap for remote/cross-process payloads carrying a nested Decimal-bearing struct).
  The downstream "never hold a bus-received Decimal — `to_string` it at the seam"
  workaround is no longer needed. Regression test: `bus_decimal_store` — three
  pinned-subscriber cases (`@form(vec)` push, `@form(hashmap)` cell, plain `self`
  field) asserting the *exact* round-tripped values + an accumulated sum, ASan-
  clean; SIGSEGVs on the pre-fix compiler.

## v0.9.0 — lock-free bus, static dispatch devirtualization, native codegen

- **Lock-free bus messaging + static dispatch devirtualization — coordination
  is no longer the weak spot.** The pinned-locus mailbox and cooperative-pool
  queues are now lock-free MPSC rings (Vyukov bounded ring + signal-only-when-
  parked wake, genmc-verified) in place of the per-message mutex + `cond_broadcast`
  handoff; and statically-eligible local bus subjects (closed-world programs, no
  transport adapter / wildcard / cross-seed) skip the `g_bus_entries` registry
  scan + the runtime dispatch entirely — a *quiet* same-thread handler (mutates
  only its own `self`, no I/O, no republish) is lowered to a **direct synchronous
  call**, proven byte-identical to the deferred dynamic path by a differential
  test harness. Net on the bench grid (vs Go): `bus_dispatch` went from ~4× behind
  to **2.4× ahead** (1.79 ms → 196 µs), `bus_dispatch_cross_pool` from 1.6× behind
  to **1.26× ahead** (10.7 → 5.0 ms), `stream_aggregator` from ~23× behind to **1.9×
  behind** (5.26 ms → 436 µs), `pipeline_3stage` ~2.4× faster. Footprint trade-off:
  the lock-free rings **pre-allocate** their cap (~4.3 MB per pinned mailbox /
  cooperative pool at the default 8192) rather than growing — lower
  `LOTUS_BUS_QUEUE_CAP` for pinned-/pool-heavy programs (see `spec/runtime.md`).

- **Native-tuned codegen + O3 by default, with `--target-cpu native|baseline`.**
  A native `hale build` now tunes generated code to the host CPU (autovectorization,
  AVX-512 where the host supports it — carried via per-function `target-features`)
  and runs LLVM's aggressive (O3) pipeline. **Consequence:** native binaries are no
  longer portable across microarchitectures — build distributed artifacts with
  `--target-cpu baseline`, which pins a portable `x86-64-v3` (AVX2 + BMI2 + FMA).
  `wasm32` is unaffected (stays generic / O2).

- **`LOTUS_LTO=1` — opt-in full-LTO build.** Emits the Hale module as LLVM bitcode
  and compiles the lotus C runtime with `-flto`, so the arena bump-allocator,
  string helpers, and shm-ring fast paths inline across the TU boundary into the
  Hale-generated callers. A few percent on allocation/coordination-heavy code,
  neutral on vectorized loops (host tuning preserved via the function attributes
  above). Off by default — the LTO link is ~3-4× slower and requires `lld`; native
  non-sanitizer builds only.

- **Collection-op inlining, bounds-check elimination, non-allocating-method
  scratch elision.** `@form(vec)` / `@form(hashmap)` `.get` / `.set` / `.pop` /
  `.push` are inlined at codegen (typed GEP + load/store, no `lotus_*` C-call
  boundary); `v.get(i)` indexed by a counted-loop variable (`for i in 0..v.len()`
  with `v` unmutated in the body) drops the per-element bounds check and the read
  vectorizes; and a method proven non-allocating — now including one whose only
  reads are scalar fields of a struct parameter (e.g. a bus handler doing
  `self.sum = self.sum + s.value`) — skips its per-call arena subregion. On the
  grid Hale now leads Go on `form_vec_get` (3.2×), `form_vec_push` (3.8×),
  `vec_amortized` (4.2×), `fn_scratch_work` (8.7×), `json_parse` (2.3×), and ties
  on `form_hashmap_get`.

- **Fixed `String + Int` (and `to_string(Int)` / `to_string(Float)`) emitting
  empty under `--target wasm32`.** The wasm libc shim's `snprintf` was a
  no-op stub (`buf[0] = 0; return 0;`) on the assumption it only built
  diagnostic labels — but `lotus_str_from_int` / `lotus_str_from_float` /
  `lotus_str_from_duration` (the `to_string` / `+`-concat paths) format their
  result through it, so every interpolated Int/Float vanished on wasm while
  native was correct (`"n=" + 5` → `"n="`). Replaced the stub with a real
  minimal `(v)snprintf` (the wasm-only shim — native uses libc, untouched):
  `%d/%i %u %x/%X %c %s %p`, the `l`/`ll`/`z` length modifiers, zero-pad width
  (`%018llu`), and `%g/%f/%e` for doubles matching glibc's default `%g`
  (6 sig digits, `%e`/`%f` selection, trailing zeros stripped) — verified
  byte-identical to native for the decimal magnitudes app/protocol data uses
  (`1e-05`, `1e+06`, `0.0001`, … all match). It also returns the would-be
  length (C semantics), which the Decimal formatter relies on
  (`p += snprintf(...)`). Test:
  `tests/wasm_target.rs::wasm_string_int_concat_formats`.

  (A follow-up — see the next entry — fixed `Decimal` on wasm too, which
  this fix had surfaced as garbage.)

- **Fixed `Decimal` under `--target wasm32` (i128 builtins).** clang lowers
  `__int128` multiply / divide / →double to compiler-rt libcalls
  (`__multi3` / `__udivti3` / `__umodti3` / `__divti3` / `__modti3` /
  `__floatuntidf`), and Ubuntu's clang ships no `libclang_rt.builtins-wasm32.a`,
  so `wasm-ld --allow-undefined` turned them into imports the JS loader stubbed
  to 0 — every `Decimal` (the i128 mantissa at scale 9: arithmetic *and*
  `to_string` *and* `std::decimal::to_float`) came out garbage. The bundled
  wasm libc (`runtime/wasm/lotus_wasm_libc.c`) now **defines** those builtins,
  with bodies that use only 64-bit ops (32-bit partial-product multiply,
  shift-subtract divmod, `f64.convert_i64_u`-based i128→double) so they never
  recurse into the very builtins they provide. Decimal on wasm now matches
  native byte-for-byte (`5.0d`→`5`, `19.99d * 3.0d`→`59.97`, `10.0d / 4.0d`→
  `2.5`, `to_float(19.99d)`→`19.99`). Test:
  `tests/wasm_target.rs::wasm_decimal_i128_builtins`.

- **`@ffi("js")` marshals `Int` / `Duration` as a JS `number` (f64), not a
  `BigInt` (i64).** A Hale `Int` passed to a host import used to arrive in JS
  as a `BigInt`, forcing every handler to `Number(x)` before using it (and a
  host import returning `Int` had to hand back a `BigInt`). Now i64-class
  scalars cross the `@ffi("js")` boundary as f64: the runtime `sitofp`s args
  before the call and `fptosi`s the return, the import's wasm signature uses
  f64, and the JS handler sees a plain `number`. Trade-off: f64's 53-bit
  integer range — an `Int` beyond 2^53 loses precision across the boundary
  (pass it as a `String`/`Bytes` payload instead). Scoped to `@ffi("js")`;
  `@ffi("c")` keeps i64 (those resolve to linked C symbols expecting i64).
  Test: `tests/wasm_target.rs::wasm_ffi_js_int_marshals_as_number`. See
  `spec/ffi.md` § WASM host interface.

- **`std::math::round` / `std::math::trunc` — Float→Int with a chosen
  rounding mode.** Both return an `Int` directly: `round(f)` is round-half-
  away-from-zero (`3.7 → 4`, `2.5 → 3`, `-2.5 → -3`), `trunc(f)` is round-
  toward-zero (an alias of the existing `float_to_int`). `round` is the
  spelling numeric code wants when building an integer field from a Float
  quantity — previously there was a toward-zero conversion (`Int(f)` /
  `std::math::float_to_int`) but no rounding one, forcing the round into the
  caller (e.g. JS, for a wasm client). Both lower to pure LLVM — `fptosi`,
  plus a compare/select half-shift for `round` (no `llvm.round` intrinsic) —
  so they need **no libm symbol and no host import on the `wasm32` target**
  (unlike `floor`/`ceil`, which stay libm and return `Float`). Native +
  wasm32 covered by `tests/ws3_int_float_conversion.rs` and
  `tests/wasm_target.rs::wasm_round_trunc_host_free`. See `spec/types.md`
  § "Explicit numeric conversions" and the `std::math` row in
  `spec/stdlib.md`.

- **Fixed a use-after-free race in the TLS handle table.** `lotus_tls_connect`
  `realloc`s (and thus *moves*) the global handle table when it grows on
  connect, while `recv_into`/`recv_bytes`/`send_bytes` read
  `g_tls_entries[handle]` lock-free. A connect on one connection that crossed
  a growth boundary while a *sibling* connection was mid-recv/send indexed a
  freed base → a wrong/garbage SSL object on the other connection (presents as
  "a busy connection silently kills a quiet sibling after enough
  reconnect churn"). The handle→SSL/fd resolution now happens under the table
  lock — held only for the table read, never across the blocking
  `SSL_read`/`SSL_write`, so concurrent connections still proceed in parallel.
  Same class as the udp remote-table relocation race fixed in #19.

- **TLS recv/send timeouts + a distinguishable recv-timeout sentinel.** Added
  `std::io::tls::set_recv_timeout(handle, d)` / `set_send_timeout` — the
  handle-aware siblings of the `std::io::tcp` timeout setters (TLS connections
  are addressed by handle, not raw fd), wrapping `SO_RCVTIMEO`/`SO_SNDTIMEO`
  on the underlying socket. And `recv_into` (TCP + TLS) now returns `-2`
  ("timed out, retryable") rather than `-1` ("fatal") on a `SO_RCVTIMEO`
  timeout (TCP `EAGAIN`; TLS `SSL_ERROR_WANT_READ`), so a long-lived client
  can bound a blocking read and run connection-liveness work instead of
  hanging forever on a half-open connection. Backward-compatible (`-2` only
  arises once a recv timeout is set). This is the language-side prerequisite
  for the pond `WsClient` liveness fix.

- **Whole-value reassignment of a locus-typed field is now a lifecycle
  transition (post-audit WS1#4 — soundness fix).** `self.conn = WsClient
  { … }` from a member fn previously lowered the RHS locus literal as a
  scope-bound temporary: birth ran, the pointer was stored, then the
  temporary was dissolved at the method's exit — leaving the field pointing
  at a torn-down locus (closed `@ffi` handles / freed arena → use-after-free
  on next use; a downstream app's reconnect crash), while the old value
  leaked. It now reclaims the old instance (its `drain`/`dissolve` run) and
  constructs the new one into the owning locus's arena, owned by the field
  and not scope-dissolved. Clean-compile→segfault closed; regression-gated by
  `ws1_ffi_handle_reassign`. In-place mutation (`self.conn.url = …`) remains
  the cheaper path for "same instance, reconfigure." See `spec/types.md`.

- **Docs-truth pass (post-audit WS5).** New book chapters: *Operations &
  debugging* (the bus-drop / arena-residency / backpressure diagnostics with
  two worked triage walkthroughs) and *Composition patterns* (the three-locus
  gateway, demand-driven discovery, the hot-path-counter/CQRS-rejection
  migration, the publish-policy gate, the view-lifetime rule) — the latter
  also condensed into AGENTS.md. Catalog refresh: `libraries.md` adds
  `http`/`term`/`tui`/`agent`/`ml`/`math` and corrects the stale `subprocess`
  "placeholder" note. Corrected a stale "no-payload-only enums" comment in
  codegen and a "deferred" enum-pattern note in design-rationale — payload-
  bearing enum variants + exhaustiveness have shipped since (verified against
  fixture 45-enum-payloads). (Modes were left un-bannered: the audit's "not
  yet exercised by real workloads" premise is false — a downstream app's orderbook
  declares `mode bulk/harmonic/resolution`.)

- **SQLite stays a library, not a language primitive (post-audit WS4).** The
  audit proposed shipping `std::db::sqlite::*`; on review that's the wrong
  layer — a third-party database belongs in a library, and Hale already has
  the general C-ABI binding surface for it (`@ffi("c")`, "no stdlib expansion
  required to bind a new library"). No `std::db::*` was added. Verified the
  one capability a driver leans on that lacked a test — a `String` *return*
  from `@ffi` (C `const char *` → usable Hale String, for `column_text`) —
  and gated it (`ffi_string_return`). The pond-side `@ffi` recipe to build
  the driver (glue.c + extern decls + `link=["sqlite3"]` + fallible wrapper)
  is in `notes/sqlite-via-ffi-recipe.md`; pond/sqlite is unblocked now, no
  compiler change.

- **Nested-param shm_ring subscribers verified + gated (post-audit WS3.5).**
  An shm_ring subscriber instantiated as a nested locus param
  (`params { sub: Sub = Sub { }; }`) — including as a param of the main
  gateway locus — spawns its reader thread and dispatches correctly; it is
  not the top-level-only silent no-op pond reported. A new regression test
  (`shm_ring_nested_param_subscriber`) covers the gateway and
  intermediate-parent shapes.

- **Two-hop qualified-name literals verified + gated (post-audit WS3.4).**
  A qualified struct/locus *literal* in expression or return position inside
  an intermediate library — `b::Thing { ... }` / `b::SomeLocus { ... }` where
  `app → b → c` and `b` instantiates `c`'s types — resolves correctly at HEAD
  (the "G34" shape pond reported as blocking library composition). The
  existing three-hop test only covered qualified *types* and *fn calls*; a
  new regression test (`two_hop_qualified_literal`) locks in the literal
  position, single- and multi-file intermediate libs, through both
  `hale build` and `hale run`.

- **`hale run <dir>` resolves cross-seed imports (post-audit WS3.3).** A
  directory `hale run` now resolves `import "..." as ...;` directives and
  threads the path-rename table into codegen, exactly as `hale build <dir>`
  already did — previously it bundled the directory's files but silently
  dropped every import, so a directory-seed app importing a vendored library
  failed on `alias::Name` references (and a topic decl appeared to need to
  live in the same file as its publisher). `run` and `build` no longer
  diverge on imports. Cross-file bus topics (`publish T` / `T <- v` resolving
  a `topic T` from a sibling file) work across both. See `spec/projects.md`
  § `hale run` interaction.

- **Nested `if` as a block tail value (post-audit WS3.2).** A
  *value-producing* trailing `if` (every arm ends in a tail expression) is
  now the block's tail expression, so `if` composes as a block value:
  `let x = if a { if b { p } else { q } } else { r };` typechecks instead
  of failing with `then=() else=Float`. A side-effect `if` (no `else`, or an
  arm with no tail) stays a statement — behavior unchanged. Matches
  docs/basics "if is an expression." See `spec/semantics.md` § Expressions —
  `if` and block tails.

- **`std::math::int_to_float` / `float_to_int` (post-audit WS3.1).** The two
  named numeric conversions now lower in any expression position (`sitofp`
  widening / `fptosi` narrowing, round-toward-zero) instead of erroring with
  "unsupported in codegen v0." Previously numeric consumers round-tripped
  through ASCII (`to_string` + `parse_*`) to change a value's type. They're
  the same conversions as the `Int(x)` / `Float(x)` casts, just callable as
  functions. See `spec/types.md` § Explicit numeric conversions.

- **Bounded cooperative bus queue + backpressure (GitHub #125).** The
  cooperative bus dispatch queue no longer grows without bound. It's capped
  at `LOTUS_BUS_QUEUE_CAP` cells (default 8192 ≈ 4.5 MB; env-overridable,
  floor 64); once a single-threaded producer that outruns its consumer hits
  the cap, it **back-pressures** — draining the queue inline (running the
  oldest handlers) to make space — instead of buffering the whole backlog.
  A `birth()` publishing 2M messages went from ~1 GB resident to ~54 MB,
  every message still delivered. Side effect: the `bus_dispatch` microbench
  got *faster* (8.7 → 3.0 ms) — the bounded queue is far more cache-friendly
  than the old unbounded one. **Cross-pool (any → pinned) backpressure** is
  also in: each pinned locus's mailbox is bounded at the same cap, and a
  cross-thread producer that hits it blocks on a condvar until the pinned
  consumer drains (a 2M any → pinned flood: ~1 GB → 54 MB, no deadlock). The
  cross-*cooperative*-pool path (multiple drainers) still grows — a
  follow-on.

- **Memory-bound warnings on by default (GitHub #18 item 1).**
  `hale check` now emits unbounded-allocation warnings without a flag.
  They're **advisory** — they print but don't fail the build (only errors
  do); `--no-warn-unbounded-alloc` opts out. The analysis reached zero
  corpus false positives first: escape-awareness (a non-escaping local in a
  per-message handler is reclaimed at the per-delivery method-scratch
  destroy, so it isn't flagged) and loop-ranking (a `while v < N` const
  counter is proven bounded). The warning flags a value that's allocated in
  a per-message handler / unbounded loop, escapes, and accumulates until
  the locus dissolves — e.g. a whole-value field replace
  `self.f = Struct{…}`, which bump-allocates a fresh value each time. The
  fix it points at is **in-place mutation** (`self.f.x = v` /
  `self.a[i] = v`), a capacity-bounded `@form`, the bus, or a per-iteration
  child locus. The `22-moving-average` and fitter examples were updated to
  mutate in place.

---

## v0.8.3 — verification track, SHM-ring interop, fast JSON

The largest release since v0.8.0 (cumulative since v0.8.2). Four
headline arcs, no source-level breaking changes:

- the compile-time **verification track** (GitHub issue #18) — six
  candidate analyses, four built, one a substrate gate, one parked;
- **binary shared-memory-ring interop** — read/write foreign SHM
  rings by declaring their layout, plus `std::bytes` packing;
- a **JSON parse/emit performance pass** that lands near V8;
- retirement of the tree-walking interpreter and a new `std::term`
  primitive surface.

### Compile-time verification (GitHub issue #18)

The verification roadmap, addressed. The canonical catalog is the
new `spec/verification.md` (#47).

- **Bus-graph property checks (item 4)** — fully landed, runs by
  default. Interprocedural blocking-call detection (warning, #44),
  orphan-topic check (#45), bus-cycle warning + re-entrant
  sync-deadlock error (#46), backpressure check (#48), and bus
  subject type-mismatch (#49).
- **Race-completeness for substrate primitives (item 2)** — a GenMC
  model-checking gate (#50–53) over the lockfree hashmap, the
  pinned-locus mailbox, and the cooperative-pool bus queue under all
  C11 interleavings, wired into CI (#52). A substrate quality bar,
  not a user-facing check.
- **Memory-bound proofs (item 1)** — opt-in
  (`hale check --warn-unbounded-alloc`, `--dump-alloc-summary`). A
  per-method allocation summary + call-graph escape/loop dataflow
  (#100), an empirically-validated reclamation model (#101) that
  **corrected the spec** (#102 — value allocations live until the
  enclosing locus dissolves; free-fn returns do *not* reclaim per
  call), a bound solver with call-graph propagation (#103),
  call-result escape tagging (#112), and **loop-ranking** that proves
  a `while v < N` const counter bounded (#117). Kept off-default
  deliberately (#118) pending an `@unbounded` escape valve, since the
  warnings include legitimately bounded-by-design patterns.
- **Resource-budget tracking (item 5)** — opt-in. Static counts of
  pinned threads / cooperative pools / bus subjects / fd-acquisition
  sites (`--dump-resource-budget`, #111/#115/#116), a CI ceiling gate
  (`--check-resource-budget budget.toml`, #113), and fd-leak
  detection (`--warn-resource-leak`, #112).
- **Closure-assertion lifting (item 3)** — scoped and **deliberately
  parked** (#114): the tractable constant case is already handled by
  typecheck, and the remaining symbolic case is low-leverage for a
  niche feature.

### Binary shared-memory-ring interop

Read and write *externally-defined* binary SHM broadcast rings by
declaring their layout — no hand-written FFI.

- **`std::bytes` binary packing** (#55, #56) — bounds-checked
  little/big-endian readers (`read_u8` … `read_u64_{le,be}`, signed +
  float variants) and `BytesBuilder` writers (`append_u16_le` …
  `append_pad`).
- **`ring_layout` declaration** (#57) — a top-level decl describing a
  foreign ring's magic / version / cursor / framing / overflow; a
  `shm_ring(..., layout: N)` binding kwarg (#58) binds a topic to it.
  Read-only consumer (#59), producer (#61), and `ring_layout` ↔
  payload conformance checks (#60), cataloged in `spec/verification.md`
  (#66).
- **Raw `BytesView` payload mode** (#72, #77) — a bounded view per
  record for heterogeneous rings, with a symmetric producer path;
  native-ring `slots` framing reachable through the same abstraction
  (#75).
- **Go-style struct field tags** (#80) + **repr-tagged field
  accessors** (#81, #82) — direct typed field access over a raw frame
  at compile-computed offsets.
- **Zero-copy ring write surface** (#78, #79) — a reserve/commit split
  for writing records in place. OOB-hole fixes at the foreign-producer
  boundary (#67), under UBSan in CI (#68).

### JSON performance

A parse + emit pass bringing generated JSON codecs near V8.

- **Tier 2 — generated codecs from `json:` tags** (#84–88): a
  single-pass object-member cursor, `Type::from_json` (including
  nested structs), and a symmetric `Type::to_json`.
- **Tier 3 — SIMD** (#90–92): SIMD-accelerated object/array cursors
  with an AVX2 path for the scan primitives.
- **Inline leaf primitives** (#93–97): the generated parser inlined
  (no per-field cursor structs), the unescape copy skipped for
  escape-free strings, and `byte_at` / `range_eq` inlined to
  gep+load / direct compares. A representative parse went ~291 ms →
  ~58 ms — within range of V8.

### Standard library & runtime

- **`std::term` + raw byte I/O** (#108–110): `is_tty(fd) -> Bool`,
  `size() -> TermSize`, the `RawMode` guard locus (atexit-backed
  termios restore), `std::io::stdout::write_bytes`, and
  `std::io::stdin::read_byte` — terminal hygiene with no vendored FFI
  glue.
- **Interpreter retired** (#41, #42): `hale run` now compiles + execs
  via codegen; the tree-walking `hale-runtime` crate is deleted, so
  there is no interpreter/codegen parity to maintain.
- **Stale-view panic via `exit()`** (#106) so `atexit` cleanup (e.g.
  the `RawMode` restore) runs on a panic path.
- **`BytesBuilder.append_str`** (#105) + a clarified StringView
  non-coercion rule at `@ffi` params.
- **ECDSA P-256** gains a `fallible(CryptoError)` form (#43).
- **Locus method names no longer mangled** (#104) — fixes inline /
  `accept`'d loci referenced in method bodies.

### Language surface

- **CQRS at the locus boundary (#18.6 / #81).** Methods on loci
  may not return locus values. The compiler rejects
  `fn lookup(id: String) -> Counter` on a registry locus at
  typecheck. The rule keeps the substrate model honest — a
  returned locus would be a stranger in the caller's scope, with
  no lifecycle tower above it. Three canonical alternatives:
  parent-child + contract (`accept`'d children, pair with an
  index slot for name-based lookup), bus topic (publish typed
  commands keyed by name), or delegation (collapse the per-child
  operation onto the parent). See `spec/semantics.md § Locus
  method dispatch`.

- **`resets_per_epoch(...)` closure clause (F.34, #75).**
  Closes the `low_corrupt_rate`-shaped friction (per-window rate
  budgets). A closure paired with `epoch duration(N)` may now
  declare `resets_per_epoch(field1, field2, ...);` — the
  runtime zeros the named fields AFTER the assertion fires at
  each duration boundary. Ordering matters: the assertion sees
  the window's accumulated value, the reset prepares the next
  window. Typecheck rejects pairing with non-duration epochs and
  non-numeric fields. See `spec/semantics.md § Per-epoch field
  reset` + `spec/design-rationale.md § F.34`.

  ```hale
  closure low_corrupt_rate {
      self.corrupt_per_min ~~ 0 within 10;
      epoch duration(1m);
      resets_per_epoch(corrupt_per_min);
  }
  ```

- **Nested long-running cooperative children rejected at typecheck
  (#76 / F.31-followup).** A non-main locus with a non-trivial
  `run()` body holding a `params` field of a locus type whose own
  `run()` is also non-trivial — including `std::http::Server` and
  the other entries on the known-long-running stdlib allowlist —
  is now a compile error pointing at the sibling-in-main +
  placement fix. The runtime starvation that motivated this rule
  was silent (parent's `run()` simply never executed), so the
  type-side rejection converts a class of hard-to-diagnose
  runtime bugs into a clear compile-time signal. See
  `spec/runtime.md § Long-running cooperative children`.

### Diagnostics

- **`@form(hashmap)` cell-locus rejection improved (#77).** The
  pre-existing rule (cells may not be locus references) now
  produces a diagnostic that names the three canonical
  alternatives (parent-child + index, bus topic, delegation) and
  cross-references `spec/semantics.md § Locus method dispatch`.
  Same framing as #18.6 at the form-synthesis layer.

- **`LOTUS_BUS_LOG_DESERIALIZE_DROP=1`.** Surfaces silent drops
  in the udp:// reader thread when no deserializer is registered
  for the inbound subject, or the deserializer returns ≤ 0 (size
  mismatch, bounded-read failure). Off by default; the silent-skip
  on cross-routed multicast noise stays correct in steady state.
  Three udp:// bring-up handoffs this week traced back to silent
  drops on `deserialize → local-dispatch`; the lack of any signal
  was load-bearing on debug cycles. Same env-gated pattern as the
  existing `LOTUS_BUS_LOG_UNMATCHED`.

### Internals

- **Codegen refactor (#22).** `crates/hale-codegen` reorganized:
  per-domain submodules (`locus/`, `bus/`, `shared/`, `stdlib/`),
  `codegen.rs` reduced by 56.2%. No surface-level changes.

### Documentation

- **`docs/src/concepts/the-locus.md`** — CQRS rule paragraph.
- **`docs/src/concepts/the-bus.md`** — routing keys +
  `on_unmatched` policies (covering machinery shipped in v0.8.2).
- **`docs/src/concepts/capacity-storage.md`** — hashmap cell-
  locus rule with alternatives.
- **`docs/src/concepts/error-handling.md`** — `resets_per_epoch`
  coverage in the closures intro.
- **`docs/src/how-tos/threading.md`** — nested-long-running
  rejection in "What you can't do".
- **`docs/src/how-tos/keeping-memory-bounded.md`** — factory /
  cached-handle sections rewritten around the boot-time Int-
  index resolution pattern (the previous example used the
  now-rejected `reg.counter().inc()` shape).
- **`spec/design-rationale.md`** — new F.34 entry.
- **`spec/verification.md`** (new) — the canonical catalog of all
  static checks: the default bus-graph rules, the `ring_layout`
  conformance + geometry checks, and the opt-in memory/resource
  analyses (with the `--check-resource-budget` TOML schema).
- **`spec/memory.md`** — corrected to the shipped reclamation model
  (value allocations live until the enclosing locus dissolves;
  free-fn returns don't reclaim per call).
- **`spec/stdlib.md`** — `std::term` + `std::io::{stdin,stdout}` raw
  I/O rows; the `std::bytes` binary-pack reader/writer family;
  `BytesBuilder.append_str`.
- **`spec/ffi.md`** / **`spec/semantics.md`** / **`spec/grammar.ebnf`**
  — StringView non-coercion at `@ffi` params; the `ring_layout`
  declaration grammar + foreign-ring payload modes.
- **mdBook** — `systems/performance.md` gains a "Catching it at
  compile time" section (the analysis flags); `everyday/cli-config.md`
  gains "Interactive terminal I/O" (`std::term` / raw byte I/O).

---

## v0.8.1 — F.32 cache-aware substrate + #24 narrowing

Cumulative changes since v0.8.0. No source-level breaking
changes; one rule narrowing (open-question #24) lifts a
previous restriction.

### Language surface

- **`fallible(E)` on user-declared locus member fns**
  (open-question #24). The blanket "locus methods cannot
  declare `fallible(E)`" rule narrowed to "substrate-facing
  surfaces cannot." User-declared `fn` member fns now
  carry `fallible(E)` like free fns do, with the full `or
  raise` / `or <substitute>` / `or <handler(err)>` /
  `or discard` disposition surface. Heap-bearing success
  and err payloads (`String`, `Bytes`, nested-struct-with-
  heap-fields) are supported via the same TLS caller-arena
  snapshot non-fallible heap-returning locus methods use.

  Still rejected (substrate-facing surfaces, no caller
  frame to address the value channel): lifecycle methods
  (`birth` / `run` / `accept` / `drain` / `dissolve` /
  `on_failure`), mode methods (`bulk` / `harmonic` /
  `resolution`), closure assertions, and bus-subscribed
  handlers. Bus-handler rejection fires at the subscribe
  site, not the fn decl. See `spec/semantics.md`
  § "Where each channel lives".

- **`@locality(L1|L2|L3|any)` annotation on a locus**
  (F.32-2 v0.2). Pins a per-locus cache-tier budget the
  working-set estimator evaluates against. `any`
  explicitly opts out of any global gate. Stacks with
  `@form(...)` in either order; max one of each. See
  `spec/grammar.ebnf` § `locality_annotation` +
  `spec/types.md` § "Working-set estimator (F.32-2)".

### Cross-pool `@form(hashmap)` sync disciplines

The cross-pool exemption that admitted plain `@form(hashmap)`
loci into concurrent-write paths was found to corrupt the
runtime's hashmap on concurrent grow (`lotus_hashmap_set` /
`_grow` are non-atomic single-threaded code).

- **F.32-0**: cross-pool exemption reverted; plain
  `@form(hashmap)` is single-pool by default. Cross-pool
  use requires an explicit `sync = X` opt-in.
- **`sync = serialized`** (α): per-map mutex. Simplest
  correct cross-pool path.
- **`sync = striped`** (β2-v2): cell-level CAS + per-map
  rwlock for grow + cache-padded cells. Parallel writers;
  grow path serializes.
- **`sync = lockfree, cap = N`** (γ-v1): fixed-cap,
  cell-level CAS, no rwlock or mutex. Highest measured
  throughput on the false-sharing bench (1.30× over α at
  2 cores, AMD Ryzen 9800X3D); no grow, no remove.

Discipline-picker table in `spec/forms.md` § "Cross-pool
sync disciplines". Inference (closed-world picks one of
α/β/γ from the pool-propagation graph) lands as a
typecheck-diagnostic enhancement; explicit pasting still
required to apply (auto-apply deferred).

### Working-set estimator (F.32-2)

Compile-time analysis projecting each locus's bytes
against a cache-tier budget. Opt-in via:

- **`hale build --locality-report`** — informational
  per-locus table on stderr; build proceeds.
- **`hale build --target-cache l1|l2|l3`** — over-budget
  loci warn on stderr; build proceeds.
- **`hale build --target-cache lN --strict`** — over-budget
  loci fail the build before codegen (exit 1).
- **Per-locus `@locality(...)`** — annotation wins over
  global `--target-cache`; `@locality(any)` opts out.

Tier sizes auto-detect from
`/sys/devices/system/cpu/cpu0/cache/index{0,2,3}/size` on
Linux (cached for the build's lifetime); static fallbacks
32 KB / 512 KB / 8 MB apply elsewhere.

Estimator accounts for alignment padding (struct interior
padding + final padding to struct alignment); previous
packed-layout assumption under-estimated by ~10-20% on
mixed-alignment shapes.

### Codegen substrate work

- **Codegen-aware per-pool chunk-size hint** (F.32-3).
  Loci instantiated on a non-`main` cooperative pool get
  a chunk-size hint sized to `target_L2_per_core /
  loci_on(pool) / typical_chunks_per_locus`, clamped to
  `[4K, 64K]`. The runtime's `lotus_arena_create_labeled_sized`
  honors the hint; env override
  (`LOTUS_ARENA_CHUNK_BYTES_OVERRIDE`) still wins via the
  upper bound.
- **Locus struct field reorder by access frequency**
  (F.32-1b). User-declared `params { }` fields are sorted
  by `self.<field>` access count, with a 10^depth
  multiplier per loop nesting level. Hot fields land on
  the first cache line of `self`.
- **Bus-dispatch prefetch hint** (F.32-4-prefetch). Producer
  emits `__builtin_prefetch(slot, 1, 3)` after the memcpy
  in `lotus_coop_pool_post` and friends. A/B toggle via
  `LOTUS_DISABLE_PREFETCH=1` at build time.
- **Huge-page-backed arenas + `mlockall`** (F.32-4a / 4c).
  Operator-tunable via `LOTUS_HUGE_PAGES=1` /
  `LOTUS_LOCK_MEMORY=1` env vars; documented in
  `docs/src/how-tos/keeping-memory-bounded.md`.

### Tooling

- **highlight.js mode** (mdbook docs site): `placement`
  and `discard` now style as keywords. `@locality(...)`
  picks up the generic `@<ident>` annotation rule.
- **heron tree-sitter grammar** (sibling repo): adds
  `placement_block` + `placement_spec` + `locality_annotation`
  + `locality_tier`. Editor highlighting + the future LSP
  parse both new constructs. Released as
  `hale-lang/pond@5d8202d`.

### Documentation

- **README** rewritten with substrate-pluralization framing:
  matchmaker example walkthrough (every phrase maps to a
  syntactic slot), "One language. Every substrate." section
  (native + browser shipped via hale-js; mobile / embedded /
  GPU / robotics / edge characterized as workload-pull, not
  roadmap), "Try it on code you already have" zero-install
  demo via AGENTS.md drop-in, "what the compiler is doing
  for you" enumeration with F.32 as receipt.
- **Spec sweep** for #24 + F.32-2: `spec/types.md`
  declaration restrictions narrowed and new "Working-set
  estimator" section; `spec/semantics.md` "Where each
  channel lives" rewritten; `spec/styleguide.md` two-channel
  rule references narrowed; `spec/stdlib.md` TCP Stream
  sentinel-shape framing updated; `spec/grammar.ebnf`
  picks up `locality_annotation`.

### Internals

- Sync inference walker covers all `Expr` arms
  (`Sum` / `Prod` / `Approx` / `Range` / `ArrayRepeat` / `Or`);
  previously the catch-all `_ => {}` arm under-counted
  `self.<field>` references inside closure assertions,
  range expressions, and `or`-substitute RHS.
- Working-set estimator's `BudgetBreach` records carry
  `tier: CacheTier` + `source: BudgetSource` so per-breach
  diagnostics name whether the contract came from
  `@locality` or `--target-cache`.

### Not in this release

The deliberately-deferred items per `notes/f32-cache-aware-delivery-plan.md`:

- **F.32-1γ-v2** (lockfree grow + tombstones). Needs tsan /
  relacy concurrency validation and a downstream workload
  that hits γ-v1's fixed-cap ceiling. Default: do not
  pursue until both gates clear.
- **Auto-applied sync inference**. The inference engine
  picks `sync = X` from the pool-propagation graph; v0.2
  will inject the kwarg into the AST so codegen honors
  it without the user pasting. v0.8.1 ships diagnostic
  enhancement only.
- **NUMA-aware placement** (`pinned(numa = N)`). No
  workload pulling yet.

---

## v0.8.0 — initial release

The language surface is stable. A few small additions are
planned, but most work from here to v1 is bugs, stability, and
performance — not new syntax or new semantics. Pin to a commit
if you build on it; small additions still land. The reference
contract is the spec under `spec/` plus the in-tree fixture
programs under `crates/hale-codegen/tests/fixtures/examples/`.
