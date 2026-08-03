# Changelog

Behavior changes by release. The canonical spec lives in
[`spec/`](./spec/) — each file there represents *current*
behavior.

---

## Unreleased

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

---

## Unreleased

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

---

## Unreleased

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

---

## Unreleased

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

---

## Unreleased

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
