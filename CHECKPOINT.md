# Lotus — session checkpoint

**Read this first** if you're picking up the lotus language work
in a new session. State as of **m60: per-payload serializer
shape (wire format deferred)** — fourth substrate piece of the
cross-process bus arc. Installs the serializer/deserializer
hooks that codegen routes every bus payload through, but
deliberately leaves the actual wire format as a future-
milestone choice. Codegen now synthesizes
`__serialize_<TypeName>(src, dst, cap) -> i64` and
`__deserialize_<TypeName>(src, n, dst, cap) -> i64` per bus
payload type during a new pass A3 (between locus declarations
and body lowering); bodies are identity at v0.1 (memcpy
sizeof(T) bytes), so a publisher and subscriber on the same
arch + same compiler version stay byte-compatible — observable
behavior is unchanged from m59. The shape is what installs the
substrate: `<-` send sites alloca a scratch buffer, call
`__serialize_T(payload, scratch, sizeof(T))` and pass the
returned size + scratch ptr to `lotus_bus_dispatch` (replacing
the previous "raw struct ptr + sizeof(T)" pattern); bus
subscribe registration passes `__deserialize_T` as a 5th
argument to the now-extended `lotus_bus_register(subject,
self, handler, mailbox, deserialize_fn)`; the m59 reader
thread looks up the deserialize fn from the first matching
local entry on each recv, decodes wire bytes into a struct
buffer, then calls `lotus_bus_local_dispatch` with struct-
layout bytes. A future wire-format milestone replaces the
function bodies — field-by-field little-endian, length-
prefixed Strings, schema versioning — without touching call
sites. Per notes/open-questions #10 the contract stays "the
receiver's arena gets a fresh copy of the payload struct"; the
serializer is just the encoder/decoder for the bytes between
publisher and that arena copy. Verified via new
`crates/lotus-codegen/tests/serializer_shape.rs` (builds a
program with two payload types, asserts the IR contains
`define i64 @__serialize_Ping`, `@__deserialize_Ping`,
`@__serialize_Pong`, `@__deserialize_Pong` and that the send
site emits `call i64 @__serialize_Ping`); m59's two-binary
publisher/subscriber test (`bus_subscriber.rs`) continues to
pass unchanged, which is the operational proof that identity
bodies preserve byte semantics through the new pipeline. 102
tests pass (was 101; +1 from serializer_shape.rs); 54 example
builds unaffected; bus examples (05-bus, 19-pinned-bus,
30-stats) rebuild + run with byte-identical output. State
before that was **m59: subscriber-side reader
thread** — third substrate piece of the cross-process bus
arc. Closes the receive half: when a deployment-config entry
declares `role=listen`, the runtime spawns a per-subject
pthread that opens the LISTEN-role transport ON THAT THREAD
(so accept() blocks the reader, not main's boot path), loops
on `lotus_transport_recv`, and dispatches the recv'd bytes
into local subscribers via a new `lotus_bus_local_dispatch`
helper extracted from `lotus_bus_dispatch` (so the call
doesn't recursively trigger remote fanout, which would loop
forever). Reader threads are joined at
`lotus_bus_remote_destroy_all` after a best-effort
`shutdown(conn_fd, SHUT_RDWR)` — natural EOF (peer-close)
drives thread exit in well-formed teardown. The codegen
prelude gains one call: `lotus_bus_set_queue(queue_ptr)` after
`bus.queue.init`, before `bus.load_config` — publishes the
cooperative queue pointer to the C runtime so reader threads
can dispatch into the same handler set in-process publishers
reach. Wire format stays **raw struct bytes** at v0.1: same
arch + same compiler version means identical layout on both
sides, which is correct at v0.1 — the wire-format
choice (field-by-field little-endian for layout robustness
across binary versions, length-prefixed Strings, schema
versioning) is deferred to a workload-driven future
milestone. m60 installed the serializer shape so that swap
is body-only when it lands. The m59 integration test
in `crates/lotus-codegen/tests/bus_subscriber.rs` builds two
**lotus** binaries from inline sources (one subscriber, one
publisher), wires them via opposite-role configs to the same
unix socket, and asserts the subscriber's stdout contains the
publisher's printed Ping value end-to-end (`subscriber got
n=4702394921427289928`). Stable across 5 sequential runs at
~730ms each. The cross-process bus loop is now closed: a
publish on a wired subject reaches in-process subscribers AND
all CONNECT-role transports (m58); recv'd bytes from any
LISTEN-role transport flow into in-process handlers (m59).
101 tests pass (was 100; +1 from bus_subscriber.rs); 54
example builds unaffected; existing bus examples (05-bus,
19-pinned-bus, trellis-demo) rebuild + run with byte-identical
output. State before that was **m58: deployment-config subject
binding** — second substrate piece of the cross-process bus
arc. Wires m57's `lotus_transport_*` surface to the source
language without coupling source to deployment topology
(notes/open-questions #8). Three additions to
`crates/lotus-codegen/runtime/lotus_arena.c`:
(a) `lotus_bus_load_config(path)` parses a tiny line-format
config file (`subject = url : role` per line, `#` comments,
whitespace-tolerant) and dispatches to `lotus_bus_register_remote`
per entry; (b) `lotus_bus_register_remote(subject, url, role)`
opens a transport via the m57 surface (currently `unix://`
scheme only — TCP/SHM/etc. grow into the same dispatch later)
and stashes (subject, transport, role) in a heap-grown
`g_bus_remote_entries` table; (c) `lotus_bus_remote_fanout` is
called from `lotus_bus_dispatch` after the existing local
loop, so a publish on a wired subject reaches BOTH local
subscribers AND any CONNECT-role transports — local + remote
share the same subject namespace per
notes/open-questions #9. `lotus_bus_router_destroy` extends to
close all remote transports + free the subject-string copies.
Codegen change is one call in main's prelude:
`lotus_bus_load_config(getenv("LOTUS_BUS_CONFIG"))` — emitted
unconditionally (the C-runtime fn no-ops on NULL path), so
binaries run without the env var set behave byte-identically
to pre-m58. v0.1 scope is **publisher-side only**: CONNECT-role
transports fan out at publish time; LISTEN-role
accept-and-spawn-reader-thread is m59+. The integration test
in `crates/lotus-codegen/tests/bus_config.rs` proves this
end-to-end in one process pair: a lotus-built publisher with
a `Ping{n: 0x4142434445464748}` payload runs under
`LOTUS_BUS_CONFIG`, its m58 fanout sends the struct bytes
through a unix socket, and the m57 transport_driver in listen
role recv's exactly those bytes (asserted i64-LE round-trip +
local-subscriber-also-fired). A second test asserts the no-config
path still routes locally (codegen prelude regression check).
100 tests pass (was 98; +2 from bus_config.rs); 54 example
builds unaffected. State before that was **m57: AF_UNIX
transport in the C runtime** — first substrate piece of the
cross-process bus arc. Adds `lotus_transport_create / send /
recv / destroy` to
`crates/lotus-codegen/runtime/lotus_arena.c`: SOCK_SEQPACKET so
each send shows up as exactly one recv (message boundaries
preserved, no framing layer needed at this milestone), bind +
listen + accept on the LISTEN role, connect-with-retry
(ENOENT/ECONNREFUSED, ~1s ceiling at 5ms backoff) on the
CONNECT role so the connector can race ahead of the listener
without an external sync. No codegen wire-up at m57: the
surface is exposed as stable C-ABI fns that m58's deployment-
config subject→transport routing calls into. New
`crates/lotus-codegen/tests/transport_driver.c` is a tiny
harness binary that the new
`crates/lotus-codegen/tests/transport.rs` integration test
compiles (clang links driver + lotus_arena.c) and exec's twice
— once as listener, once as connector — to verify a byte-for-
byte round-trip across two processes. Two assertions: (a) short
message round-trip, (b) message-boundary preservation
(payload with embedded whitespace + trailing newline). State
before that was **m55 + m56 (design-decision
substrate cleanup)** — applied The Design's calls on the
deferred recovery vocabulary, generics direction, and several
spec-vs-impl drifts. m55: removes `drain` / `dissolve` from
the `RecoveryOp` enum (vocabulary becomes restart /
restart_in_place / quarantine / bubble + reorganize — five
primitives, no overlap with lifecycle methods); they remain
lifecycle methods only, callable via lifecycle declarations
in the locus body. m56: docs-only — resolves notes/
open-questions #8 (bus subject → transport via deployment
config), #9 (same-subject = runtime fan-out), #10 (bus
messages cross arena boundary as copies), #16 (`reorganize`
= restart-in-place lifted to substructure level, preserving
children), #17 (drain/dissolve as recovery ops removed), #2
(extends ProjectionClass with `Numeric` bound for v1; defines
generic + bus payload + closure interactions); spec/memory
"Region size tuning" rewrites the panic-on-exceed language
to reflect that arenas grow linked-list chunks indefinitely
(declared params are sizing hints, not ceilings); + a new
"Bus handler shape" doc-section locking m54's rejection of
defaults on bus handler payload params as the spec position.
State before that was **m54: default params on mode
methods** — closes the m34 deferral that left modes unable to
declare defaulted params. The codegen-declare rejection is
replaced with the same suffix-only ordering check the locus
`fn` path uses; `lower_self_method_call` already routes Fn and
Mode through the same call-site default-fill-in flow, so no
call-site changes were needed. Bus handlers stay correctly
rejected (their single payload param is always provided by
dispatch — defaults on it would never fire). New
`examples/50-mode-defaults` exercises a `mode bulk(scale: Int =
10) -> Int` called both with and without the override; both
backends produce identical output. State before that was
**m53: free-fn implicit-locus handle-rooting** — closes the second half of spec/memory.md
"Free `fn` functions": "the function returns when (a) body's
last statement completes, AND (b) all children of the implicit
locus have dissolved." m49 shipped (a) for the arena boundary;
m53 ships (b). `flush_dissolve_frame` is now hoisted into
`emit_fn_exit_epilogue` (called once per fn at the unified
exit block), so every return path — open-end, `return;`,
`return v;` — uniformly drains the bus queue and dissolves any
long-lived loci bound in the fn body before the fn returns.
Pre-m53 the typed-return path silently popped the frame
without flushing, leaking handles past return; m51's
non-leaf-return regression test would have masked this since
the leaks weren't visible at stdout. New
`examples/49-fn-handle-rooting` exercises the fix: a
`make_pair(n) -> Int` binds Watcher (subscribes "evt") and
Pulse (publishes "evt" in birth), the publish enqueues a cell
for Watcher, the fn return triggers flush which drains (firing
Watcher.on_evt) and dissolves both. Both backends produce
identical output. State before that was **m52: in-loop drain
after each dissolve** — closes the m26 v0 limitation that cells enqueued
during a dissolve method's execution were leaked at main-exit
queue teardown. `flush_dissolve_frame_kind` now drains the
cooperative bus queue AFTER each iteration of the dissolve
loop (in addition to the existing initial drain), so cells
published by locus N's dissolve method get dispatched to
still-alive later-iter loci before those loci themselves
dissolve. The deregister-on-dissolve invariant
(m45-followup-2) means the drain never targets the
just-dissolved locus, so the additional drain is safe. New
`examples/48-publish-during-dissolve` exercises the path: a
Sender locus publishes "farewell" in its dissolve method, a
Receiver locus (instantiated first → dissolves last) receives
it before its own dissolve runs. Both backends produce
byte-identical output. State before that was **m51: free-fn
return-copy completion** — extends m49's deep-copy to the three branches
left as `Unsupported`: Array (recursive walk over elements),
TypeRef-struct (walk over declared fields by struct slot, with
recursion through heap-typed fields like String), and
has-payload-Enum (switch-on-tag with per-variant alloc + payload
field deep-copy + PHI join). Free-fn return-copy is now
type-complete: every LotusType except `LocusRef` (which by design
shouldn't cross arena boundaries — pass via bus instead) is
supported. New `examples/47-fn-arenas-extras` exercises all three
paths: a `[Int; 4]` returner, a `Point { x, y, label: String }`
returner (recursion through the String field), and a
`Result::Ok(Int) / Result::Err(String)` returner. State before
that was **m50: immutable-binding enforcement**: `let x = 0; x = 1;` is now the compile-time
error spec/types.md "Mutability" + design-rationale §E always
said it should be. The typechecker tracks `is_mut` on every
local symbol — `let mut` propagates true, plain `let`
propagates false, and fn params / loop vars / pattern bindings
default to false. `Stmt::Assign` to a bare-head non-mut local
(no `.field` / `[i]` segments, head ≠ `self`) raises a clear
diagnostic; field / index reassignment THROUGH an immutable
head stays allowed (mutates state, doesn't rebind). All 50
examples continued to build clean — the corpus already wrote
`let mut` discipline correctly; m50 closes the spec/impl drift
without breaking anyone. notes/open-questions #23 marked
resolved. State before that was **m49: free-fn implicit-locus
arenas**: every non-main free fn now opens a per-call subregion
of its caller's arena at body entry; body allocations route
through it; heap-typed return values are deep-copied into the
caller's arena (`lotus_str_clone` for String, recursive walk
for Tuple, identity for value types) before the subregion is
wholesale-freed; `main` keeps `arena.global` as the single
caller-less fn. Closes the m20 deferral that kept all free-fn
allocations leaking into a program-wide arena until process
exit; spec/memory.md "Free `fn` functions" boundary is now
enforced at the codegen substrate. Interpreter unchanged
(Rust ownership already handles lifetimes correctly). Heap
returns of Array / TypeRef-struct / has-payload-Enum reject
at v0.1 — none currently appear as free-fn returns; ship when
a workload demands. New `examples/46-fn-arenas` exercises
String + Tuple deep-copy in a loop.

State before that was **enums-complete**: a session arc
that started with no-payload tagged unions (m47) and finished
with full-fidelity payload-bearing variants. Surface coverage:
`type X = enum { A, B(Int), C(Decimal, String) };` declares any
mix of no-payload and payload variants; `X::A` (no-args) and
`X::B(7)` (with-args) construct; `match` accepts both bare and
arg-binding constructor patterns plus guards on payload bindings
(`X::B(n) if n > 10 -> ...`); `==` does deep equality across tag
+ payload fields; `println` and `to_string` render
`"X::B(7)"` / `"X::C(3.14, hello)"`; bus dispatch carries
has-payload enum values cleanly; enum values work as struct
fields, array elements, tuple components, locus params, fn
args/returns, and `self.field = ...` mutations. Codegen
representation is per-enum: pure no-payload enums stay as i32
tags (value semantics), once any variant has a payload the whole
enum becomes a pointer to `{ i32 tag, [N x i8] body }` (heap
storage in the current arena). Interpreter mirrors via
`Value::EnumVariant { enum_name, variant_name, payload: Vec<Value> }`.
The typechecker resolves `EnumName::Variant` paths and
`EnumName::Variant(...)` calls to the named enum type; match
exhaustiveness counts constructor arms (with or without
sub-patterns) as covering their variant.

Before the enums arc this session shipped m48 (**Decimal
fixed-point** — exact i128-backed arithmetic in both backends;
pre-m48 Decimal was f64-backed and round-tripped through
`parse::<f64>()` / `%g`. Interpreter uses `DecimalVal { mantissa:
i128, scale: u32 }` with per-value scale that round-trips source
spelling; codegen uses i128 with fixed scale 9 and a C-runtime
helper splits the i128 into hi:lo halves for printf, calling
`lotus_decimal_to_string` — output trims trailing zeros so the
two paths print identically).
State before that was m46-vocab (`count()` no-arg + `mean(x)`
accumulator builtins built on m46's sum accumulator) and m46
(**closure accumulators** — the streaming-fold half of F.9:
`sum(self.X)` calls inside a closure assertion accumulate
`self.X` across every epoch fire and the assertion reads the
running total; recovery events zero the accumulators by default,
opt-out via `persists_through(...)`).
This session also shipped (a) **bus.entries proper fix**
(m45-followup-2) — bus storage migrated out of fixed-cap LLVM
globals into a heap-grown C-runtime dynamic vec, removing the
`× 32` ceiling entirely; LLVM-side linear-scan dispatch fn is
gone, replaced by `lotus_bus_register` / `lotus_bus_dispatch`
/ `lotus_bus_quarantine_self` / `lotus_bus_router_destroy`
C-runtime fns, and (b) **pinned-duration fix** (m43-followup) —
synthesized `__duration_closures_wrapper` paired with the
existing `__tick_closures_wrapper`, called from
`__pinned_main_<Locus>` after `run()` so duration-epoch closures
fire post-run on the pinned thread (closes the m43 v0 limit).
The F.9 invariant-and-repair substrate is now both
closure-epoch-complete (5/5: Birth + Dissolve + Tick + Duration +
Explicit) AND closure-accumulator-complete (the streaming-fold
half — `sum(...)` only at v0.1; count/mean/window deferred).
Recovery primitive set covers restart / restart_in_place /
quarantine / bubble. Surface-completeness arc through m38, then
the substrate-foundation arc m39 (birth-epoch trigger), m40
(restart response, cap-2), m41 (quarantine — stop-trying flag),
m41b (quarantine extends to bus dispatch), m42 (tick — steady-state
pulse), m43 (duration — every-N-of-monotonic-time gate), m44
(explicit — user-triggered via `check_closures();`), m45
(`restart_in_place` recovery primitive), m46 (accumulators).
Substrate arc: m19→m23 (region allocator with
rich/chunked/recognition + per-locus arenas + bus copy), m24
(`match`), m25 (bimodal schedule-class annotation), m26
(cooperative scheduler — deferred bus + drain loop), m26b
(explicit `yield`), m27 (pinned threads, run-only), m28a (full
pinned lifecycle), m28b (cross-thread bus mailboxes), m28c
(`pinned(core = N)` core pinning). Surface-completeness arc:
m29 (match arm guards), m30 (fixed-size arrays + indexing +
for-over-array), m30b (indexed local-array assignment), m31
(integer ranges as for-iterators), m32 (default param values
on free fns), m33 (multi-file `import` resolution; std/* skipped
as built-ins), m34 (default param values on locus `fn`
methods; bus-handlers + modes still reject), m35 (tuples
— anonymous heterogeneous records of fixed arity ≥ 2, with
`(a, b)` literals, `t.0` / `t.1` numeric field access, `let
(a, b) = pair;` destructure, and tuple patterns in match
arms), m36 (string ops — `+` concat, `==`/`!=` equality,
`len`, exclusive + inclusive range slicing `s[lo..hi]` /
`s[lo..=hi]` with bounds clamping), m37 (`to_string(x)`
primitive→String conversion for dynamic composition; output
matches println formatting), m38 (stdlib helpers — `min` /
`max` / `abs` across numeric types, plus `starts_with` /
`contains` for String predicates), m39 (birth-epoch
closures — F.9 invariants checked right after `birth()`
returns, before `run()` runs), m40 (restart recovery
primitive — `restart(child);` from inside `on_failure`
bumps a per-locus counter; within cap the runtime re-runs
birth + birth-epoch closures on the same memory), m41
(`quarantine(child);` — sticky flag that gates `run()`
without affecting drain/dissolve cleanup) + m41b
(quarantine extends to bus dispatch — quarantined
subscribers stop receiving messages), m42 (tick-epoch
closures — fire after each bus handler + after run()
returns; substrate-coupling between F.9 invariants and the
cooperative scheduler's substrate-cell boundary), m43
(duration-epoch closures — every-N-of-monotonic-time gate
on top of the tick cadence), m44 (explicit-epoch closures —
user-triggered checkpoint via `check_closures();`), m45
(`restart_in_place` factory-reset recovery), m45-followup-2
(bus.entries proper-fix — C-runtime dynamic vec replaces the
fixed-cap LLVM-side table + linear-scan dispatch fn),
m43-followup (pinned post-run duration via the new
`__duration_closures_wrapper`), m46 (closure accumulators
— `sum(self.X)` running totals across epoch fires +
`persists_through(...)` recovery gating), and a small
post-m46 hardening — `emit_locus_arena_destroy` now calls
`lotus_bus_quarantine_self(self_ptr)` before destroying the
arena, mirroring the m41b deregistration path so a stale
entries-vec entry can never direct dispatch into freed
memory after dissolution. **54 of 55 examples build to
native ELF — every single-binary example.** Only
`trellis-pair` (multi-binary, cross-process bus) remains.

**The bimodal scheduler is fully complete.** Cooperative loci
yield between substrate cells via the inline-payload deferred
queue; pinned loci own their thread, run their full lifecycle
(including subscribed bus handlers via per-locus mailboxes),
and can pin to a CPU core. Both layers stay arena-lock-free —
the substrate cost lives at the boundary (the queue/mailbox
mutex + the cell's two memcpy's).

**The Design / lotus is now visible at the codegen substrate.**
Same source, two execution shapes (cooperative / pinned) and
three memory shapes (rich / chunked / recognition), all
expressed as locus annotations. Substrate-invariance applied
to time was kept honestly **bimodal** — no third "greedy"
class, since cooperative already guarantees handler-atomicity
and anything beyond that means leaving the shared scheduler =
own thread = pinned. (Memory has more genuine intermediate
ground than time does, so projection class stays three-way.)

Two prior-session design decisions still drive the bus arc:
runtime owns kernel-level transports (shared memory / AF_UNIX
/ TCP / UDP), stdlib owns protocols on top (NATS / MQTT /
gRPC / TLS); cardinality (SPSC/SPMC/MPSC/MPMC) is emergent
from locus connectivity at link time, not a runtime config.
Both documented below.

This is part of the alpha-conjecture program (see
`~/notes/alpha-conjecture/CLAUDE.md`). Lotus is the language-substrate
arm — a programming language whose primitives are the framework's
coordination primitives.

## Where we are

A working compiler that **runs** lotus programs end-to-end (tree-
walking interpreter) AND **produces** native ELF binaries (LLVM via
inkwell) for a substantial subset including loci with `run()` and
parent-child `accept()` lifecycle methods. 91 tests pass across
the workspace.

```
$ lotus run examples/02-parent-child/main.lt    # interpreter path
greeting from child: hello
greeting from child: hi
greeting from child: yo

$ lotus build examples/02-parent-child/main.lt  # codegen path
built: examples/02-parent-child/main
$ ./examples/02-parent-child/main
greeting from child: hello
greeting from child: hi
greeting from child: yo
```

Phase status:
- **Phase 0** (spec stabilization) — complete
- **Phase 1** (lex / parse / typecheck) — complete; F.1–F.18 enforced
- **Phase 2 v0** (interpreter + bus router) — 54 of 55 example
  projects execute end-to-end via `lotus run` (only multi-binary
  trellis-pair waits on cross-process bus)
- **Enums arc complete** (m47 base + m47-followups + m47-payloads
  + m47-payloads-followups). Spans the whole toolchain and lands
  full substrate parity between interpreter and codegen for any
  enum a v0.1 program is likely to need.

  Surface coverage:
  - Decl: `type X = enum { A, B(Int), C(Decimal, String) };` —
    any mix of no-payload and payload-bearing variants.
  - Construction: `X::A` for no-args (parsed as `Expr::Path`),
    `X::B(7)` for with-args (parsed as `Expr::Call` with the
    Path as callee).
  - Match: bare `X::A -> ...`, arg-binding `X::B(n) -> ...`,
    AND literal sub-patterns `X::B(0) -> ...` (specific-value
    arm before a generic catch-all). Payload bindings name local
    values inside the arm body. Guards on payload bindings work
    (`X::B(n) if n > 10 -> ...`). Match exhaustiveness checks
    variant coverage.
  - Equality: `==` / `!=` are deep — tag-equality gate then
    per-variant per-field comparison; `Result::Ok(1) ==
    Result::Ok(2)` correctly returns false in both backends. v0.1
    no Ord operators (declaration order isn't meaningful).
  - Display: `println(v)` and `to_string(v)` produce
    `"X::B(7)"` / `"X::C(3.14, hello)"`; output matches between
    backends byte-for-byte.
  - Composite contexts: works as struct field, array element,
    tuple component, locus param (with default expressions),
    function arg, function return, `self.field = ...` mutation,
    bus payload (publish + subscribe).

  Codegen representation is per-enum:
  - **Pure no-payload enums** (every variant has zero fields)
    stay as i32 tag values — pure value semantics, no allocation.
    Same as the original m47 ship.
  - **Has-payload enums** (any variant has fields) lower to a
    pointer to `{ i32 tag, [N x i8] body }` allocated in the
    current arena. N = max payload byte size across variants
    (8-byte stride per field, 16 for Decimal). Construction
    stores the tag at slot 0 and packs payload fields into the
    body; pattern matching loads the tag through the pointer and
    on match reads each field back out at its offset for the
    arm's bindings. Tag-equality, println, to_string, and bus
    dispatch all use the pointer form. Switch-based dispatch in
    the deep-eq and to-string helpers, with PHIs joining
    per-variant results.

  Interpreter representation: `Value::EnumVariant { enum_name,
  variant_name, payload: Vec<Value> }`. `Expr::Path` covers
  no-args (returns variant with empty payload); `Expr::Call`
  with a 2-segment Path callee covers with-args (evaluates each
  arg, collects into payload). `pattern_match`'s constructor arm
  binds each Wildcard / Binding sub-pattern against the
  corresponding payload position. `values_equal` does
  tag-and-deep-payload comparison; `Value::display` renders
  `EnumName::VariantName(p0, p1, ...)`; `to_string` builtin and
  `~~` closure assertions go through the same path.

  Typechecker: `Expr::Path` (2-segment) and `Expr::Call` (with
  a 2-segment Path callee) resolving against the top scope's
  `TypeKind::Enum` entries return `Ty::Named(enum)`.
  `match_is_exhaustive` treats any constructor arm as covering
  its variant (the inner Wildcard / Binding sub-patterns are
  catch-alls over the payload).

  New helpers along the way:
  - `lotus_decimal_to_string` / `lotus_str_from_decimal` for the
    Decimal payload print path (m48; reused by enum-render).
  - `lotus.enum.<E>.names` global per enum: `[N x ptr]` of
    `"EnumName::VariantName"` strings indexed by tag, used by
    no-payload to_string and seeded by `enum_names_array`.
  - `enum_storage_struct(info)`, `lower_enum_variant_alloc`,
    `load_enum_tag`, `load_enum_payload_fields`,
    `lower_enum_deep_eq`, `lower_enum_with_payload_to_string`,
    `value_to_string`, `str_concat`.
  - `lotus_str_eq` (already existed for String ==) now also fires
    from `lower_match_eq_cmp`'s String / Time arm — used by both
    `match s { "hello" -> ... }` and by the per-field comparison
    inside `lower_enum_deep_eq`.

  Examples: 43-enums (pure no-payload Light state machine; uses
  direct-println of an enum value), 45-enum-payloads (Result +
  Event mixing no-payload Halt, single-arg Tick, multi-arg
  Trade(Decimal, Int); exercises match, direct println,
  deep ==).

- **Phase 3 milestone 56** (design-decision documentation
  pass) — complete. Resolves seven notes/open-questions
  entries via doc-only updates that lock The Design's calls
  before further implementation work. (#8) bus subject →
  transport: deployment config maps subjects to transport
  URLs; runtime owns kernel-level transports (shared memory,
  AF_UNIX, TCP, UDP); stdlib owns protocol adapters (NATS,
  MQTT, gRPC, TLS); source stays transport-agnostic — this
  is the runtime/stdlib split as a Design commitment. (#9)
  same-subject-by-multiple-loci = runtime fan-out; subjects
  are coordination points, not single-owner channels.
  (#10) cross-process payload semantics = copy-into-arena;
  the wire is just a longer copy path; transport adapters
  define their own wire format. (#16) `reorganize` =
  `restart_in_place` lifted to substructure level; parent's
  params reset, children re-attach to the new instance,
  nothing migrates laterally (locks vertical-only-flow at
  the failure-recovery boundary; impl deferred until a
  workload exercises it). (#17) drain/dissolve as recovery
  ops resolved by m55's removal — see below. Generics
  direction: extends ProjectionClass with `Numeric` bound
  (Int / Float / Decimal / Duration) for v1; locks generic
  + bus-payload (monomorphize, no special handling), generic
  + closure (T: Numeric for tolerance, otherwise literal-
  only) interactions. New "Bus handler shape" §: payload
  defaults stay rejected as spec position (codegen has done
  this since m34; m54 confirmed the design call). spec/
  memory "Region size tuning" rewritten: declared params
  are sizing hints, not ceilings; arenas grow linked-list
  chunks indefinitely (matches actual C-runtime behavior,
  drops the stale "panic on exceed" language). 96 unit
  tests pass; 54 examples build native + same 11 known
  scheduler-ordering diffs. Codegen / interpreter / runtime
  unchanged.

- **Phase 3 milestone 55** (recovery vocabulary cleanup) —
  complete. Removes `Drain` and `Dissolve` from
  `RecoveryOp` enum per The Design call: drain and dissolve
  are *lifecycle methods*, not recovery operations; their
  recovery-context spelling overlapped with `bubble(err)`
  semantically (failure propagates up, runs the locus's
  drain → dissolve → arena_destroy lifecycle). Two
  spellings for one concept violates substrate-invariance.
  Recovery vocabulary is now `restart` / `restart_in_place`
  / `quarantine` / `bubble` + `reorganize` — five
  primitives. Parser dispatch + parse_recovery_stmt match
  arm + AST enum updated. `drain;` / `dissolve;` at
  statement position now produces a clean parse error
  ("expected expression, got Drain" — falls through to
  parse_expr). Lifecycle method declarations
  (`drain() { ... }` / `dissolve() { ... }`) unaffected —
  the corpus only ever used drain/dissolve in declaration
  position, so no source updates were required. 96 unit
  tests pass; 54 examples build native + same 11 known
  scheduler-ordering diffs.

- **Phase 3 milestone 54** (default params on mode methods) —
  complete. Closes the m34 deferral. m32 shipped defaults on
  free fns, m34 extended them to locus `fn` methods called via
  `self.method(...)`; modes were left out at codegen-declare
  time with a "not yet lowered" rejection on any defaulted
  param. The CHECKPOINT polish entry blamed F.10 for a
  "tighter param surface", but design-rationale F.10 only
  governs mode-keyword-as-member-name parsing — not param
  shape. m54 lifts the rejection and replaces it with the same
  suffix-only ordering check the locus `fn` path uses.
  `lower_self_method_call` already dispatches Fn and Mode
  through the same MethodSig-based default-fill-in path
  (program-walk that returns a uniform `(params, ret)` tuple
  whether the source was LocusMember::Fn or LocusMember::Mode),
  so no call-site changes were required. Interpreter parity is
  automatic: `lookup_method` synthesizes an FnDecl from a Mode
  decl carrying its full `params` (with defaults), and standard
  fn-call evaluation handles the omit-trailing-args path.
  Bus-subscribed handlers stay correctly rejected: their
  single payload param is always provided by dispatch, so a
  default on it would never fire — keeping the rejection
  documents the constraint rather than introducing dead
  syntax. New `examples/50-mode-defaults/`: `mode bulk(scale:
  Int = 10) -> Int` called both bare (`self.bulk()` → 100) and
  overridden (`self.bulk(2)` → 20). Both backends
  byte-identical. 96 unit tests pass; 54 examples build
  native + same 11 known scheduler-ordering diffs as pre-m54.

- **Phase 3 milestone 53** (free-fn handle-rooting) —
  complete. Closes the second clause of spec/memory.md "Free
  `fn` functions": fn return waits for in-fn-bound children
  to dissolve. m49 shipped the arena half; m53 ships the
  lifecycle half. `flush_dissolve_frame` is hoisted from the
  body-fall-through arm of `lower_user_fn_body` into
  `emit_fn_exit_epilogue`, so every exit path (open-end,
  `return;`, `return v;`) goes through one uniform flush.
  The flush emits `lotus_bus_queue_drain` first (so cells
  enqueued by the fn body's loci during their birth() get
  dispatched while subscribers are still alive), then walks
  the deferred-dissolve frame in reverse — for each long-
  lived locus: optional pinned-thread join + mailbox shutdown,
  then drain method, dissolve_closures fn, dissolve method,
  bus deregister, arena destroy. Pre-m53 the typed-return
  path silently popped the frame, leaking the loci. m51's
  46/47 fn-arena examples didn't catch it because their free
  fns don't bind long-lived loci. New
  `examples/49-fn-handle-rooting/`: `make_pair(n) -> Int`
  binds Watcher (subscribes "evt") and Pulse (publishes
  "evt" at birth); Pulse's birth-publish enqueues a cell for
  Watcher; the typed-return triggers flush at fn.exit which
  drains the cell (Watcher prints "watcher got: 42") and
  dissolves both loci before the fn returns. Both backends
  produce byte-identical output. 96 unit tests pass; 53
  examples build native; same 11 known scheduler-ordering
  diffs as pre-m53. With m53 the free-fn-implicit-locus arc
  (m49 + m51 + m53) is fully spec-aligned: arena owns body
  allocations + heap-typed return-copies; lifecycle waits on
  bound children. The `LocusRef` return case stays rejected
  (locus references shouldn't cross arena boundaries — pass
  via bus instead).

- **Phase 3 milestone 52** (in-loop drain after each dissolve) —
  complete. Closes the m26 v0 limitation: cells enqueued by a
  dissolve method's bus publishes used to sit in the
  cooperative queue until `emit_bus_queue_destroy` freed the
  storage at main exit (CHECKPOINT noted "realistic programs
  don't publish during dissolve" + "fix would be a
  drain-loop-until-empty wrapper"). Design call resolved as:
  drain after EACH iteration of the dissolve loop in
  `flush_dissolve_frame_kind`, not just at the top. So if
  locus N's dissolve method publishes, the drain at the end of
  N's iteration dispatches the cells to still-alive later-iter
  subscribers before those subscribers themselves dissolve.
  The deregister-on-dissolve invariant (m45-followup-2)
  guarantees a cell never targets the just-dissolved locus —
  by the time N's dissolve method ran, N had already been
  deregistered, so any subject N publishes to dispatches to
  the OTHER still-registered subscribers, not back to N. Drain
  is therefore safe at every iteration. The C-runtime drain
  loop pops until the queue is empty at pop time, so
  chain-reactions where a fired handler publishes more cells
  get caught in the same drain pass. New
  `examples/48-publish-during-dissolve` exercises it: a Sender
  locus publishes "farewell" in its dissolve, a Receiver
  locus (instantiated first → bottom of the deferred-dissolve
  stack → dissolves last) receives it. The pre-m52 path would
  have leaked the message; m52 dispatches it. Updated comment
  on `emit_bus_queue_destroy` to reflect that the queue is
  expected empty by destroy time. 96 workspace tests still
  pass; 52 examples build native + same 11 known
  scheduler-ordering diffs as pre-m52.

- **Phase 3 milestone 51** (free-fn return-copy completion) —
  complete. Closes the three "Unsupported" branches m49 left in
  `emit_return_value_deep_copy`: Array, TypeRef-struct, and
  has-payload-Enum. Each is a recursive walk on the lotus type
  structure that allocates a fresh storage value in the
  caller's arena, copies each component (recursing through any
  heap-typed sub-component — e.g. a String field inside a
  returned struct), and returns the new pointer. Array case
  uses `llvm_array_storage_type` to size the alloc + GEP each
  slot. TypeRef-struct case looks up `TypeInfo` from
  `user_types`, walks `field_order`, and copies each field by
  struct slot index. Has-payload-Enum case is the most
  involved: a new `emit_enum_payload_deep_copy` helper that
  loads the tag, switches into per-variant blocks, calls
  `load_enum_payload_fields` to read each field's value, deep-
  copies each one through `emit_return_value_deep_copy`, then
  calls `lower_enum_variant_alloc` (with
  `current_arena_override = Some(dest_arena)` so the new enum
  storage struct lands in the caller's arena, not the fn
  subregion), and PHI-joins the per-variant pointers in a
  `cont` block. Default switch case is unreachable in
  well-typed programs but emits a null PHI incoming to keep
  IR well-formed. The only remaining LotusType not supported
  for free-fn return is `LocusRef` — by design those shouldn't
  cross arena boundaries (pass via bus). New
  `examples/47-fn-arenas-extras/` exercises all three paths
  end-to-end (`[Int; 4]` array, `Point` struct with String
  field, `Result::Ok / Result::Err` enum), with byte-identical
  interpreter / codegen output. 51 of 52 examples build native
  + same 11 known scheduler-ordering diffs as pre-m51. 96
  workspace tests pass.

- **Phase 3 milestone 50** (immutable-binding enforcement) —
  complete. Closes notes/open-questions #23, the documented
  spec-vs-impl drift on `let` immutability. Spec/types.md
  "Mutability" + design-rationale §E always said `let x = 0;
  x = 1;` should be a compile-time error and only `let mut x`
  should permit reassignment; the typechecker silently allowed
  the violation through m49. m50 tightens the typechecker:
  `LocalSym` (in `crates/lotus-types/src/check.rs`) now
  carries `is_mut: bool`; `Stmt::Let` and `Stmt::LetTuple`
  propagate the AST `is_mut` flag; the four other
  symbol-insertion sites (locus lifecycle params, mode params,
  `on_failure` params, free-fn / locus-method params, and
  `for x in ...` loop vars) default to false per spec —
  params are immutable bindings (F.10); loop vars rebind
  fresh each iteration. `Stmt::Assign` to a bare-head local
  (target.tail.is_empty() && head ≠ "self") looks up the
  symbol's is_mut and pushes a diagnostic when false. Field
  and index reassignment through an immutable head stays
  allowed: `let s = Point { x: 0, y: 0 }; s.x = 7;` is fine
  because the head binding isn't being rebound; only locus
  state is being mutated. `self.field = ...` in lifecycle
  methods is also allowed unconditionally (locus state is
  mutable by design, separate from any binding's is_mut).
  Diagnostic format: `cannot assign to ` + name + `: binding
  is immutable. Declare with let mut ` + name + ` to permit
  reassignment.` Five new unit tests cover the matrix (immut
  bare reassign / let mut reassign / fn-param reassign /
  for-loop-var reassign / self.field through immut). All 96
  workspace tests pass (was 91 + 5 new); 50 examples still
  build native + same 11 known scheduler-ordering diffs as
  pre-m50 — the corpus already wrote `let mut` discipline
  correctly so no source updates were needed. Codegen and
  interpreter unchanged: enforcement is purely at the type
  layer. Spec parity is now restored at the binding boundary.

- **Phase 3 milestone 49** (free-fn implicit-locus arenas) —
  complete. Closes the m20 deferral that kept all free-fn
  allocations on the program-wide `arena.global` until process
  exit. Per spec/memory.md "Free `fn` functions" §, every free
  function should have its own implicit locus — m20 punted on
  the per-call arena, m49 ships it. Codegen-only (the
  interpreter relies on Rust ownership and is unaffected). Each
  non-main free fn now takes an implicit `__caller_arena: ptr`
  first param at the LLVM ABI; at body entry the callee opens
  a subregion of `__caller_arena` via
  `lotus_arena_create_subregion`, stores the handle in a
  fn-local alloca, and `current_arena_ptr` falls through to it
  (a new tier between `current_self`'s arena and `arena.global`,
  the latter now reachable only from `main`'s body). Body
  allocations route through the subregion; on every `return`
  the body branches to a unified `fn.exit` epilogue that
  deep-copies the return value into `__caller_arena`, destroys
  the subregion wholesale, and emits `build_return`. Refactor
  routes Stmt::Return through the exit block via a ret-value
  alloca + br instead of inline build_return so the destroy +
  copy never duplicate at every return site. Lifecycle methods
  (mode, run, accept, drain, dissolve, on_failure, bus
  handlers) keep their direct build_return path — they don't
  own a subregion (they run *as the locus*) and `lower_return`
  picks the path by checking whether `current_user_fn_exit_bb`
  is set. Deep-copy is a small recursive helper:
  `emit_return_value_deep_copy(value, ty, dest_arena)` — value
  types (Int / Float / Bool / Decimal-i128 / Time / Duration /
  no-payload-Enum) identity; String calls a new
  `lotus_str_clone(arena, src)` C-runtime helper (sibling of
  `lotus_str_concat` minus the right operand); Tuple allocates
  a fresh storage struct in dest_arena and recursively copies
  each field (so a `(Int, String)` return deep-copies the
  String buffer too). Heap returns of Array, TypeRef-struct, or
  has-payload-Enum reject for v0.1 — none currently appear as
  free-fn returns; ship as a follow-up when a workload demands.
  `main` keeps its existing `arena.global` setup unchanged.
  Bound handles in free fn bodies still attach to the enclosing
  deferred-dissolve frame (lifecycle parity with main); the
  full implicit-locus *handle-rooting* semantic — fn return
  waits for in-fn-bound children to dissolve as if the fn were
  a locus — remains future-work, not exercised by any current
  example. New `examples/46-fn-arenas/` exercises the path:
  a `decorate(name, n) -> String` that allocates two intermediate
  concats inside the subregion called in a `for i in 1..=5`
  loop, plus a `pair(a, b) -> (Int, String)` exercising
  recursive deep-copy through a tuple. Both backends produce
  byte-identical output. Existing examples 27-strings,
  26-tuples, 09-functions, 23-ranges, 24-default-params,
  28-to-string, 29-helpers, 21-arrays, 13-decimal-and-exit,
  43-enums all still pass parity (their free-fn returns now
  route through the new code path; the surface output is
  unchanged because parity was always defined at stdout, not at
  arena residency). All 91 unit tests still pass.

- **Phase 3 milestone 48** (Decimal fixed-point) — complete.
  v0 stored Decimal as a String (interpreter) / f64 (codegen) and
  round-tripped through `parse::<f64>()` for arithmetic, masking
  dust via `%g` formatting. m48 replaces both with exact i128
  fixed-point. **Interpreter:** `DecimalVal { mantissa: i128,
  scale: u32 }` with per-value scale — source spelling round-trips
  (`100.40d` keeps scale=2). Add/Sub align to max scale; Mul
  multiplies mantissas and adds scales; Div picks `max(a.scale,
  b.scale, 9)` so `1d/3d` keeps useful precision (`0.333333333`)
  rather than truncating. `display` strips trailing zeros so
  `1.50d` prints `"1.5"`. `eval_binop` Decimal arms call
  `DecimalVal::add/sub/mul/div/cmp` directly; `Value::Decimal`
  stores the struct (not the string). **Codegen:** `LotusType::Decimal`
  lowers to LLVM i128 with implicit fixed scale 9; per-value
  scale would require an LLVM struct + per-op alignment, and one
  fixed scale produces matching output after the trailing-zero
  trim. Decimal literals lower via `parse_decimal_to_i128_scale9`
  + `i128_const`; arithmetic uses `build_int_add`/`sub`/`mul`/
  `signed_div` with the scale-9 adjustment in mul (divide by 10^9
  after the multiply) and div (multiply numerator by 10^9 before
  the divide); comparison uses signed integer predicates. Unary
  `-` is `0 - x`. New C-runtime fns `lotus_decimal_to_string(hi,
  lo, buf)` and `lotus_str_from_decimal(arena, hi, lo)` render
  i128 → trimmed string; codegen splits the i128 into i64 hi:lo
  via `lshr`/`trunc` (the i128 ABI for direct passing isn't
  uniformly modeled by inkwell). `println` Decimal allocates a
  64-byte stack buffer, calls the helper, and splices in as %s.
  `to_string` Decimal calls the arena-allocating helper.
  Closure-check (`~~ within`) for Decimal merges into the int-ops
  arm with i128 zero. Mean accumulator for Decimal divides the
  i128 sum-as-f64 by 10^9 before the count division. ParamValue
  carries i128 mantissa instead of f64. Violation `diff` field
  stays i64 — Decimal closures truncate i128 → i64 for the
  diagnostic store (precision loss past 2^63 mantissa-units is
  acceptable since diff is never recomputed). New
  `examples/44-decimal-exact/` exercises arithmetic that
  previously diverged from interpreter or hit f64 dust:
  `0.1d - 0.05d → 0.05`, `1d / 3d → 0.333333333`, compound
  `1000 × 1.05^2 → 1102.5`, `1.5d - 10.25d → -8.75`. Both
  backends produce identical output. Overflow at v0.1 is
  unchecked — same policy as Int. i128 fits operands up to
  ~10^19 cleanly through mul/div; per-value-scale operands
  pushing past that would need wider intermediates which v0.1
  defers.

  Decimal+Duration parity follow-ups landed as small
  per-feature commits alongside the enums arc:
  - `%` operator on Decimal in both backends (mantissa
    int-mod after scale-align in the interpreter; direct
    `build_int_signed_rem` on the i128 in codegen).
  - `~~ within` accepts Decimal AND Duration in the
    interpreter (`approx_pass` does exact i128 mantissa
    sub/abs/cmp for Decimal and i64-ns arithmetic for
    Duration; previously only Int/Float).
  - Decimal + Duration literal patterns parse in `match`
    arms (`match x { 1.0d -> ..., 100ms -> ..., _ -> ..., }`);
    `literal_matches` resolves Decimal via DecimalVal::parse +
    DecimalVal::eq so source-spelling differences in scale
    don't disagree.

- **Phase 3 milestone 47** (enums — tagged union + match) —
  complete. v0.1 ships no-payload variants only; payload-bearing
  variant decls are rejected upstream at typecheck/codegen
  registration. Surface: `type Light = enum { Red, Yellow, Green };`
  declares; `Light::Red` constructs; `match l { Light::Red -> ..., }`
  destructures. Codegen represents each variant as its declaration-
  order index in an `i32`; `LotusType::Enum(String)` wraps the
  type name; `Codegen.user_enums: BTreeMap<String, EnumInfo>`
  records the variant lists for tag lookup; `lower_expr` adds an
  `Expr::Path` arm that resolves 2-segment paths to a const
  `i32`; `match` lowering adds a `Pattern::Constructor` arm that
  emits scrutinee-vs-tag equality. `llvm_basic_type` /
  `type_expr_to_lotus` / fn-signature blocks / `alloca_for`
  all gain `LotusType::Enum(_) => i32` arms; `lower_println`
  rejects enum values for v0.1 (substring formatting deferred —
  no symbol-table at runtime). Interpreter: `Value::EnumVariant
  { enum_name, variant_name }`; `Expr::Path` recognizes 2-segment
  paths against the existing `types` registry (`TopDecl::Type`
  with `TypeDeclBody::Enum` body); `pattern_match`'s
  `Pattern::Constructor` arm matches the variant's enum_name +
  variant_name. Typechecker: `match_is_exhaustive` extended —
  for `Ty::Named` pointing at a `TypeKind::Enum`, every
  unguarded constructor pattern's variant name is collected and
  the function returns true iff every declared variant is
  covered. New `examples/43-enums/` exercises a Light state
  machine through both backends; outputs match.
- **Phase 3 milestone 46** (closure accumulators) — complete.
  Closes the streaming-fold half of F.9: `closure C { sum(self.X)
  ~~ Y within Z; ... }` accumulates `self.X` across every epoch
  fire and the assertion reads the running total. Each `sum(expr)`
  detected in left/right/tolerance gets one struct field on the
  locus (`__acc_<n>: T`); on each epoch fire the synthesized
  closure-eval fn re-evaluates the inner expr, adds to the slot,
  and only THEN evaluates the assertion (so the assertion observes
  the post-update total — natural reading of "sum across cells
  through this moment"). Detection walks the assertion AST in
  declaration order — `Expr::Sum(Box<Expr>, Span)` is a dedicated
  AST variant the parser already produces. Recovery interaction
  matches the spec example: by default a recovery event
  (restart / restart_in_place / quarantine) zeroes accumulators;
  `persists_through(restart_in_place, quarantine)` opts that
  closure's accumulators out of zeroing for the named events.
  Codegen: `AccumulatorSlot { inner_expr, ty, field_idx }` per
  slot, stored on `LocusInfo.accumulators_per_closure`; new
  `accumulator_ctx: Option<AccumulatorCtx>` on Codegen state set
  during assertion lowering so `Expr::Sum` substitutes a load
  from the next slot instead of re-evaluating; recovery dispatch
  in `lower_restart_call_kind` and `lower_quarantine_call`
  invokes `emit_accumulator_reset_for_event` after their existing
  bookkeeping. Interpreter parity via `LocusHandle.accumulators`
  (`Rc<RefCell<BTreeMap<String, Vec<Value>>>>`) — slots lazy-init
  on first sample using each sample's runtime type for the zero;
  `Interpreter::accumulator_ctx` enables the same substitution
  in `Expr::Sum`'s eval arm; `reset_accumulators_for_event`
  fires from RecoveryOp dispatch. Vocabulary at v0.1: `sum(self.X)`
  for Int / Float / Decimal / Duration. Inner expr restricted to
  `self.X` reads — type comes straight from the locus's params,
  no AST-time inference pass needed. count() / mean() / rolling
  windows deferred until a workload calls for them. Parser
  extended to accept recovery-event keyword tokens (restart,
  restart_in_place, quarantine, dissolve) as bare names inside
  `persists_through(...)` / `resets_on(...)` — the spec example
  spelled them as bare keywords and the prior identifier-only
  parser rejected them. New `examples/41-closure-accumulator/`
  exercises a Tracker locus where running-total drift past a
  band of 100 trips the closure; Coordinator absorbs the
  violation and quarantines.
- **Phase 3 m45-followup-2 + m43-followup** (bus router
  proper-fix + pinned-duration wrapper) — complete. Two
  substrate fixes that close prior v0 limits without adding new
  surface. (a) **bus.entries proper fix.** The m45-followup
  quickfix bumped the LLVM-side bus table to `decl_subs × 32`
  to unblock multi-instance subscribers; this swap migrates
  bus storage out of LLVM entirely. New C-runtime fns
  `lotus_bus_register` / `lotus_bus_dispatch` /
  `lotus_bus_quarantine_self` / `lotus_bus_router_destroy`
  back a heap-grown dynamic vec (initial cap 16, doubles on
  demand). The hand-rolled LLVM dispatch fn body (~330 lines:
  header / strcmp / call / mailbox-post / enqueue / inc) is
  gone. `bus_entry_type` is gone. `BusState` shrinks from
  `{entries, count, capacity, dispatch_fn}` to a unit marker
  that just records "program contains at least one
  subscribe." `init_bus_state` no longer takes a capacity.
  `emit_bus_register` is one C-runtime call.
  `lower_quarantine_call`'s 130-line entries walk is one
  C-runtime call. `lower_send` calls
  `lotus_bus_dispatch(queue, subject, payload, size)` — the
  queue still lives in LLVM (its lifecycle is bound to main's
  prelude/exit) but the entries vec lives in the C runtime.
  Net delta: codegen.rs lost ~450 lines; lotus_arena.c gained
  ~95. There's no compile-time capacity ceiling anymore.
  `emit_bus_queue_destroy` also calls
  `lotus_bus_router_destroy` so the entries vec frees at
  process exit alongside the queue. (b) **Pinned-duration
  wrapper.** m43 documented "v0 limit: pinned-thread
  post-run() doesn't fire duration; cooperative + post-handler
  bus paths fire duration correctly." Cooperative was fine
  because lifecycle dispatch in `lower_locus_instantiation`
  calls `duration_closures_fn(self, parent, on_failure)`
  directly with parent fields from `resolve_failure_route`.
  Pinned post-run() ran on the pinned thread, where there's
  no `current_self` for that helper, so duration was simply
  skipped. Tick had the same problem and m42 solved it via
  `__tick_closures_wrapper(self_ptr)` — a 1-arg adapter that
  loads `__parent_self` + `__parent_on_failure` from the
  struct (baked at instantiation by the parent) and forwards
  to the 3-arg fn. The fix mirrors that exactly: new
  `duration_wrapper_fn` field on LocusInfo,
  `__duration_closures_wrapper` synthesized when the locus
  has any duration closure, called from `__pinned_main_<Locus>`
  after `run()` alongside the tick wrapper. The wrapper
  bodies are now generated by a shared 2-iteration loop over
  `(tick, duration)` pairs in pass C. New
  `examples/40-pinned-duration/` exercises the path: a
  `Heartbeat : schedule pinned` whose `run()` sleeps 50ms,
  with an always-fail duration(20ms) closure absorbed by
  `Coordinator.on_failure` — pre-fix the violation was silent
  because the wrapper never ran; post-fix Coordinator prints
  "duration fired post-run on pinned (#1)".
- **Phase 3 milestone 45** (restart_in_place recovery primitive
  + bus.entries multi-instance fix) — complete. Two related
  fixes shipped together. (1) `restart_in_place(c)` is a
  factory-reset variant of m40's `restart(c)` that zeros user
  fields back to declared defaults BEFORE re-running birth(),
  rather than preserving whatever state the previous attempt
  left. Both share the cap-2 budget on the same retry counter.
  When to choose which: plain `restart` for "advance the state
  machine forward" patterns, `restart_in_place` for "got into
  an inconsistent state, retry from clean" patterns. Codegen:
  one synthetic `__restart_in_place_pending: i64` flag added
  to every locus struct after `__quarantined`; zero-init at
  instantiation; set by `restart_in_place(c)`; the rerun
  branch in `__birth_closures` gates on it (zero-fields block
  re-evaluates each declared default into its slot, then
  clears the flag, before falling into the call_birth block).
  Interpreter: LocusHandle.restart_in_place_pending: Cell<bool>
  with the same handshake. New `examples/38-restart-in-place/`.
  (2) bus.entries capacity bumped to `total_subs × 32` so
  multiple instances of the same subscribed locus type can
  each register their own runtime entry without overflowing
  the global. Pre-fix limit was "use distinct types"
  (workaround in 34-quarantine-bus); fix unblocks the
  natural multi-instance pattern. New
  `examples/39-multi-instance-bus/` exercises three Watcher
  instances on one subject. Proper fix is to migrate
  bus.entries storage to a C-runtime dynamic vec; captured
  in next-steps as future polish.
- **Phase 3 milestone 44** (explicit-epoch closures —
  user-triggered checkpoint) — complete. Closes the closure-
  epoch lowering arc: all five epochs Birth + Dissolve +
  Tick + Duration + Explicit now lower in both interpreter
  and codegen. Where the other four fire automatically at
  scheduler / lifecycle boundaries, explicit fires only
  when the user calls `check_closures();` from inside the
  locus's body — useful for "audit at THIS specific
  checkpoint" patterns where the locus author knows
  precisely when an invariant should hold (e.g. the
  double-entry balance closure in 37-explicit-closures
  is meaningful only at transaction-boundary checkpoints,
  not after each individual leg). Codegen: synthetic
  `__explicit_closures(self, parent, on_failure)` fn lowered
  through the same per-epoch loop used by Birth + Dissolve +
  Tick. New `lower_check_closures_call` helper detects the
  builtin in `Stmt::Expr(Call(Ident("check_closures")))`
  and emits a direct call, reading parent / on_failure
  from the m42 `__parent_self` / `__parent_on_failure`
  struct fields. Silent no-op when the locus has no
  explicit-epoch closures. Interpreter: LocusHandle gains
  a `parent` field set at instantiation (parent_stack.last()
  at that moment); `Expr::Call` intercepts ident
  `check_closures` before normal callee evaluation and
  fires `fire_explicit_closures(handle, handle.parent)`.
  Skipped on quarantined loci. New
  `examples/37-explicit-closures/`.
- **Phase 3 milestone 43** (duration-epoch closures —
  every-N-of-monotonic-time gate) — complete. `closure C
  { ...; epoch duration(N); }` evaluates at substrate-cell
  boundaries (alongside tick) but only fires when at least
  N of monotonic time has elapsed since the closure's
  last fire (or since instantiation for the first). Where
  m42's tick fires every cell, duration fires every-N-of-
  time — the right epoch for drift / heartbeat / rate-
  limited audit invariants. F.9 routing reused unchanged.
  Codegen: per-locus struct gains one i64
  `__duration_last_fire_<i>` field per duration closure
  (declaration order); synthetic
  `__duration_closures(self, parent, on_failure)` fn
  lowers each gate inline (load last, get monotonic-now,
  compute elapsed, compare elapsed >= N, on fire store now
  -> last BEFORE assertion runs so an absorbed violation
  in on_failure doesn't reset the interval clock).
  Instantiation seeds each last-fire to monotonic-now via
  the existing `lower_time_monotonic` helper. Call sites
  shared with tick: after each subscribed handler body
  (before m26 tail bus drain) and after run() returns.
  Interpreter: LocusHandle.duration_last_fire: Vec<i64>
  parallel to declared duration closures; new
  `closure_fires_at_duration` predicate +
  `duration_expr_for(c)` accessor +
  `fire_duration_closures` helper. Duration expression
  evaluated in self-scope at fire-check time so
  `duration(self.poll_interval)` works. The original m43 v0
  limit (pinned post-run() didn't fire duration) was closed
  by m43-followup via `__duration_closures_wrapper`. New
  `examples/36-duration-closures/`.
- **Phase 3 milestone 42** (tick-epoch closures — F.9 substrate
  steady-state pulse) — complete. Where m39 audits at birth and
  dissolve audits at end-of-life, m42 lights up the "between
  cells" beat: closures with `epoch tick;` fire after every
  bus handler invocation on the locus AND after `run()`
  returns. Birth + Dissolve are lifecycle bookends; tick is
  the steady-state monitor — useful for drift / accumulator
  overflow / any property that can be violated mid-life by a
  single handler. F.9 routing reused unchanged: tick
  violations reach the parent's on_failure, which can absorb,
  bubble, or quarantine. m41b's bus-dispatch quarantine
  gating composes naturally — a parent that quarantines on
  tick violation silences the locus from receiving further
  bus messages, AND existing queued cells observe the gate
  via a new __quarantined entry-check on subscribed handlers
  (closes a pre-existing m41b gap). Codegen: synthetic
  `__tick_closures(self, parent, on_failure)` fn (3-arg
  shape mirrors birth/dissolve); synthetic
  `__tick_closures_wrapper(self)` adapter for the pinned
  thread post-run path (loads parent fields baked onto the
  struct at instantiation); two new ptr fields
  (`__parent_self` + `__parent_on_failure`) appended to the
  locus struct for that wrapper to read. Tick call inserted
  inline into subscribed user-fn bodies just before the m26
  tail bus_queue_drain — the thunk-wrapper draft broke
  ordering because the handler's own tail drain recursively
  processed queued cells before the thunk's tick step ran.
  Interpreter: Subscription gains a `parent: Option<...>`
  captured at subscribe time; dispatch_bus uses it to route
  violations to the correct on_failure;
  `closure_fires_at_tick` predicate +
  `fire_tick_closures` helper called after each
  `run_handler` and after `run()` returns. Duration +
  Explicit epochs still reject pending the runtime epoch
  engine. New `examples/35-tick-closures/`.
- **Phase 3 milestone 41b** (bus-dispatch quarantine gating)
  — complete. Closes the v0 gap m41 left open: quarantined
  subscribers now stop receiving bus messages, completing
  the "stop trying" semantic. Codegen: dispatch's check_bb
  null-checks entry.subject before strcmp; quarantine(c)
  walks bus.entries (bounded by bus.count) and nulls subject
  of any entry whose self matches the quarantined child.
  Interpreter: `dispatch_bus` checks
  `delivery.subscription.locus.quarantined` and skips. The
  quarantine recovery primitive is now substrate-complete:
  gates run() (m41) AND bus dispatch (m41b), both via the
  same `__quarantined` flag. Plus a small dev affordance:
  `LOTUS_DUMP_IR=1` env var writes the LLVM IR alongside
  the .o file for debugging codegen issues.
  New `examples/34-quarantine-bus/`.
- **Phase 3 milestone 41** (quarantine recovery primitive —
  sticky-flag F.9 sibling) — complete. Where m40's
  `restart(c)` is "give birth another shot," m41's
  `quarantine(c)` is the "stop trying" response: sets a
  sticky flag on the locus; the lifecycle dispatch checks
  it after birth + `__birth_closures` and skips `run()`
  if set. Drain / dissolve still fire (cleanup is
  unconditional). Codegen: synthetic `__quarantined: i64`
  flag appended after `__restart_count`, zero-init at
  instantiation; `quarantine(c)` writes 1 via GEP+store;
  run() gated by load+icmp+cond branch. Interpreter mirrors
  via `LocusHandle.quarantined: Rc<Cell<bool>>` +
  `RecoveryOp::Quarantine` setting it +
  `instantiate_locus` skip on Run lifecycle. Bus-dispatch
  gating (quarantined subscribers stop receiving) waits on
  m41b — needs a fixed-offset C-runtime load. The two
  recovery primitives now form a small response menu
  inside on_failure: restart for retry-with-budget,
  quarantine for stop-trying. Both can fire; quarantine
  wins. New `examples/33-quarantine/`.
- **Phase 3 milestone 40** (restart recovery primitive — F.9
  response half) — complete. m39 delivered the trigger half
  (birth-epoch closures detect violations); m40 delivers the
  response half. From inside `on_failure`, the parent calls
  `restart(child);` to bump a per-locus retry counter; if
  the new count is within the v0 cap (2 attempts per locus
  lifetime), the runtime re-runs `birth()` + the entire
  birth-epoch closure sequence on the SAME memory.
  Semantics: "give birth another shot" — the locus's state
  at the start of the second attempt is whatever the first
  attempt left it in. Past the cap, `restart()` still bumps
  the counter (observable) but the runtime skips the
  re-run; the violation falls through to the parent's
  collapse path. Cap is design-time-parameterized at 2 by
  default per user framing; could become a per-locus
  annotation later. Runtime cost: one i64 load + add +
  store per `restart()` call, no hot-path branch when
  restart isn't used.
  Codegen: synthetic `__restart_count: i64` field on every
  locus struct, zero-init at instantiation;
  `lower_closure_check` extended with an epoch parameter so
  the birth-epoch path captures pre/post counts around the
  on_failure call, conditionally branching to a `rerun_bb`
  that calls `birth()` + recursively calls
  `__birth_closures` + `ret void`.
  Interpreter: `LocusHandle` gains a
  `restart_count: Rc<Cell<i64>>`; `RecoveryOp::Restart`
  bumps it unconditionally; `instantiate_locus`'s
  birth-epoch evaluation rewritten as a depth-bounded loop
  that re-runs birth + closures on bumped-within-cap.
  New `examples/32-restart/`. With m39 + m40, the F.9
  invariant-and-repair pair is now substrate-complete for
  the birth + dissolve epochs.
- **Phase 3 milestone 39** (birth-epoch closures — F.9
  substrate deepening) — complete. Pre-m39 only
  dissolve-epoch closures lowered, so invariants could
  only be checked at end-of-life; m39 lights up
  birth-epoch closures: assertions evaluated right after
  `birth()` returns, before `run()` touches state. Same
  F.9 routing as dissolve — violation reaches the
  parent's `on_failure` if matching, else stderr+exit.
  `LocusInfo.closures` gains an `EpochSpec` tag per entry
  so the synthesis pass partitions by epoch; `closures_fn`
  split into `birth_closures_fn` + `dissolve_closures_fn`
  (renamed from `closures_fn`). Lifecycle dispatch in
  `lower_locus_instantiation` now sequences birth() →
  `__birth_closures` → run() (existing drain →
  `__dissolve_closures` → dissolve still runs at the end).
  Default closures (no `epoch` clause) stay dissolve-only
  so no pre-existing example's stdout shifts. Tick /
  Duration / Explicit still reject with a clear "covers
  Birth + Dissolve" diagnostic — those need the runtime
  epoch engine. Interpreter parity via
  `closure_fires_at_birth` predicate + a birth-eval block
  in `instantiate_locus`. New
  `examples/31-birth-closures/`. Why "bottom of the
  locus-of-design": closures are F.9's first-class
  invariant primitive; epoch is *when* the invariant is
  checked. Birth + Dissolve are the two epochs that
  don't need the runtime epoch engine, so they're the
  substrate's foundational layer. Recovery primitives
  (`restart` / `quarantine` / `reorganize`) sit on top —
  they're the *response* to a violation; m39 delivers
  the trigger half, recovery is the natural next
  substrate milestone.
- **Phase 3 milestone 38** (stdlib helpers — math + string
  predicates) — complete. Five small language-native helpers
  that fill the most-common gaps before deeper-substrate
  work: `min(a, b)` / `max(a, b)` / `abs(x)` across Int /
  Duration (signed integer compare + select) and Float /
  Decimal (float compare + select); `starts_with(s, prefix)`
  / `contains(s, sub)` over String, returning Bool.
  Per spec/stdlib.md these are formally `std::math` and
  `std::string` territory; lotus exposes them as built-ins
  until stdlib resolution wires up — same pattern as
  `print` / `println` / `len` / `to_string`. C runtime adds
  `lotus_str_starts_with` (strncmp wrapper) and
  `lotus_str_contains` (strstr wrapper). Empty prefix /
  sub matches anything (Rust semantics). Codegen
  `lower_math_builtin` covers min/max/abs uniformly with a
  build_int_compare-or-build_float_compare + build_select
  shape; `lower_str_predicate_builtin` dispatches String
  predicates. Interpreter mirrors via `builtin_min` /
  `_max` / `_abs` / `_starts_with` / `_contains`; new
  `parse_decimal_pub` alias added so Decimal min/max can
  compare without duplicating the strip-`d`-then-parse-f64
  logic. New `examples/29-helpers/` covers all five plus
  a `classify_temp` that composes min + max for clamping,
  and a path-prefix filter using `starts_with` over an
  array of operation strings.

  Bonus: `examples/30-stats/` ties the m35 → m38 surface
  together — Producer publishes Samples, Aggregator
  subscribes and emits a single-line summary built via
  concat + to_string + min/max per receipt. Demonstrates
  the substrate's per-locus arena ownership + bus dispatch
  copy semantics under realistic flow.
- **Phase 3 milestone 37** (`to_string` builtin) — complete.
  Closes the "build a string from typed data" gap that m36
  left open. `to_string(x)` returns a String formatted
  exactly like `println` would render the same value: Int
  via %lld, Float / Decimal via %g, Bool as `true`/`false`,
  Duration as `<n>ns`, String passes through. Result lifetime
  follows the m20 / m36 region rule — arena-allocated against
  `current_arena_ptr` so concat-chains land in the caller's
  locus arena. Codegen declares three snprintf-backed runtime
  helpers (`lotus_str_from_int` / `_float` / `_duration`);
  Bool dispatches via `build_select` against true/false
  globals; String returns the input ptr unchanged.
  Interpreter mirrors via `fmt_decimal_pub` (one-line pub
  alias of `fmt_decimal`) so Float / Decimal output matches
  codegen's %g semantics. Per spec/stdlib.md, richer
  formatting (split / startswith / format-template) lives in
  `std::string` once stdlib resolution wires up; `to_string`
  is the language-native primitive those would build on.
  New `examples/28-to-string/`.
- **Phase 3 milestone 36** (string ops — concat / eq / len /
  slicing) — complete. Strings before m36 were opaque
  pointers used only by `println`; m36 lights up the four
  ops that turn them into a usable type for real programs.
  `+` for concat, `==` / `!=` for equality (strcmp wrapper),
  `len(s)` for byte length (strlen), and exclusive +
  inclusive range slicing `s[lo..hi]` / `s[lo..=hi]`. Result
  lifetimes follow spec/memory.md region rules — concat /
  slice land in `current_arena_ptr`, freed wholesale on
  locus dissolution. Bounds-clamp on slice produces a
  (possibly empty) substring rather than panicking; matches
  the substrate "best-effort, predictable" ethos and keeps
  interpreter / codegen output identical. C runtime
  additions: `lotus_str_concat`, `lotus_str_eq`,
  `lotus_str_len`, `lotus_str_slice`. `len` also returns the
  compile-time N for `LotusType::Array(_, N)` (no strlen
  call needed). Codegen extends `lower_binop`,
  `lower_expr`'s `Expr::Index` (Range index → slicing), and
  adds `lower_len_builtin`. Interpreter mirrors via
  `builtin_len` + Range-over-String special case in
  `Expr::Index`. New `examples/27-strings/` exercises all
  four ops + walks byte-by-byte to find a first-word
  delimiter.
- **Phase 3 milestone 35** (tuples — values + patterns +
  destructure) — complete. Tuples are anonymous
  heterogeneous records of fixed arity ≥ 2, lowered as
  pointers to arena-backed anonymous LLVM structs. New
  surface: tuple literal `(a, b)`, numeric field access
  `t.0` / `t.1`, flat `let (a, b) = pair;` destructure, and
  tuple patterns in match arms (sub-patterns: Wildcard,
  Binding, Literal). Tuple types in fn signatures including
  return position — multi-value return without forcing the
  caller to invent a one-off `type` decl. Empty tuple `()`
  rejected per the user's lock-in (no unit-typed surface in
  v0); nested tuple sub-patterns deferred until a workload
  asks. AST gained `Stmt::LetTuple` as a sibling of
  `Stmt::Let`; parser peeks `LParen` after `let mut?` to
  disambiguate. Numeric tuple-field access lands by
  extending parse_postfix to accept an IntLit after `.`
  (digit string stored as the field name) and routing
  through tuple-shaped receivers in codegen / typecheck /
  interpreter. Match-arm refactor: `binding: Option<...>` →
  `bindings: Vec<...>` so tuple patterns can introduce
  multiple bindings; literal-EQ comparison extracted to
  `lower_match_eq_cmp` for shared dispatch. New
  `examples/26-tuples/` exercises divmod returning `(Int,
  Int)`, let destructure, numeric field access, and a
  classify(x, y) match against a (sign(x), sign(y))-shaped
  tuple.
- **Phase 3 milestone 34** (default param values on locus `fn`
  methods) — complete. Locus methods called via
  `self.method(...)` now support default param values
  (suffix-only rule, evaluated at the call site). Bus-subscribed
  handlers reject defaults — bus dispatch is fixed-arity
  `(self, payload)` at the C-runtime level and can't materialise
  extra args at call time. Mode methods (bulk / harmonic /
  resolution) still reject defaults — F.10 keeps their param
  surface tight as designated regime selectors.
  `examples/24-default-params/` extended with a `Counter` locus
  whose `fn bump(step: Int = 1)` exercises the path.
- **Phase 3 milestone 33** (multi-file imports) — complete.
  CLI's `parse_with_imports` walks the entry's
  `import "..."` directives, recursively parses each, dedups by
  canonical path, merges items into one logical Program. Paths
  resolve relative to the importing file's directory with `.lt`
  implicit; cycles short-circuit; `std/*` paths are skipped (the
  toolchain handles `time::*` and friends as built-ins).
  Both `lotus run` and `lotus build` use the merged Program for
  single-file targets. New `examples/25-imports/` is a 3-file
  project (types.lt + notional.lt + main.lt) demonstrating the
  diamond pattern.
- **Phase 3 milestone 32** (default param values on free fns)
  — complete. `fn greet(name: String, greeting: String = "hi")`
  callers may omit trailing args; defaults evaluate at the call
  site in the caller's scope. Suffix-only rule enforced at decl
  time. FnSig grew a `defaults: Vec<Option<Expr>>`.
  New `examples/24-default-params/`.
- **Phase 3 milestone 31** (integer ranges in for-iterators) —
  complete. New `Expr::Range { lo, hi, inclusive }`; parser
  tail-attaches `..` / `..=` at the lowest precedence so
  `for i in 0 .. n + 1` reads naturally; for-stmt handlers in
  both interpreter and codegen special-case Range as a counted
  loop. Range outside iterator position rejects (it's not a
  first-class collection in v0). `examples/23-ranges/` covers
  exclusive + inclusive + break-inside-range.
- **Phase 3 milestone 30b** (indexed local-array assignment) —
  complete. `arr[i] = v` lowers via a GEP into the local
  array's storage + store; compound-assignment ops come for
  free since the assignment path reuses slot-pointer + load +
  store. `self.arr[i] = v` not yet (needs a struct-walk + GEP
  combo). New `examples/22-moving-average/` exercises the
  whole array stack: a sliding-window mean over a `[Int; 4]`
  state array, driven by a bus-published "sample" subject.
- **Phase 3 milestone 30** (arrays) — complete. New
  `LotusType::Array(elem, N)`; fixed-size `[T; N]` lowers to
  arena-allocated `[N x T]` storage. `arr[i]` indexing, `for x
  in arr` iteration, and arrays-as-fn-params all work; element
  type is inferred from the literal's first element. Empty
  array literals + variable-size arrays remain rejected (need
  a type ascription / element-type carrier the literal-only
  path doesn't carry). Per The Design / lotus, the arena's
  wholesale-free shape is the reason arrays are fixed-size in
  v0: dynamic Vec would need either reallocation under a
  separate growth policy or a fundamentally different lifetime
  story. New `examples/21-arrays/` covers indexing, for-loop,
  and arrays as fn parameters.
- **Phase 3 milestone 28c** (pinned CPU-core affinity) —
  complete. `: schedule pinned(core = N)` syntax parses through
  to a `pthread_setaffinity_np` call right after pthread_create.
  ScheduleClass::Pinned grew to `Pinned(Option<i64>)`; the
  parser recognizes optional `(core = N)` after `pinned`; the
  C-runtime helper `lotus_set_core_affinity` wraps the syscall
  behind a stable signature so codegen doesn't have to know the
  cpu_set_t layout. Best-effort: if the requested core doesn't
  exist or the call is denied, the runtime silently falls back
  to ordinary OS scheduling. New `examples/20-pinned-core/`
  pins two workers to cores 0 and 1. Per The Design / lotus,
  this is a refinement WITHIN pinned, not a third mode —
  bimodality holds.
- **Phase 3 milestone 28b** (cross-thread bus mailboxes) —
  complete. Pinned loci can now declare `bus subscribe` and
  publish to cross-thread subjects; the gate is fully lifted
  (only `accept()` and closures remain pinned-incompatible —
  both require cross-thread cascade/violation routing that's
  separate from the mailbox post-and-continue m28b
  delivers). Stage 1 refactored bus queue cells to carry
  inline payloads (with a `pthread_mutex_t`) so the queue is
  the single point of cross-thread synchronization — each
  per-locus arena stays single-threaded territory. Stage 2
  added `lotus_mailbox_t` (a bounded ring buffer with
  mutex+condvar+shutdown flag), grew the bus entry struct
  to `{subject, self, handler, mailbox}`, taught
  `bus_dispatch` to route by `entry.mailbox`
  (null → cooperative global queue; non-null → pinned
  mailbox), and grew the synthesized
  `__pinned_main_<Locus>` body with a mailbox loop between
  `run()` and `drain()`. Coordinated shutdown:
  deferred-dissolve flush calls `lotus_mailbox_shutdown` →
  pthread_join → arena/mailbox destroy. Per The Design /
  lotus, the substrate cost lives at the layer boundary
  (the mailbox lock + the inline payload's two memcpy's),
  not inside either layer's arena. Bimodality holds. New
  `examples/19-pinned-bus/` exercises a cooperative
  publisher feeding a pinned subscriber across threads.
  v0 limit: payloads above 512 bytes drop silently;
  trellis-grade typed messages are well under this.
- **Phase 3 milestone 28a** (pinned full lifecycle on the pinned
  thread) — complete. m27's "run-only" gate is lifted: pinned
  loci can now declare birth / run / drain / dissolve, and the
  full sequence executes on the pinned thread, in order. Codegen
  synthesizes a per-locus `__pinned_main_<LocusName>(self_ptr)
  -> ptr` whose signature matches pthread's start-routine
  contract directly; `pthread_create` gets that function pointer
  with `self_ptr` as its argument. The C-side `lotus_thread_entry`
  adapter and the `(fn, self_ptr)` args struct are gone — the
  generated thread_main calls each declared lifecycle method in
  sequence, returns null. `flush_dissolve_frame` short-circuits
  drain / dissolve for pinned entries (those already ran on the
  pinned thread); main thread's only remaining work is the
  pthread_join + arena_destroy. v0 m28a still gates: pinned
  loci cannot declare `accept()` (cross-thread cascade
  dissolves), bus subscribe / publish (cross-thread mailbox),
  or closures. Those wait on m28b. New
  `examples/18-pinned-lifecycle/` exercises the full lifecycle
  with a 30ms sleep in `run()` so the main thread races past
  before the pinned thread reaches `run`'s body — proves real
  parallelism + correct ordering of all four methods on the
  pinned thread.
- **Phase 3 milestone 27** (pinned threads, run-only) —
  complete. Pinned-class loci spawn a real pthread at
  instantiation: codegen arena-allocates a `(run_fn, self_ptr)`
  tuple, calls `pthread_create` with the C-runtime adapter
  `lotus_thread_entry` as the start routine, and defers
  `pthread_join` to the deferred-dissolve flush via a new
  optional `thread_id_alloca` field on frame entries (parallel
  to cooperative long-lived's None-tagged entries). pthread_join
  blocks until run() returns; arena destroy follows.
  Linker now passes `-lpthread` unconditionally. v0 m27 scope:
  pinned loci can declare ONLY `run()` — no birth/drain/dissolve,
  no bus subscribe/publish. Codegen errors clearly otherwise.
  Full pinned lifecycle on the pinned thread + cross-thread
  bus mailbox (the "any → pinned" post-and-continue side of
  cross-class semantics) wait on m28.
  `examples/16-schedule-classes/` updated to actually exercise
  the new substrate: PinnedWorker.run() does a 50ms
  `time::sleep` so the main thread's println races deterministically.
  Output ordering "cooperative ... / main: spawned both / pinned
  ran on its own pthread" demonstrates the parallelism.
- **Phase 3 milestone 26b** (explicit `yield` primitive) —
  complete. `yield` lifted from reserved keyword to a real
  statement. Codegen lowers `yield;` to a call to
  `lotus_bus_queue_drain` at this point — pending substrate
  cells fire mid-body. Interpreter treats it as a no-op
  (single-threaded synchronous dispatch — no queue to drain).
  Per spec/runtime.md cooperative yield points: "explicit
  `yield` (rare, for long-running computations)" — the
  implicit yield points (handler exit, lifecycle transition,
  bus dispatch) cover most cases; `yield` is the escape hatch
  for long-internal-loop bodies. New `examples/17-yield/`
  exercises the primitive end-to-end.
- **Phase 3 milestone 26** (cooperative scheduler semantics) —
  complete. Bus dispatch is now deferred: each `<-` enqueues
  `(handler, self, payload_copy)` cells onto a program-wide
  FIFO queue (`@lotus.bus_queue.global`) instead of running
  handlers inline. The C-runtime drain loop pops cells one at
  a time and invokes the handler — handler-atomic per substrate
  cell, with cooperative yields between cells rather than
  nested call frames. Handlers may publish more events; drain
  continues until empty. Drain runs at the start of every
  `flush_dissolve_frame` so cooperative subscribers process
  pending cells before they themselves dissolve. v0 limitation:
  cells enqueued during dissolves are leaked (subscriber gone).
  trellis-demo + 05-bus output unchanged from sync-nested days
  — interleaving naturally produces the same observable order
  for these examples (kernel multipliers all 1.0; 05-bus is a
  linear two-stage chain). Spec/runtime.md updated;
  spec-aligned per "cooperative yield points: between handler
  invocations, between lifecycle transitions, on bus dispatch."
- **Phase 3 milestone 25** (schedule-class annotation
  infrastructure, bimodal) — complete. New keywords
  `schedule`, `cooperative`, `pinned` in lexer (no `greedy` —
  see preamble); `LocusAnnotation::Schedule(ScheduleClass)` in
  AST; parser recognizes the `: schedule X` annotation alongside
  `tier N` and `projection X`; typechecker stores it on
  `Annotations`; codegen resolves it onto
  `LocusInfo.schedule_class` (default cooperative). Runtime
  today still runs everything synchronously — no semantic
  branch on the class yet. m26 will introduce deferred bus
  dispatch + a scheduler loop on the main thread; m27 spawns
  dedicated threads for pinned loci.
  `examples/16-schedule-classes/` exercises both classes;
  spec/runtime.md gets a "Schedule classes" section
  documenting the surface, the explicit bimodality reasoning
  ("Why no greedy class"), and the implementation status.
- **Phase 3 milestone 24** (`match` expressions) — complete.
  Match statements lower to LLVM as a chain of test-blocks +
  body-blocks, falling through to the next arm on mismatch.
  Patterns supported: `Literal` (Int / Bool / Duration / Float /
  Decimal), `Wildcard`, and `Binding(x)` (binds the scrutinee to
  `x` for the arm body, with shadow/restore of any prior local
  with the same name). `Tuple` / `Constructor` patterns + arm
  guards remain interpreter-only. F.18 exhaustiveness is
  enforced upstream by the typechecker, so the post-arms
  fallthrough block is unreachable for well-typed programs.
  Match arm bodies handle `Call` exprs by routing through
  `lower_stmt` (so `println` / void-returning user fns work
  identically to statement-position calls). New
  `examples/15-match/` exercises Int + Bool + Binding patterns.
- **Phase 3 milestones 22 + 23** (per-projection-class arena
  strategies) — complete. Each locus's projection class
  resolves from `: projection <class>` annotation or per-spec
  default rule (chunked if accept declared, rich otherwise) at
  declare-locus-struct time. m22 wires chunked parents through
  `lotus_arena_create_subregion`: each accepted child gets a
  sub-region carved from the parent's bookkeeping space, with
  slot indices reused via a free-list when children dissolve.
  m23 lights up the recognition annotation behind the same
  sub-region path — the pre-allocated bitmap-cell pool
  optimization is deliberately deferred until a workload
  exercises it, and that gap is documented in
  `spec/memory.md`. New `examples/14-projection-classes/`
  exercises all three classes end-to-end.
- **Phase 3 milestone 20** (locus-owned arenas + bus copy
  semantics) — complete. Every locus struct now carries a
  synthetic `__arena: ptr` field at struct slot 0; instantiation
  fills it via `lotus_arena_create()`; the per-locus arena is
  wholesale-freed via `lotus_arena_destroy` after `dissolve()`
  runs (both the ephemeral path and the deferred long-lived
  flush). Allocations route through three tiers: an explicit
  override (used during locus-instantiation field init so
  composite-default literals land in the new locus's arena), the
  enclosing locus's arena field (when `current_self` is set), or
  the program-wide arena (`@lotus.arena.global`, used in `main`
  and free fns). Bus dispatch implements the spec's "typed
  message crossing a locus boundary is a copy, not a pointer"
  rule: each `<-` passes the payload's compile-time size to
  `lotus.bus_dispatch`, which allocates `size` bytes in each
  matching subscriber's arena (loaded from `self_ptr + 0`,
  the fixed arena-field offset), memcpy's the payload, and
  passes the COPY to the subscriber's handler. Trellis-demo's
  `self.current_kernel = msg` pattern now actually works under
  per-locus arenas — subscriber's stored copy outlives publisher
  locus dissolution.
- **Phase 3 milestone 19** (region allocator substrate) —
  complete. The codegen path now links a small C arena runtime
  (`crates/lotus-codegen/runtime/lotus_arena.c`, bundled into the
  compiler via `include_str!`) into every emitted binary. ABI:
  `lotus_arena_create()` / `lotus_arena_alloc(arena, size, align)`
  / `lotus_arena_destroy(arena)`. An arena is a linked list of
  bump chunks (default 64 KiB; oversized requests get a fresh
  chunk sized to fit); allocation is pointer-bump in the head
  chunk, destruction walks + frees wholesale.
- **Phase 3 milestone 18** (codegen subset). **17 of 18 example
  projects build to native ELF — every single-binary example.**
  Modes (lowered as locus methods named bulk/harmonic/resolution;
  callable via `self.<mode>()`), built-in `self.children` array
  (fixed-cap 16, embedded after user fields on every locus that
  declares accept; appended at accept dispatch + counter bumped),
  `for child in self.children { ... }` lowered as an indexed
  loop with the var bound as a LocusRef-typed local, and locus
  literals in expression position so `let _l1 = LeafL { ... }`
  works. Interpreter parity: replaced the m10 dedup-pop with a
  `dissolved: Cell<bool>` flag on LocusHandle so ephemeral
  handles stay in parent.children (for `for child in
  self.children`) but the parent's later cascade skips
  already-dissolved children.
- **Phase 3 next** — `trellis-pair` (multi-binary, cross-process
  bus + entry-point selection) is now the only example
  remaining. The substrate is in good shape: full bimodal
  scheduler with cross-thread bus, per-projection-class arenas,
  cooperative deferred dispatch + explicit yield, pinned threads
  with full lifecycle + mailboxes + core affinity. trellis-pair
  needs `lotus build --bin <Locus>` entry-point selection plus a
  cross-process bus transport (decided last session: shared-
  memory ring buffer, per the runtime/stdlib transport split
  documented below). It also exercises pieces still
  interpreter-only: module / `import` resolution, `perspective`
  declarations with `is_stable()`, and tick-epoch closures.

## Transport layering (decided 2026-05-08)

Runtime / stdlib split for bus transports:

- **Runtime owns kernel-level IO primitives** + thin `Transport`
  adapters that wrap them: shared memory (`shm_open` + `mmap` +
  atomic indices), Unix domain sockets (`AF_UNIX`), TCP/UDP
  (`AF_INET` + multicast). Direct syscall plumbing wired into
  the bus router. `io_uring` / `epoll` / `kqueue` integration
  also lives here when the cooperative scheduler lands.
- **Stdlib owns protocols on top of those primitives**:
  `std::bus::nats` (NATS frames over TCP), `std::bus::mqtt`,
  `std::bus::http_sse`, `std::bus::grpc`. TLS lives in
  stdlib too (`std::tls`); serialization (json/protobuf/msgpack)
  in `std::encoding`.

This matches `spec/runtime.md`'s "transport-agnostic" framing —
runtime defines the `Adapter` interface, specific protocols
plug in from stdlib. The new clarification is that the runtime
*also* directly exposes the kernel primitives those protocols
need, rather than forcing every adapter to vendor its own
syscall wrappers.

## Producer/consumer cardinality is emergent (insight, 2026-05-08)

The standard MPSC / SPSC / SPMC / MPMC taxonomy doesn't
describe a transport configuration — it describes
**locus connectivity** on a subject. Count the loci with
`publish "X"` and the loci with `subscribe "X"` at link time:

| Publishers on X | Subscribers on X | Required machinery |
|---|---|---|
| 1 | 1 | SPSC — wait-free, no claim ticket |
| 1 | N | SPMC — Disruptor's natural shape |
| N | 1 | MPSC — fan-in queue with producer claim |
| N | N | MPMC — atomics on both sides |

In trellis-demo all subjects are SPSC / SPMC; **no MPMC
machinery needed**. That's a real speedup vs a uniform
"every subject is MPMC" runtime — SPSC rings are 5-10x faster
than MPMC ones.

The current `BusRouter` doesn't exploit this — it's uniform
MPMC-shaped. When the substrate gets more serious, per-subject
specialization is exactly the kind of optimization the
framework's coordination primitives unlock that a general
pub-sub library can't: **the locus surface carries the shape
information; the substrate gets to specialize.** Connects to
F.14 (three-way interface: locus + parent + contract) — the
contract surface declares data flow shape; bus declarations
declare connectivity shape; together that's enough to pick
the cheapest correct primitive.

## Codegen milestone arc (Phase 3 progress)

Each milestone below is one focused commit + a CHECKPOINT/README
refresh. The arc moved fast: nineteen milestones (m0–m18)
landed across two sessions in 2026-05-08, taking the codegen
path from "no-op stub" to "every single-binary example is a
build target." Each load-bearing piece was intentionally narrow:

```
m0  Phase 3 milestone 0: lotus build → native ELF via LLVM      (77b977f)
m1  Codegen milestone 1: Int / Float / Bool params + println    (5c9b6f7)
m2  Codegen milestone 2: let + Int/Float arithmetic + cmp       (5224d53)
m3  Codegen milestone 3: let mut + assignment                   (03c2f55)
m4  Codegen milestone 4: if / while / break / continue          (cae8c9a)
m5  Codegen milestone 5: time::sleep on CLOCK_MONOTONIC         (929efa2)
m6  Codegen milestone 6: multi-fn programs                      (9955bea)
m7  Codegen milestone 7: locus runtime ABI    ← load-bearing    (206fbd0)
m8  Codegen milestone 8: accept() + parent-child wiring         (d5afffd)
m9  Codegen milestone 9: time::monotonic() + Duration arith     (cdd7353)
m10 Codegen milestone 10: drain() / dissolve() lifecycle        (3ba3e05)
m11 Codegen milestone 11: user `type` decls + struct literals   (5cb4882)
m12 Codegen milestone 12: bus router (subscribe + <- + deferral)(5645eaa)
m13 Codegen milestone 13: self.method() calls                   (b036c7f)
m14 Codegen milestone 14: Decimal + return-from-main exit code  (b036c7f)
m15 Codegen milestone 15: closures (collapse-only path)         (9bf21c1)
m16 Codegen milestone 16: Time + composite defaults + heap lits (e33e8ee)
                          ⇒ trellis-demo builds to native ELF
m17 Codegen milestone 17: on_failure routing (absorb / bubble)  (4bf84e3)
                          ⇒ 03b / 03c build to native ELF
m18 Codegen milestone 18: modes + self.children + for + locus  (d48df6b)
                          literal in expression position
                          ⇒ 04-modes builds; 17/18 single-binary
                            examples are build targets
m19 Codegen milestone 19: region allocator substrate           (ea4892b)
                          ⇒ libc malloc removed; lotus_arena_*
                            backs every type-literal + ClosureViolation
                            allocation; same example ladder still passes
m20 Codegen milestone 20: locus-owned arenas + bus copy        (d511670)
                          ⇒ __arena field on every locus struct
                            (slot 0), lifecycle-bound; bus dispatch
                            copies payloads between publisher /
                            subscriber arenas per spec
m22 Codegen milestone 22: chunked-class sub-regions            (010db7a)
                          ⇒ chunked parents allocate accepted
                            children via lotus_arena_create_subregion;
                            free-list bookkeeping reuses slot
                            indices as children dissolve
m23 Codegen milestone 23: recognition-class stub               (010db7a)
                          ⇒ recognition annotation parses /
                            resolves / dispatches; behaviorally
                            equivalent to chunked at v0; bitmap-
                            pool optimization deliberately deferred
                          + examples/14-projection-classes
m24 Codegen milestone 24: match expressions                    (bb948c6)
                          ⇒ Literal / Wildcard / Binding patterns
                            in codegen; Tuple / Constructor +
                            guards remain interpreter-only;
                            F.18 exhaustiveness still enforced at
                            typecheck
                          + examples/15-match
m25 Codegen milestone 25: schedule-class annotation infra      (bbe2731 +
                                                                763edf8)
                          ⇒ `: schedule cooperative | pinned`
                            parses, typechecks, resolves on
                            LocusInfo; default cooperative; no
                            runtime semantic branch yet.
                            Bimodal-only: greedy dropped on
                            review as a bimodality violation.
                          + examples/16-schedule-classes
m26 Codegen milestone 26: cooperative scheduler semantics      (9c0ba40)
                          ⇒ bus dispatch deferred via process-
                            wide FIFO queue (lotus_bus_queue_*);
                            drain runs at flush_dissolve_frame
                            entry so subscribers process cells
                            before they dissolve; cells enqueued
                            during dissolves are leaked (v0)
m26b Codegen milestone 26b: explicit `yield` primitive         (6760a44)
                          ⇒ yield lifted from reserved to real;
                            codegen lowers to lotus_bus_queue_drain;
                            interpreter no-op
                          + examples/17-yield
m27 Codegen milestone 27: pinned threads (run-only)            (cc57ee4)
                          ⇒ pthread_create at pinned instantiation;
                            run() executes on its own thread;
                            deferred pthread_join at scope exit;
                            -lpthread linked unconditionally;
                            v0 scope: pinned loci must be run-only
                            (no other lifecycle, no bus)
m28a Codegen milestone 28a: pinned full lifecycle              (c70b551)
                          ⇒ pinned loci can declare birth/run/
                            drain/dissolve, all run on the pinned
                            thread in order; synthesized per-locus
                            __pinned_main_<Locus> matches pthread
                            start-routine signature directly (no C
                            adapter, no args struct); flush skips
                            drain/dissolve for pinned entries
                            (already ran on the thread)
                          + examples/18-pinned-lifecycle
m28b/1 m28b stage 1: inline-payload bus queue + mutex          (8f8d20d)
                          ⇒ queue cells carry [u8; 512] inline
                            payload; pthread_mutex_t guards cell
                            array; drain copies inline →
                            subscriber arena before invoking
                            handler. Prereq for cross-thread
                            bus: queue is the single sync point;
                            arenas stay single-threaded.
m28b/2 m28b stage 2: per-pinned mailbox + dispatch routing     (fe296ae)
                          ⇒ lotus_mailbox_t (mutex+condvar+
                            shutdown flag); bus entry grows
                            mailbox field; dispatch routes by
                            entry.mailbox null/non-null;
                            synthesized __pinned_main_<Locus>
                            grows mailbox loop between run()
                            and drain(); coordinated shutdown
                            via shutdown-flag-then-join
                          + examples/19-pinned-bus
m28c   Codegen milestone 28c: pinned(core=N) affinity          (5b10337)
                          ⇒ ScheduleClass::Pinned(Option<i64>);
                            parser optional (core=N); C-side
                            lotus_set_core_affinity wraps
                            pthread_setaffinity_np; codegen
                            calls it after pthread_create when
                            core is set; best-effort fallback
                          + examples/20-pinned-core
m29    m29: match arm guards in codegen                        (0398d42)
                          ⇒ pattern → guard_bb (binding install
                            + guard eval + cond branch) → body;
                            falls through to next arm on false;
                            extends m24 surface
                          + examples/15-match (extended)
m30    m30: arrays — literal + indexing + for-over-array       (2bc3fbb)
                          ⇒ LotusType::Array(elem, N); fixed-
                            size [T; N] only; arena-backed
                            storage; arr[i] indexing; for x in
                            arr lowers to indexed loop; arrays
                            pass through fn params (as ptrs)
                          + examples/21-arrays
m30b   m30 follow-up: indexed local-array assignment           (78ea6e7)
                          ⇒ `arr[i] = v` lowers via GEP-into-
                            local-array-storage + store; rest
                            of LValue surface unchanged
                          + examples/22-moving-average (real
                            flex: bus-driven sliding-window
                            mean over a [Int; 4] state array)
m31    m31: integer ranges in for-loop iterators               (2e7cb06)
                          ⇒ Expr::Range { lo, hi, inclusive }
                            in AST; parser tail-attaches at
                            lowest precedence; for-stmt
                            handlers (interp + codegen) special-
                            case Range as a counted loop; range
                            outside iterator position rejects
                          + examples/23-ranges
m32    m32: default fn param values (free fns)                 (d211c60)
                          ⇒ Defaults must form a suffix; caller
                            may omit trailing args; default expr
                            evaluates at the call site in the
                            caller's scope. Locus methods still
                            reject — m32 is free-fn-only.
                          + examples/24-default-params
m33    m33: import resolution for multi-file projects          (3440a92)
                          ⇒ CLI's parse_with_imports walks the
                            entry's `import "..."` directives,
                            recursively parses each, dedups by
                            canonical path, merges items into
                            one logical Program. Paths resolve
                            relative to importing file's dir
                            with .lt extension implicit. Cycles
                            short-circuit. Both `lotus run` and
                            `lotus build` use the merged Program
                            for single-file targets.
                          + examples/25-imports
m34    m34: default param values on locus `fn` methods         (315ad4d)
                          ⇒ Locus methods called via
                            `self.method(...)` now support
                            default param values (suffix-only
                            rule, fill at call site). Bus-
                            subscribed handlers reject defaults
                            with a clear error (bus dispatch is
                            fixed-arity self+payload). Mode
                            methods still reject — F.10 keeps
                            their param surface tight.
                          + examples/24-default-params (extended)
m33fix m33 fix: skip std/* imports during file resolution      (2358ea3)
                          ⇒ `import "std/time"` declarations in
                            01-locus-with-run + 08-monotonic-sleep
                            now resolve as namespace markers
                            (toolchain handles time::* as
                            built-ins) instead of trying to load
                            on-disk source. Fixed regression.
m35    m35: tuples — values + patterns + destructure          (4e0a19b)
                          ⇒ LotusType::Tuple(Vec<LotusType>);
                            tuple literal `(a, b)` lowers to
                            arena-backed anonymous struct alloc;
                            numeric field access `t.0` / `t.1`
                            via parse_postfix IntLit-after-dot
                            extension; flat let destructure via
                            new Stmt::LetTuple variant; tuple
                            patterns in match arms with sub-
                            patterns Wildcard / Binding / Literal
                            (acc-AND-merged). Empty tuple `()`
                            rejected (no unit-typed surface);
                            nested tuple sub-patterns deferred.
                            Match-arm code refactored to a Vec
                            of bindings; literal-EQ comparison
                            extracted to lower_match_eq_cmp.
                          + examples/26-tuples
m36    m36: string ops — concat, eq, len, slicing             (5f948f8)
                          ⇒ Four C runtime helpers
                            (lotus_str_concat / _eq / _len /
                            _slice); codegen lower_binop adds
                            (Add, String) and (Eq|NotEq,
                            String); Expr::Index Range-receiver
                            routes to slicing with hi+1 for
                            inclusive form; lower_len_builtin
                            handles String (strlen) and Array
                            (compile-time N). Bounds-clamp on
                            slice (no panic). Interpreter
                            mirrors via builtin_len +
                            Range-over-String in Expr::Index.
                          + examples/27-strings
m37    m37: to_string(x) primitive → String                   (ccbaec8)
                          ⇒ Three snprintf-backed runtime
                            helpers (lotus_str_from_int /
                            _float / _duration); Bool dispatches
                            via build_select between "true" /
                            "false" globals; String passes
                            through. Output matches println
                            formatting (%lld / %g / %lldns).
                            Interpreter uses fmt_decimal_pub
                            so Float / Decimal output matches
                            codegen's %g semantics.
                          + examples/28-to-string
m38    m38: stdlib helpers — math + string predicates         (5787acd)
                          ⇒ min(a, b) / max(a, b) / abs(x)
                            across Int / Duration (signed
                            integer compare + select) and
                            Float / Decimal (float compare +
                            select). starts_with / contains
                            over String → Bool, backed by C
                            strncmp / strstr wrappers. Empty
                            prefix / sub matches anything
                            (Rust semantics). Interpreter
                            mirrors via parse_decimal_pub
                            alias for Decimal compare.
                          + examples/29-helpers
                          + examples/30-stats (bonus —
                            Producer + Aggregator combining
                            m35 → m38 surface)
m39    m39: birth-epoch closures (substrate F.9 deepening)    (cba1e96)
                          ⇒ Closures gain a per-entry
                            EpochSpec tag; closures_fn splits
                            into birth_closures_fn +
                            dissolve_closures_fn. Lifecycle
                            dispatch sequences birth() →
                            __birth_closures → run(); existing
                            drain → __dissolve_closures →
                            dissolve still runs at end.
                            Default closures stay dissolve-
                            only (purely additive).
                            Tick/Duration/Explicit still
                            reject. Interpreter mirrors via
                            closure_fires_at_birth + birth-
                            eval block in instantiate_locus.
                          + examples/31-birth-closures
m40    m40: restart recovery primitive (F.9 response half)    (eab0f96)
                          ⇒ Synthetic __restart_count i64
                            field on every locus struct, zero-
                            init at instantiation. restart(c)
                            bumps it via GEP+load+add+store.
                            lower_closure_check extended with
                            epoch param: birth-epoch captures
                            pre/post counts around on_failure
                            call, branches to rerun_bb on
                            bumped-within-cap (2). rerun_bb
                            calls birth() + recursive
                            __birth_closures + ret void.
                            Interpreter mirrors via
                            LocusHandle.restart_count +
                            depth-bounded loop in
                            instantiate_locus.
                          + examples/32-restart
m41    m41: quarantine recovery primitive (sticky flag)       (ff525fe)
                          ⇒ Synthetic __quarantined i64 flag
                            after __restart_count, zero-init.
                            quarantine(c) writes 1 via
                            GEP+store; run() gated by load+
                            icmp+cond branch around the call.
                            Drain / dissolve still fire.
                            Bus-dispatch gating deferred to
                            m41b (needs fixed-offset C-runtime
                            load). Interpreter mirrors via
                            LocusHandle.quarantined +
                            RecoveryOp::Quarantine + skip on
                            Run lifecycle.
                          + examples/33-quarantine
m41b   m41b: bus-dispatch quarantine gating                   (cbf23cc)
                          ⇒ Bus dispatch null-checks
                            entry.subject before strcmp.
                            quarantine(c) walks bus.entries
                            and nulls subject of matching
                            self entries. Interpreter mirrors
                            via dispatch_bus skipping
                            quarantined locus. + LOTUS_DUMP_IR
                            env var for codegen debugging.
                          + examples/34-quarantine-bus
m45    m45: restart_in_place recovery primitive             (01f4e69)
                          ⇒ Variant of m40's restart(c)
                            that zeros user fields back to
                            declared defaults BEFORE re-
                            running birth(). Shared cap-2
                            budget on the same retry counter.
                            Synthetic __restart_in_place_
                            pending flag distinguishes the
                            two re-run kinds. New
                            lower_restart_in_place_call
                            shares lower_restart_call_kind
                            body. Rerun branch in
                            __birth_closures gates on the
                            flag and re-stores defaults
                            before call_birth. Interpreter
                            mirrors via LocusHandle.
                            restart_in_place_pending +
                            top-of-loop zero pass.
                          + examples/38-restart-in-place
m45fix m45 follow-up: bus.entries cap × 32                  (fc72504)
                          ⇒ Quick fix for a pre-existing
                            m41b limit: bus.entries was
                            sized to one slot per declared
                            subscription, so multiple
                            instances of the same locus type
                            overflowed at runtime. Multiplier
                            bumps capacity to 32 instances/
                            type. Proper fix (C-runtime
                            dynamic vec) deferred to future
                            polish.
                          + examples/39-multi-instance-bus
m45fix2 m45 follow-up #2: bus router → C-runtime dynamic vec
                          ⇒ Bus storage migrated out of LLVM
                            entirely. New C runtime fns
                            lotus_bus_register / _dispatch /
                            _quarantine_self / _router_destroy
                            back a heap-grown vec (init 16,
                            doubles). LLVM-side state shrinks
                            to a presence marker; the hand-
                            rolled dispatch fn body is gone;
                            emit_bus_register and the
                            quarantine entries-walk are each
                            one C-runtime call now. lower_send
                            calls lotus_bus_dispatch(queue,
                            subject, payload, size). No
                            compile-time capacity ceiling.
                            Net: codegen.rs −450, runtime +95.
m43fix m43 follow-up: pinned post-run duration wrapper
                          ⇒ Closes m43 v0 limit. New
                            duration_wrapper_fn on LocusInfo
                            mirrors tick_wrapper_fn (1-arg
                            adapter loading __parent_self /
                            __parent_on_failure from struct,
                            tail-calls 3-arg
                            __duration_closures). Synthesized
                            for every locus with at least
                            one duration closure. Called from
                            __pinned_main_<Locus> after run()
                            alongside the tick wrapper.
                            Wrapper bodies for tick + duration
                            generated by a shared 2-iteration
                            loop in pass C.
                          + examples/40-pinned-duration
m46    m46: closure accumulators (sum streaming-fold)
                          ⇒ `sum(self.X)` inside a closure
                            assertion accumulates self.X across
                            every epoch fire; the assertion
                            reads the running total. Per
                            `sum(...)` detected: one struct
                            field (Int/Float/Decimal/Duration),
                            re-evaluated + added to slot at
                            each fire BEFORE the assertion runs.
                            `persists_through(...)` opts out of
                            recovery-zero (default = reset on
                            restart/restart_in_place/quarantine).
                            Parser extended to accept recovery-
                            event keyword tokens as bare names.
                            Vocabulary v0.1: sum only; inner
                            expr restricted to `self.X` reads.
                            Interpreter parity via
                            LocusHandle.accumulators (lazy-init
                            on first sample) +
                            Interpreter::accumulator_ctx for
                            Expr::Sum substitution in
                            evaluate_closure.
                          + examples/41-closure-accumulator
m43    m43: duration-epoch closures (per-N-monotonic gate)   (c115829)
                          ⇒ EpochSpec::Duration lowers
                            alongside Birth + Dissolve + Tick.
                            Per-locus struct gains one i64
                            __duration_last_fire_<i> field per
                            duration closure (declaration
                            order). Synthetic
                            __duration_closures(self, parent,
                            on_failure) fn loads each last,
                            calls clock_gettime(MONOTONIC) for
                            now, evaluates the duration
                            expression in self-scope (so
                            `duration(self.poll_interval)`
                            works), compares elapsed >= N. On
                            fire: store now -> last_fire
                            BEFORE assertion runs so an
                            absorbed violation doesn't reset
                            the interval. Instantiation
                            seeds each field to monotonic-
                            now. Same call sites as tick
                            (post-handler, post-run). v0
                            limit: pinned-thread post-run
                            doesn't fire duration; cooperative
                            + bus paths do. Interpreter via
                            LocusHandle.duration_last_fire +
                            closure_fires_at_duration +
                            fire_duration_closures.
                          + examples/36-duration-closures
m44    m44: explicit-epoch closures (user-triggered)         (b4512df)
                          ⇒ Closes the closure-epoch
                            lowering arc — all five epochs
                            Birth + Dissolve + Tick + Duration
                            + Explicit now lower. EpochSpec::
                            Explicit fires only when the user
                            calls `check_closures();` from
                            inside the locus's body. Synthetic
                            __explicit_closures fn lowered
                            through the same per-epoch loop.
                            New lower_check_closures_call
                            helper detects the builtin in
                            Stmt::Expr(Call(Ident("check_
                            closures"))) and emits a direct
                            call reading parent /
                            on_failure from the m42
                            __parent_self / __parent_on_failure
                            struct fields. Silent no-op when
                            the locus has no explicit
                            closures. Interpreter:
                            LocusHandle.parent field captured
                            at instantiation;
                            Expr::Call intercepts ident
                            "check_closures" before normal
                            callee evaluation and fires
                            fire_explicit_closures(handle,
                            handle.parent). Skipped on
                            quarantined loci.
                          + examples/37-explicit-closures
m42    m42: tick-epoch closures (steady-state pulse)         (1539dff)
                          ⇒ EpochSpec::Tick lowers alongside
                            Birth + Dissolve. Synthetic
                            __tick_closures(self, parent,
                            on_failure) fn + 1-arg
                            __tick_closures_wrapper adapter
                            for the pinned post-run path.
                            Two new ptr fields on every locus
                            struct (__parent_self +
                            __parent_on_failure) baked at
                            instantiation via the same
                            resolve_failure_route the
                            birth/dissolve epochs use. Tick
                            call inlined into subscribed
                            user-fn bodies BEFORE their m26
                            tail bus_queue_drain (thunk-
                            wrapper draft broke ordering
                            because the handler's own tail
                            drain recursively pulled queued
                            cells first). Plus a
                            __quarantined entry gate on
                            subscribed handlers — closes a
                            pre-existing m41b gap where
                            cells enqueued before
                            quarantine() still fired their
                            handler. Interpreter mirrors via
                            Subscription.parent field +
                            closure_fires_at_tick predicate
                            + fire_tick_closures helper
                            called after each run_handler
                            and after run().
                          + examples/35-tick-closures
```

The architectural pivots are **m7** (locus → LLVM struct,
lifecycle methods take `self_ptr`, `self.X` via GEP) and **m8**
(accept's child param as `LotusType::LocusRef(String)`,
parent-aware child instantiation, F.7 dispatch ordering).
Everything before m7 was scalar-only fn-bodies; everything after
m7 builds on the struct ABI.

## What runs vs. what builds

| Primitive | Interpreter | Codegen |
|---|---|---|
| `fn main()` entry | ✅ | ✅ |
| Int / Float / Bool / String literals + params | ✅ | ✅ |
| `let` bindings | ✅ | ✅ |
| Arithmetic, comparisons, logical ops | ✅ | ✅ |
| `self.X` reads (in lifecycle methods) | ✅ | ✅ (runtime GEP+load) |
| Locus instantiation + `birth()` | ✅ | ✅ (ephemeral only) |
| Mixed-type println (single printf) | ✅ | ✅ |
| `let mut` + assignment (incl. compound `+=` etc.) | ✅ | ✅ |
| `if` / `else` / `else if` / `while` + `break` / `continue` | ✅ | ✅ |
| `time::sleep` on CLOCK_MONOTONIC + EINTR retry | ✅ | ✅ |
| `time::monotonic()` + Duration ± Duration / cmp | ✅ | ✅ |
| User-defined fns called from main / each other | ✅ | ✅ |
| `run()` lifecycle method | ✅ | ✅ |
| `self.X = ...` mutation in lifecycle methods | ✅ | ✅ |
| `accept()` lifecycle method (F.7 ordering) + child `g.X` reads | ✅ | ✅ |
| `drain()` / `dissolve()` lifecycle methods (F.4 cascade) | ✅ | ✅ |
| User `type` decls + struct literals + field reads | ✅ | ✅ |
| Locus `fn` members (called from bus dispatch, etc.) | ✅ | ✅ |
| Bus router (`<-` send + subscribe dispatch) | ✅ | ✅ |
| Long-lived locus deferred drain/dissolve (subscribers) | ✅ | ✅ |
| `self.method()` calls inside lifecycle / fn bodies | ✅ | ✅ |
| `Decimal` type + arithmetic + comparisons (f64 v0) | ✅ | ✅ |
| `return n;` from main → process exit code | ✅ | ✅ |
| Closures: collapse on pass, exit-non-zero on fail | ✅ | ✅ |
| Closures: parent absorb / bubble routing (F.9) | ✅ | ✅ |
| Built-in `ClosureViolation` type (locus/closure/diff fields) | ✅ | ✅ |
| Modes (`mode bulk()` etc.) + self-method dispatch | ✅ | ✅ |
| `self.children` (fixed-cap array on accept-declaring loci) | ✅ | ✅ |
| `for child in self.children { ... }` iteration | ✅ | ✅ |
| Locus literals in expression position (`let l = L { }`) | ✅ | ✅ |
| Time literals + Time as a typechecked primitive | ✅ | ✅ (string-spelling v0) |
| Composite locus param defaults | ✅ | ✅ |
| Nested field reads (self.x.y, expr-receiver-of-Field) | ✅ | ✅ |
| Heap-allocated user-type literals (escape via bus) | ✅ | ✅ |
| Contracts (typecheck only — F.8) | ✅ | ✅ (skipped at codegen) |
| `match` (Literal / Wildcard / Binding patterns) | ✅ | ✅ |
| `match` arm guards (`pat if cond -> body`) | ✅ | ✅ |
| `match` (Constructor patterns — enum variants) | ✅ | — |
| Array literals `[T; N]` + indexing | ✅ | ✅ |
| `for x in arr` over fixed-size arrays | ✅ | ✅ |
| Indexed local-array assignment `arr[i] = v` | ✅ | ✅ |
| `for i in lo..hi` / `lo..=hi` range loops | ✅ | ✅ |
| Default fn param values (free fns; suffix-only rule) | ✅ | ✅ |
| Default values on locus methods (non-bus, non-mode) | ✅ | ✅ |
| `import "..."` resolution (multi-file projects) | ✅ | ✅ |
| Tuple literals `(a, b)` + tuple types `(T1, T2)` | ✅ | ✅ |
| Tuple numeric field access (`t.0`, `t.1`) | ✅ | ✅ |
| Flat `let (a, b) = pair;` destructure | ✅ | ✅ |
| Tuple patterns in match (Wildcard / Binding / Literal subs) | ✅ | ✅ |
| String concat (`+`), equality (`==`/`!=`) | ✅ | ✅ |
| `len(s)` / `len(arr)` builtin | ✅ | ✅ |
| String slicing `s[lo..hi]` / `s[lo..=hi]` (bounds-clamped) | ✅ | ✅ |
| `to_string(x)` for primitives → String | ✅ | ✅ |
| `min(a, b)` / `max(a, b)` / `abs(x)` for numeric types | ✅ | ✅ |
| `starts_with(s, p)` / `contains(s, sub)` for String | ✅ | ✅ |
| Birth-epoch closures (F.9 invariants checked after `birth()`) | ✅ | ✅ |
| Tick-epoch closures (fire after each handler + run() return) | ✅ | ✅ |
| Duration-epoch closures (fire when N monotonic elapsed) | ✅ | ✅ |
| Explicit-epoch closures (fire on `check_closures();`) | ✅ | ✅ |
| Closure accumulators (`sum(self.X)` streaming fold) | ✅ | ✅ |
| Accumulator `persists_through(...)` recovery gating | ✅ | ✅ |
| `restart(child)` recovery (cap-2 birth re-run) | ✅ | ✅ |
| `restart_in_place(child)` recovery (factory-reset re-run) | ✅ | ✅ |
| `quarantine(child)` recovery (sticky flag, gates run + bus) | ✅ | ✅ |
| Schedule-class annotation (`: schedule cooperative \| pinned`) | — | ✅ (resolved on LocusInfo) |
| Cooperative scheduler (deferred bus + drain loop) | — | ✅ |
| Explicit `yield` primitive | ✅ (no-op) | ✅ (drains queue) |
| Pinned threads (full lifecycle: birth/run/drain/dissolve) | — | ✅ |
| Pinned + cross-thread bus mailbox | — | ✅ |
| Region allocator — per-locus arenas, bus copy semantics | — | ✅ |
| Region allocator — chunked sub-regions + free-list | — | ✅ |
| Region allocator — recognition bitmap-pool | — | — (chunked-equivalent stub) |
| Recovery primitives (bubble) | ✅ | ✅ |
| Recovery primitive (`reorganize`) | parsed | — |

## Locked design commitments (F.1–F.18)

Spec source: `spec/design-rationale.md`. Summary:

- **F.1** k_max = B / [(1−φ)c + φσ] is the framework equation.
- **F.2** `ProjectionClass` as built-in any-of-three constraint.
- **F.3** Per-arena defrag/free-list, no whole-program GC.
- **F.4** `drain()` always cascades depth-first.
- **F.5** Mode projections share the locus's arena.
- **F.6** Lifecycle methods are not implicit loci.
- **F.7** `accept()` runs before child birth.
- **F.8** Contract compatibility type-checked across coordinator /
  coordinatee.
- **F.9** Collapse vs. explosion + parent on_failure routing
  (absorb / bubble).
- **F.10** Mode keywords accepted post-dot as member names.
- **F.11** `self.children` typing and lifecycle.
- **F.12** Bus send is `<-`; subscribe is declarative.
- **F.13** Bus subscription handler signature.
- **F.14** Three-way interface: locus + parent + contract.
- **F.15** Predefined type names are PascalCase, not keywords.
- **F.16** `self.k_max` as built-in computed field (F.1 executable).
- **F.17** Strict field-access; method types on locus / perspective.
- **F.18** Match exhaustiveness checked at typecheck.

## Files to read for orientation

In order:

1. `README.md` — overview, status, F-table, example list, toolchain.
2. `spec/design-rationale.md` — why each construct is shaped the way
   it is. Source of truth for F.1–F.18.
3. `spec/grammar.ebnf` — formal grammar.
4. `spec/tokens.md` — lexical structure.
5. `spec/precedence.md` — operator precedence table.
6. `spec/memory.md` — memory model + the "Codegen ABI (v0)" section
   documenting the locus struct lowering, F.7 dispatch ordering,
   and ephemeral-only constraint (added in m7, extended in m8).
7. `spec/runtime.md` — runtime semantics + the "Time" section
   documenting the monotonic-only-scheduling discipline (m5, m9).
8. `examples/hello-world/main.lt` → `examples/10-stateful-locus/`
   → `examples/trellis-demo/main.lt` — the example ladder.
   06-10 are the codegen-arc demos; trellis-demo exercises the
   full interpreter pipeline.
9. `crates/lotus-syntax/src/lib.rs` — public API of the parser/AST.
10. `crates/lotus-types/src/lib.rs` — typechecker entry + unit
    tests that lock the F.x rules.
11. `crates/lotus-runtime/src/lib.rs` + `eval.rs` + `bus.rs` +
    `builtins.rs` — interpreter, dissolve cascade, bus router,
    `time::sleep` / `time::monotonic` via libc::clock_*.
12. `crates/lotus-codegen/src/codegen.rs` — current LLVM lowering.
    The biggest single file in the workspace; the locus runtime
    ABI is what makes it interesting. Worth a careful read if
    extending codegen.
13. `crates/lotus-codegen/runtime/lotus_arena.c` — the lotus
    region allocator (m19) AND cooperative scheduler queue (m26)
    AND pthread adapter (m27). Bundled into the compiler via
    `include_str!`, written next to each generated `.o` file at
    link time, compiled + linked into the final binary. The
    surface every `arena_alloc` / `bus_queue_*` /
    `lotus_thread_entry` call site in codegen.rs targets.
14. `crates/lotus-cli/src/main.rs` — CLI dispatch (lex / parse /
    check / run / build).
15. `~/.claude/plans/witty-foraging-lightning.md` — the original
    delivery plan to team-wide internal v1.0 (~18–30 months total).
16. `notes/open-questions.md` — tracked deferrals, including the
    spec-vs-impl gap on immutable-binding compile-time
    enforcement (§23).

For broader program context:

- `~/notes/alpha-conjecture/CLAUDE.md` — the master project guide.
  Lotus is one substrate-arm among several; paper 4 is the program's
  foundational anchor (read its memory file too).
- `~/notes/alpha-conjecture/lotus/` — the design-time meta-framework
  that lotus-the-language is the compile-time projection of.

## Strategic preferences locked in

These are user (Riley) directions saved into auto-memory at
`~/.claude/projects/-home-riley-notes-alpha-conjecture/memory/`:

- **Greenfield cleanup as we go** — pre-ship code is greenfield;
  drop "preserved old behavior" / fallback patterns; clean up
  rather than accumulate compatibility cruft. (See
  `feedback_greenfield_cleanup.md`.)
- **Stay focused on lotus** for the foreseeable session — don't
  swing back to paper-4 / theory work without explicit redirect.
- **LLVM is the codegen target** — committed; toolchain installed
  (llvm-18 + clang + lld + libpolly-18-dev). inkwell 0.5 +
  llvm-sys 180.0.0 against system LLVM.
- **Trellis informs but doesn't dictate** — production trellis-pair
  (analyst/executor as separate binaries) is the eventual real-world
  use case, but we're not building specifically toward it. It's a
  milestone we'll hit when the pieces are right; for now,
  `examples/trellis-demo/` is the single-process surrogate that
  exercises the full pipeline.

## User context (Riley)

Junior partner at small finance firm. Deep software-architecture
expertise via brain3 (production deployment at the firm,
brained.dev). The trellis trading system is the natural first
real-world use case for lotus.

## Recent commit history (newest first)

```
51653bd Bus router: deregister subscribers on locus dissolve
1fde35c CHECKPOINT.md: ahead-count 16 + recent-commits refresh post-0ff7794
0ff7794 Substrate session: bus router rewrite + pinned-duration + sum accumulators
47f9792 CHECKPOINT.md: correct ahead-count to 15
8c45f8c CHECKPOINT.md: m45 + bus-cap follow-up refresh
fc72504 m45 follow-up: bus.entries cap × 32 for multi-instance subs
01f4e69 m45: restart_in_place recovery primitive
d4367c1 CHECKPOINT.md: m43 + m44 closure-epoch refresh
b4512df m44: explicit-epoch closures (substrate)
c115829 m43: duration-epoch closures (substrate)
5a2c93c CHECKPOINT.md: m42 tick-epoch closures refresh
1539dff m42: tick-epoch closures (substrate)
a35c128 CHECKPOINT.md: correct ahead-count post-push
bc5f702 CHECKPOINT.md: rewrite Next-Steps for post-m41b state
9da460b CHECKPOINT.md: m41b bus-dispatch quarantine gating refresh
cbf23cc m41b: bus-dispatch quarantine gating
366baee CHECKPOINT.md: m41 quarantine refresh
ff525fe m41: quarantine recovery primitive (sticky-flag F.9)
0128f56 CHECKPOINT.md: m40 restart recovery refresh
eab0f96 m40: restart recovery primitive (F.9 response half)
eada334 CHECKPOINT.md: m39 birth-epoch closures refresh
cba1e96 m39: birth-epoch closures (substrate F.9 deepening)
c1184dc CHECKPOINT.md: m38 + bus aggregator example refresh
f90c8b4 examples/30-stats: bus aggregator combining recent surface
5787acd m38: stdlib helpers — min / max / abs / starts_with / contains
21ac4a2 CHECKPOINT.md: m36 + m37 string ops refresh
ccbaec8 m37: to_string(x) primitive → String
5f948f8 m36: string ops — concat, equality, len, slicing
faa231e CHECKPOINT.md: m35 tuples refresh
4e0a19b m35: tuples — values + patterns + destructure
1a01f40 CHECKPOINT.md: align ahead-count with new origin/master tip
2358ea3 m33 fix: skip std/* imports during file resolution
315ad4d m34: default param values on locus fn methods
3440a92 m33: import resolution for multi-file projects
d211c60 m32: default fn param values (free fns)
2e7cb06 m31: integer ranges in for-loop iterators
78ea6e7 m30 follow-up: indexed local-array assignment + moving-average flex
2bc3fbb m30: arrays — fixed-size literal + indexing + for-over-array
a5fc8bd CHECKPOINT.md: 83 commits ahead post-decimal-parity polish
0c03c79 interpreter: align Decimal arithmetic output with codegen's %g
325a2e8 CHECKPOINT.md: bump ahead-of-origin count to 81
7308f89 CHECKPOINT.md: reflect m28a→m28c→m29 session state
c4ec399 codegen: remove dead `_ =>` arm in lower_stmt
0398d42 m29: match arm guards in codegen
5b10337 Codegen milestone 28c: pinned(core=N) CPU-core affinity
fe296ae m28b stage 2: cross-thread bus mailboxes for pinned loci
8f8d20d m28b stage 1: inline-payload bus queue + mutex
c70b551 Codegen milestone 28a: pinned full lifecycle on the pinned thread
1cb4aaa CHECKPOINT.md: session-resume reference
cc57ee4 Codegen milestone 27: pinned threads (run-only)
6760a44 m26b: explicit `yield` primitive
9c0ba40 Codegen milestone 26: cooperative scheduler semantics
763edf8 m25 cleanup: drop greedy from schedule classes (bimodality)
bbe2731 Codegen milestone 25: schedule-class annotation infrastructure
bb948c6 Codegen milestone 24: match expressions
010db7a Codegen milestones 22 + 23: per-projection-class arena strategies
d511670 Codegen milestone 20: locus-owned arenas + bus copy semantics
ea4892b Codegen milestone 19: region allocator substrate
79e839c CHECKPOINT.md: capture transport layering + cardinality insight
b18febb CHECKPOINT.md: update milestone-arc preamble
601c0b7 CHECKPOINT.md: backfill m18 commit hash
d48df6b Codegen milestone 18: modes + self.children + for-loops
4bf84e3 Codegen milestone 17: on_failure routing (absorb / bubble)
e33e8ee Codegen milestone 16: trellis-demo builds to native ELF
9bf21c1 Codegen milestone 15: closures (collapse-only path)
b036c7f Codegen milestones 13 + 14: self.method, Decimal, return-from-main
5645eaa Codegen milestone 12: bus router lowering
5cb4882 Codegen milestone 11: user `type` decls + struct literals
3ba3e05 Codegen milestone 10: drain() / dissolve() lifecycle
cdd7353 Codegen milestone 9: time::monotonic() + Duration arithmetic
d5afffd Codegen milestone 8: accept() lifecycle + parent-child wiring
206fbd0 Codegen milestone 7: locus runtime ABI
9955bea Codegen milestone 6: multi-fn programs
929efa2 Codegen milestone 5: time::sleep on CLOCK_MONOTONIC
```

17 commits ahead of origin/master at checkpoint time. This
session bundled m45-followup-2 bus router proper-fix +
m43-followup pinned-duration wrapper + m46 closure accumulators
into one commit (0ff7794), then a small follow-up
(51653bd) hardened bus dispatch by deregistering subscribers
on locus dissolve. The F.9
substrate stays closure-epoch-complete (all 5 epochs: Birth +
Dissolve + Tick + Duration + Explicit) on both cooperative AND
pinned dispatch paths AND has both restart variants AND has the
streaming-fold accumulator half (sum-only at v0.1). The
locus-of-design substrate covers the F.9 invariant-and-repair
pair end-to-end: detect-at-any-epoch (5 epochs × snapshot
or running-total), route-via-on_failure, respond-via-restart-
or-quarantine. Bus storage no longer has a compile-time
capacity ceiling — the m45 quickfix `× 32` multiplier is gone.
**54 of 55 examples build to native ELF.**

## Next steps in priority order

### RESUME HERE (next session)

**Cross-process bus substrate arc is closed.** m57 (kernel
transport) + m58 (publisher fanout) + m59 (subscriber reader
thread) + m60 (serializer shape) shipped the substrate for
cross-process coordination per
notes/open-questions #8/#9/#10. Trellis-pair is a v1
acceptance test, not a development driver — per the user's
hard rule, **no code towards trellis-pair until v1 language
is done.** Multi-binary `lotus build --manifest` is post-v1
deployment tooling, not language, and waits with it.

**Next: close the v1 language gaps surface-up.** The CHECKPOINT
already names "Generics" as the biggest visible 'feels v1'
surface gap; it's the natural next arc.

**Start with m61: generics — first session.** Design is locked
(m56): compile-time monomorphization, `ProjectionClass`
bounds, plus `Numeric` bound for v1 (notes/open-questions #2).
Sub-milestones (each its own session, ~4-5 sessions total):

- **m61 generic struct + enum monomorphization** — `type Box<T>
  { value: T }`, `type Result<T, E> = enum { Ok(T), Err(E) }`.
  Codegen instantiates one struct/enum per used type
  combination. Wire up the typechecker's existing generic
  bookkeeping (lotus-types already has the design).
- **m62 generic free fns** — `fn first<T>(xs: [T; 4]) -> T`.
  Per-callsite-type-tuple monomorphization.
- **m63 generic loci** — same shape lifted to locus
  declarations.
- **m64 `Numeric` bound + generic closures** — `T: Numeric`
  bound permits arithmetic on generic params; closures over
  generic loci.
- **m65 stdlib `Result<T,E>` / `Option<T>`** — the canonical
  payoff types that motivate generics in the first place; ship
  as built-ins.

After generics, scan for any remaining v1 punch-list items:
- Multi-peer fanout per subject (m58 hardcodes one peer)
- String fields in cross-process bus payloads (m60's identity
  serializer can't follow String pointers across process
  boundaries; would need real wire format)
- The handful of workload-pending items (count() / mean() /
  rolling-window accumulators, reorganize impl, recognition-
  class real bitmap pool)

Once those gaps that v1 actually *requires* are closed —
**call v1**, then trellis-pair / multi-binary build /
release artifacts come next, in that order.

The cross-process bus arc was chosen as a multi-session
commitment per The Design's delivery-lotus framing: it's the
substrate-root layer of the v1 trajectory, the runtime/stdlib
split is locked (m56), and the design questions for #8/#9/#10
are resolved. Memory note holds (and the user re-affirmed
2026-05-09): trellis-pair is the acceptance test, not the
development driver — every substrate milestone in this arc
was substrate-justified, not trellis-justified, and the arc
closed at m60 without compiling trellis-pair, which is the
right shape. Multi-binary build + trellis-pair sit behind v1
done; the user's hard rule is "no code towards trellis until
v1 language is done."

### Cross-process bus arc (substrate, COMPLETE at m60)

- **m57 AF_UNIX transport in C runtime — DONE** —
  `lotus_transport_*` fns over SOCK_SEQPACKET, raw bytes, no
  protocol layer. Verified via `crates/lotus-codegen/tests/
  transport.rs` (driver + runtime linked into one binary,
  exec'd twice to round-trip a message between processes).
- **m58 Deployment-config subject binding — DONE** —
  `LOTUS_BUS_CONFIG=<path>` parsed at boot via
  `lotus_bus_load_config`; `subject=url:role` lines map each
  subject to a transport. CONNECT-role transports fan out at
  publish time via `lotus_bus_remote_fanout` after the local
  loop. Verified via `crates/lotus-codegen/tests/bus_config.rs`
  (lotus publisher → unix socket → m57 driver listener,
  byte-exact round-trip + local-also-fired + no-config
  regression check).
- **m59 Subscriber-side reader thread — DONE** —
  LISTEN-role binding spawns a per-subject pthread that opens
  the transport on its own stack (so accept() doesn't block
  main's boot), loops `lotus_transport_recv`, and dispatches
  via the new `lotus_bus_local_dispatch` helper. Thread joins
  at `lotus_bus_remote_destroy_all`. Verified via
  `crates/lotus-codegen/tests/bus_subscriber.rs` (two-lotus-
  binary end-to-end: publisher → unix socket → subscriber's
  reader thread → cooperative queue → Sub.on_evt → stdout).
- **m60 Per-payload serializer shape — DONE (wire format
  deferred)** — codegen synthesizes
  `__serialize_<T>` / `__deserialize_<T>` per bus payload
  type and routes every send/recv through them; bodies are
  identity at v0.1. `lotus_bus_register` extended with a
  deserialize_fn ptr arg; reader thread looks it up by
  subject and decodes wire bytes before local dispatch. The
  shape unblocks a future wire-format milestone (field-by-
  field little-endian + length-prefixed Strings + schema
  versioning) as a body-only change. Verified via
  `crates/lotus-codegen/tests/serializer_shape.rs` (IR
  contains expected `define i64 @__serialize_T` /
  `@__deserialize_T` symbols) plus continued passing of
  `bus_subscriber.rs` (operational proof that identity bodies
  preserve byte semantics through the pipeline).
**Substrate complete at m60.** Multi-binary build orchestration
and trellis-pair end-to-end are POST-v1 — they're the v1
acceptance test artifact and the deployment tooling around it,
not language work. They sit behind whatever closes v1
(generics + any v1-required substrate gaps); see RESUME HERE.

### Generics arc (~4-5 sessions, current commitment)

The biggest "feels v1" surface gap. Design locked (m56):
compile-time monomorphization, `ProjectionClass` bounds,
`Numeric` bound for v1 (notes/open-questions #2). Sub-
milestones m61-m65 expanded in RESUME HERE above. After this
arc, scan v1 punch-list for any remaining gaps the substrate
exposes (multi-peer fanout, Strings cross-process, etc.) and
close those before calling v1 done.

### Single-session implementations (workload-pending)

- **`reorganize` impl.** Semantic locked m56: `restart_in_place`
  lifted to substructure level — parent's params reset,
  children re-attach, no lateral migration. Defer until a
  workload exercises it (no example currently does).
- **`count()` / `mean(x)` accumulator vocabulary.** Both build
  on m46 slot machinery; ~80 lines codegen. Workload-pending.
- **Rolling-window accumulators.** Fixed-cap storage decision
  that interacts with arena lifetime; deeper than count/mean.
  Workload-pending.
- **Recognition-class real bitmap-pool.** Currently chunked-
  equivalent stub per spec/memory.md. Surface contract is
  exercised; only the optimization is missing. Workload-
  pending.

### Polish

- **Decimal precision tightening** (printf %g vs Display) —
  m48 trims trailing zeros in `lotus_decimal_to_string`;
  remaining gap (if any) is workload-driven.
- **Constructor patterns in `match`** for enum variants —
  shipped in m47 + m47-payloads. The CHECKPOINT polish-list
  entry from earlier sessions referred to a struct-by-name
  shape that no example exercised; resolved.

### Tooling (post-substrate-v1)

- Tree-sitter grammar derivation from EBNF (~2-4h)
- LSP server (~8-12h, the big tooling commitment)
- Stdlib growth (workload-pending — collections, time/decimal
  helpers, bus-transport adapters)

### Long-deferred (post-v1)

- Self-hosting (Phase 6, distant)
- Multi-implementation contract surface (deferred to v0.5+)
- Perspective `serialize_as TypeV1` versioning protocol
  (open-question #13)

## Toolchain state

System has:

- `llvm-config` 18.1.3 at `/usr/bin/llvm-config`
- `clang` 18.1.3 at `/usr/bin/clang`
- `lld` at `/usr/bin/lld`
- `libpolly-18-dev` (required by llvm-sys for static link)
- `gcc` 13.x

Cargo workspace builds clean. `cargo test --workspace --tests` passes
all 102 tests (the locus-with-run test runs 3×500ms sleeps so the
runtime + codegen integration buckets clock ~1.5s each; m57 added
two transport round-trip tests under tests/transport.rs that fork
listener + connector subprocesses; m58 added two more under
tests/bus_config.rs that route a lotus publisher's bus dispatch
through a unix socket to the m57 driver; m59 added one more under
tests/bus_subscriber.rs that runs the full two-lotus-binary
publisher → reader-thread → cooperative-queue → handler path; m60
added one more under tests/serializer_shape.rs that verifies the
synthesized __serialize_T / __deserialize_T symbols + send-site
calls show up in the IR).

## How to verify the checkpoint

```
cd ~/code/lotus-lang
cargo test --workspace --tests           # 102 passed
cargo run --bin lotus -- run examples/trellis-demo/main.lt
cargo run --bin lotus -- build examples/hello-world/main.lt
./examples/hello-world/main              # prints "hello, world"
rm examples/hello-world/main             # clean up artifact
cargo run --bin lotus -- build examples/01-locus-with-run/main.lt
./examples/01-locus-with-run/main        # tick 0..2 over 1.5s
rm examples/01-locus-with-run/main       # clean up artifact
cargo run --bin lotus -- build examples/02-parent-child/main.lt
./examples/02-parent-child/main          # 3× "greeting from child: ..."
rm examples/02-parent-child/main         # clean up artifact
cargo run --bin lotus -- build examples/06-mutable-counter/main.lt
./examples/06-mutable-counter/main       # prints "n=2"
rm examples/06-mutable-counter/main      # clean up artifact
cargo run --bin lotus -- build examples/07-control-flow/main.lt
./examples/07-control-flow/main          # prints "sum=29 stopped at n=9"
rm examples/07-control-flow/main         # clean up artifact
cargo run --bin lotus -- build examples/08-monotonic-sleep/main.lt
./examples/08-monotonic-sleep/main       # prints tick 0..2 + done; ≥150ms
rm examples/08-monotonic-sleep/main      # clean up artifact
cargo run --bin lotus -- build examples/09-functions/main.lt
./examples/09-functions/main             # prints square(7)=49 / fib(12)=144 / ...
rm examples/09-functions/main            # clean up artifact
cargo run --bin lotus -- build examples/10-stateful-locus/main.lt
./examples/10-stateful-locus/main        # prints total=160 / step=30
rm examples/10-stateful-locus/main       # clean up artifact
cargo run --bin lotus -- build examples/11-drain-dissolve/main.lt
./examples/11-drain-dissolve/main        # parent: birth, child-a/b drain+dissolve, parent: drain+dissolve
rm examples/11-drain-dissolve/main       # clean up artifact
cargo run --bin lotus -- build examples/12-user-types/main.lt
./examples/12-user-types/main            # p.x=3 p.y=4, q.x=13 q.y=8, alice says hello (priority 7)
rm examples/12-user-types/main           # clean up artifact
cargo run --bin lotus -- build examples/05-bus/main.lt
./examples/05-bus/main                   # got: hello from sender-1, ack: hello
rm examples/05-bus/main                  # clean up artifact
cargo run --bin lotus -- build examples/13-decimal-and-exit/main.lt
./examples/13-decimal-and-exit/main      # bid/ask/spread/mid/fee printed
rm examples/13-decimal-and-exit/main     # clean up artifact
cargo run --bin lotus -- build examples/03-closure-test/main.lt
./examples/03-closure-test/main          # collapsed cleanly.
rm examples/03-closure-test/main         # clean up artifact
cargo run --bin lotus -- build examples/trellis-demo/main.lt
./examples/trellis-demo/main             # 5x intent + 3x kernel hot-load
rm examples/trellis-demo/main            # clean up artifact
cargo run --bin lotus -- build examples/03b-closure-absorbed/main.lt
./examples/03b-closure-absorbed/main     # AuditL absorbs the violation, exits 0
rm examples/03b-closure-absorbed/main
cargo run --bin lotus -- build examples/03c-closure-bubbled/main.lt
./examples/03c-closure-bubbled/main      # bubble → exits non-zero
rm examples/03c-closure-bubbled/main
cargo run --bin lotus -- build examples/04-modes/main.lt
./examples/04-modes/main                 # bulk=60, harmonic=3, resolution=30
rm examples/04-modes/main
cargo run --bin lotus -- build examples/14-projection-classes/main.lt
./examples/14-projection-classes/main    # rich/chunked/recognition: total=6
rm examples/14-projection-classes/main
cargo run --bin lotus -- build examples/15-match/main.lt
./examples/15-match/main                 # zero/two/other; status: live/dormant; got value=42
rm examples/15-match/main
cargo run --bin lotus -- build examples/16-schedule-classes/main.lt
./examples/16-schedule-classes/main      # cooperative + main + (50ms) + pinned on its own pthread
rm examples/16-schedule-classes/main
cargo run --bin lotus -- build examples/17-yield/main.lt
./examples/17-yield/main                 # logged tick 1/2/3 with `--- after first/second yield ---`
rm examples/17-yield/main
cargo run --bin lotus -- build examples/18-pinned-lifecycle/main.lt
./examples/18-pinned-lifecycle/main      # main: spawned + pinned.birth/run/drain/dissolve on pinned thread
rm examples/18-pinned-lifecycle/main
cargo run --bin lotus -- build examples/19-pinned-bus/main.lt
./examples/19-pinned-bus/main            # cooperative publisher feeds 3 ticks to pinned subscriber
rm examples/19-pinned-bus/main
cargo run --bin lotus -- build examples/20-pinned-core/main.lt
./examples/20-pinned-core/main           # two pinned workers on cores 0 and 1 (best-effort)
rm examples/20-pinned-core/main
cargo run --bin lotus -- build examples/21-arrays/main.lt
./examples/21-arrays/main                # nums[i] reads + sum_of + dot product over [Int; N]
rm examples/21-arrays/main
cargo run --bin lotus -- build examples/22-moving-average/main.lt
./examples/22-moving-average/main        # 6 samples → smoothed averages 25/75/150/250/350/450
rm examples/22-moving-average/main
cargo run --bin lotus -- build examples/23-ranges/main.lt
./examples/23-ranges/main                # triangular(10)=45, factorial(5)=120, factorial(7)=5040, square>50 at i=8
rm examples/23-ranges/main
cargo run --bin lotus -- build examples/24-default-params/main.lt
./examples/24-default-params/main        # greet/pow with omitted trailing args + Counter.bump default step
rm examples/24-default-params/main
cargo run --bin lotus -- build examples/25-imports/main.lt
./examples/25-imports/main               # multi-file: types.lt + notional.lt + main.lt → "GOOG notional = 17050"
rm examples/25-imports/main
cargo run --bin lotus -- build examples/26-tuples/main.lt
./examples/26-tuples/main                # divmod / let-destructure / numeric field access / classify match
rm examples/26-tuples/main
cargo run --bin lotus -- build examples/27-strings/main.lt
./examples/27-strings/main               # concat / equality / len / inclusive+exclusive slicing / first_word
rm examples/27-strings/main
cargo run --bin lotus -- build examples/28-to-string/main.lt
./examples/28-to-string/main             # per-primitive to_string + label/summary helpers + concat round-trip
rm examples/28-to-string/main
cargo run --bin lotus -- build examples/29-helpers/main.lt
./examples/29-helpers/main               # min/max/abs over Int/Float/Duration + starts_with + contains + clamping classify_temp
rm examples/29-helpers/main
cargo run --bin lotus -- build examples/30-stats/main.lt
./examples/30-stats/main                 # bus aggregator: 6 samples → running n/sum/min/max/avg lines
rm examples/30-stats/main
cargo run --bin lotus -- build examples/31-birth-closures/main.lt
./examples/31-birth-closures/main        # configured locus with birth-epoch closures: pass/pass/fail absorbed by AuditL
rm examples/31-birth-closures/main
cargo run --bin lotus -- build examples/32-restart/main.lt
./examples/32-restart/main               # restart cap-2: target=2 succeeds via 1 retry; target=1 no retry; target=4 hits cap and falls through
rm examples/32-restart/main
cargo run --bin lotus -- build examples/33-quarantine/main.lt
./examples/33-quarantine/main            # quarantine: valid=1 closure passes + run() fires; valid=0 fails + quarantined + run() skipped
rm examples/33-quarantine/main
cargo run --bin lotus -- build examples/34-quarantine-bus/main.lt
./examples/34-quarantine-bus/main        # FailingWatcher quarantined and silenced; HealthyWatcher receives all 3 published samples
rm examples/34-quarantine-bus/main
cargo run --bin lotus -- build examples/35-tick-closures/main.lt
./examples/35-tick-closures/main         # Counter tick-closure: 4 cells fire then violation+quarantine silences the rest
rm examples/35-tick-closures/main
cargo run --bin lotus -- build examples/36-duration-closures/main.lt
./examples/36-duration-closures/main     # Watcher 5ms-duration: cells 2-4 fire (3 fires across 4 cells spaced 7ms apart)
rm examples/36-duration-closures/main
cargo run --bin lotus -- build examples/37-explicit-closures/main.lt
./examples/37-explicit-closures/main     # Ledger explicit balance: imbalance after Tx#3 → Auditor absorbs; Tx#4 rebalances
rm examples/37-explicit-closures/main
cargo run --bin lotus -- build examples/38-restart-in-place/main.lt
./examples/38-restart-in-place/main      # Worker scratch resets to 0 between attempts; cap-2 hit
rm examples/38-restart-in-place/main
cargo run --bin lotus -- build examples/39-multi-instance-bus/main.lt
./examples/39-multi-instance-bus/main    # 3 Watcher instances all receive 3 published Samples
rm examples/39-multi-instance-bus/main
cargo run --bin lotus -- build examples/40-pinned-duration/main.lt
./examples/40-pinned-duration/main       # Coordinator absorbs duration violation fired post-run on pinned thread
rm examples/40-pinned-duration/main
cargo run --bin lotus -- build examples/41-closure-accumulator/main.lt
./examples/41-closure-accumulator/main   # sum(self.delta) running total trips band of 100 at 4th cell; quarantined
rm examples/41-closure-accumulator/main
cargo run --bin lotus -- build examples/46-fn-arenas/main.lt
./examples/46-fn-arenas/main             # m49: per-call subregion + return-copy; [tick:1..5] + sum=42 / label=[sum:42]
rm examples/46-fn-arenas/main
cargo run --bin lotus -- build examples/47-fn-arenas-extras/main.lt
./examples/47-fn-arenas-extras/main      # m51: deep-copy across Array / Struct / has-payload-Enum returns
rm examples/47-fn-arenas-extras/main
cargo run --bin lotus -- build examples/48-publish-during-dissolve/main.lt
./examples/48-publish-during-dissolve/main   # m52: dissolve-time publish dispatched via in-loop drain
rm examples/48-publish-during-dissolve/main
cargo run --bin lotus -- build examples/49-fn-handle-rooting/main.lt
./examples/49-fn-handle-rooting/main         # m53: handles bound in a returning free fn drain + dissolve at fn.exit
rm examples/49-fn-handle-rooting/main
cargo run --bin lotus -- build examples/50-mode-defaults/main.lt
./examples/50-mode-defaults/main             # m54: mode bulk(scale = 10) callable bare or with override
rm examples/50-mode-defaults/main
```

If all of these work, the checkpoint is intact.
