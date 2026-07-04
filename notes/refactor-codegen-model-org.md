# Codegen monolith → model-organized modules: delivery plan

**Branch:** `refactor/codegen-model-org`
**Date drafted:** 2026-05-27
**Status:** EXECUTED — the split shipped; `crates/hale-codegen/src/` now carries `bus/`, `form/`, `locus/`, etc. and `codegen.rs` is ~26.5k lines (down from 45.5k). This note is retained as the delivery plan of record.
**Spec anchor (post-delivery):** would land as `F.34 codegen-file-layout` in `spec/design-rationale.md` once Round 1 ships.

## TL;DR

`crates/hale-codegen/src/codegen.rs` is **45,509 lines** in a single file — 97% of the codegen crate. Every change has to grep the monolith; every feature lives scattered across hundreds of unrelated lines; the verification candidates in issue #18 are *structurally blocked* by it because they need global passes the monolith makes inhumane.

This plan reorganizes the codegen along the lotus / hypergraph language model. **The file layout mirrors the conceptual model:** `locus/`, `bus/`, `form/`, `channels/`, `stdlib/`, `types/`. Each domain gets its own directory; each domain's functions live together.

Done as **7 incremental rounds**, each a self-contained PR with tests green throughout. Round 1 (stdlib extraction) is lowest risk + biggest agent-ergonomics win; Round 4 (locus extraction) is highest risk + highest substrate-payoff. Estimated total: 5-7 sessions of work.

This is structural, not aesthetic. The monolith is a real productivity drag and a real blocker on the formal-verification roadmap.

---

## Motivation

### The monolith's actual cost

`codegen.rs` size by area (rough function counts):

| Area | Functions | Rough line share |
|---|---|---|
| `lower_std_*` (stdlib path-calls) | 142 | ~10,000 lines |
| `@form(...)` synthesis | ~30 | ~7,000 lines |
| Locus codegen (lifecycle, methods, dissolve, m49/m90) | ~40 | ~9,000 lines |
| Bus codegen (subscribe, publish, wire, transport, routing keys) | ~25 | ~5,000 lines |
| Type lowering (CodegenTy, views, interfaces, generics) | ~30 | ~6,000 lines |
| Channels (fallible, structural, closures) | ~15 | ~3,000 lines |
| Shared utilities, struct, dispatch tables, top-level passes | misc | ~5,500 lines |

Adding a stdlib function today (the CRC32 work, #14) touched 4 widely-spaced locations in one file: the extern declaration block, two dispatch tables, and the lower function. For a 50-line feature, the navigation overhead is half the work.

### Why this hurts an AI agent more than a human

Humans build mental models of file regions over time. An AI agent loads what's in context. Today, "work on the bus" means:
- Read `lotus_bus_*` extern declarations (lines ~3970-4500)
- Read dispatch tables (lines ~16700-16820)
- Read `lower_send` and friends (lines ~41000-41700)
- Read `synthesize_serializer` (lines ~5760-5940)
- Read `emit_per_field_serialize` (lines ~5970-6300)
- Read `emit_per_field_deserialize` (lines ~6580-7250)
- Plus the dozen helpers scattered between

That's a working set of ~3,000 lines spanning a 45,000-line file. With model-organized modules, the same task loads `bus/` (probably ~2,000 lines total in 5-6 files) and nothing else. Context budget per task drops by an order of magnitude.

### Connection to open issues

**Issue #9 (m90 return-slot ABI)** — touches the locus-method return path codegen. Today scattered across `synthesize_*`, `emit_locus_*`, the deferred-dissolves machinery, and the m49/m90 return-arena routing. After refactor: lives in `locus/return_path.rs` as a single coherent unit. The implementation effort drops because the conceptual surface drops.

**Issue #18 (formal verification roadmap)** — every candidate localizes:
- **Race-completeness for substrate primitives**: each `runtime/{bus, locus, form}/*.c` is small enough to model independently. The monolithic `lotus_arena.c` (12k lines) mirrors the same problem on the C side.
- **Bus-graph property checks** (cycle / orphan / deadlock detection): the graph IS the `bus/` directory. The walker reads the topic decls + publishers + subscribers as colocated source.
- **Memory-bound proofs**: alloc sites live in `locus/arena.rs` + `form/*`. The annotation pass is mechanical when colocated; impossible when scattered.
- **Resource-budget tracking** (fds, threads, bus subjects): per-resource home in the corresponding module.
- **Closure-assertion lifting**: `channels/structural.rs` is the home for both runtime check and compile-time lift.

The verification roadmap is **structurally blocked** by the monolith. The refactor is the unlock.

### Why now

- Just shipped v0.8.2 — clean stopping point on feature work.
- 11 PRs this session have established a steady iteration rhythm.
- The monolith's cost compounds: every new feature adds to it. The longer we wait, the more work the refactor becomes.
- The verification roadmap (#18) is the next strategic investment. The monolith blocks it.

---

## Target structure

```
crates/hale-codegen/
├── src/
│   ├── lib.rs                        // public API: build_executable, build_program
│   ├── codegen.rs                    // Codegen<'ctx> struct + orchestration only (shrinks over rounds)
│   ├── mangle.rs                     // already separate, unchanged
│   ├── shared/
│   │   ├── mod.rs
│   │   ├── errors.rs                 // CodegenError
│   │   ├── memcpy.rs                 // emit_memcpy_call and llvm IR helpers
│   │   ├── strings.rs                // global_string, build_global_string_ptr helpers
│   │   └── llvm_types.rs             // ptr_t, i8_t, i64_t convenience accessors
│   ├── types/
│   │   ├── mod.rs                    // CodegenTy enum, layout pass entry
│   │   ├── primitives.rs             // Int/Float/Bool/Decimal/Time/Duration
│   │   ├── composite.rs              // structs, enums, tuples, arrays
│   │   ├── views.rs                  // BytesView / StringView (F.30) coercions
│   │   ├── interface.rs              // F.20 interface dispatch + vtables
│   │   └── generics.rs               // monomorphization
│   ├── locus/
│   │   ├── mod.rs
│   │   ├── lifecycle.rs              // birth / run / dissolve emit
│   │   ├── method.rs                 // user fn members
│   │   ├── param.rs                  // params + defaults
│   │   ├── arena.rs                  // per-locus arena alloc helpers
│   │   ├── dissolve.rs               // dissolve cascade, m82 scope-exit
│   │   ├── mode.rs                   // bulk / harmonic / resolution
│   │   ├── placement.rs              // pinned, cooperative pool
│   │   ├── return_path.rs            // m49/m90 — the #9 issue's home
│   │   └── closure.rs                // closure declarations (in-locus assertions)
│   ├── bus/
│   │   ├── mod.rs
│   │   ├── topic.rs                  // topic decls + payload type registration
│   │   ├── publish.rs                // lower_send, dispatch sites
│   │   ├── subscribe.rs              // subscriber registration in birth
│   │   ├── wire.rs                   // serialize/deserialize codegen (incl. #7 bound-check)
│   │   ├── routing_keys.rs           // Phase 3 keyed dispatch
│   │   └── transport.rs              // LOTUS_BUS_CONFIG wiring (udp:// / unix:// / adapters)
│   ├── form/
│   │   ├── mod.rs
│   │   ├── hashmap.rs                // @form(hashmap) synthesis (incl. F.32 sync modes)
│   │   ├── vec.rs                    // @form(vec)
│   │   ├── ring_buffer.rs            // @form(ring_buffer)
│   │   └── shm_ring.rs               // @form(shm_ring) — Form K
│   ├── channels/
│   │   ├── mod.rs
│   │   ├── fallible.rs               // fallible(E) lowering
│   │   ├── structural.rs             // ↑ channel, violate / on_failure
│   │   └── closure_assert.rs         // runtime closure-test check codegen
│   └── stdlib/
│       ├── mod.rs                    // dispatch entry, namespace → module routing
│       ├── process.rs                // std::process::*
│       ├── env.rs                    // std::env::*
│       ├── time.rs                   // std::time::*
│       ├── decimal.rs                // std::decimal::*
│       ├── str.rs                    // std::str::* (parse, predicates, builder)
│       ├── bytes.rs                  // std::bytes::* (at, slice, BytesBuilder)
│       ├── text.rs                   // std::text::* (base64, predicates, tokenize)
│       ├── crypto.rs                 // std::crypto::* (sha1, sha256, hmac, crc32)
│       ├── math.rs                   // std::math::*
│       ├── rand.rs                   // std::rand::*
│       ├── io_fs.rs                  // std::io::fs::*
│       ├── io_file.rs                // std::io::file::File
│       ├── io_stdin.rs               // std::io::stdin::*
│       ├── io_tcp.rs                 // std::io::tcp::*
│       ├── io_udp.rs                 // std::io::udp::*
│       ├── io_tls.rs                 // std::io::tls::*
│       ├── sockopt.rs                // std::io::sockopt::* (named constants)
│       ├── json.rs                   // std::json::*
│       ├── http.rs                   // std::http::* (Server, Handler, Request, Response)
│       ├── log.rs                    // std::log::*
│       ├── test.rs                   // std::test::*
│       ├── cli.rs                    // std::cli::*
│       ├── yaml.rs                   // std::yaml::*
│       └── bus.rs                    // std::bus::* (Adapter, __local_dispatch)
└── runtime/                          // C side (refactor deferred — see "out of scope")
    └── (unchanged this round; possible follow-up plan)
```

### What stays in `codegen.rs` (the residual orchestration file)

After all 7 rounds, `codegen.rs` should be ~2,000 lines containing only:

- `Codegen<'ctx>` struct definition (the shared-state container)
- `build_executable` / `build_program` entry points
- Pass A / pass B orchestration (the top-level passes that drive the rest)
- The top-level `lower_expr` / `lower_stmt` dispatch on AST node kinds (which delegates to module-specific lowering)
- Anything that's truly cross-cutting orchestration

Everything domain-specific moves to its module.

---

## Decision matrix

### Module-method binding pattern

**Decision: trait extensions per module.**

```rust
// src/stdlib/crypto.rs
use crate::{Codegen, CodegenError, CodegenTy, Scope};

pub(crate) trait CryptoStdlib<'ctx> {
    fn lower_std_crypto_sha1(&mut self, args: &[Expr], scope: &Scope<'ctx>)
        -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    fn lower_std_crypto_crc32(&mut self, args: &[Expr], scope: &Scope<'ctx>)
        -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
    // ...
}

impl<'ctx> CryptoStdlib<'ctx> for Codegen<'ctx> {
    fn lower_std_crypto_sha1(&mut self, args: &[Expr], scope: &Scope<'ctx>)
        -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>
    {
        // body lifted verbatim from codegen.rs
    }
    // ...
}
```

In `codegen.rs` (the residual file):

```rust
use crate::stdlib::crypto::CryptoStdlib;
// self.lower_std_crypto_sha1(...) keeps working
```

**Why this and not alternatives:**

- **Free functions** (`crypto::lower_std_crypto_sha1(cg, args, scope)`): forces every call site to switch from `self.foo()` to `crate::module::foo(self, ...)`. Touches the dispatch tables alone (~300 call sites). Trait extensions preserve the call shape.
- **Submodule with own state** (`BusCodegen<'ctx>` owning sub-fields): biggest design churn, biggest payoff long-term. Defer until we feel the pain — refactor-of-refactor if needed.

### Field access from trait methods

**Decision: most `Codegen` fields become `pub(crate)`.**

The Codegen struct has ~50 fields. Trait methods in other files need to read/write them. Two options:

1. Make fields `pub(crate)` — direct access, no boilerplate, one-line diff per field.
2. Add accessor methods (`fn user_types(&self) -> &...`) — encapsulation, ~150 lines of boilerplate, no runtime cost.

Going with (1) for the refactor. The encapsulation isn't load-bearing — Codegen is a private struct of the codegen crate. If we later want stricter boundaries between subsystems, we can tighten field-by-field as boundaries crystallize.

### Helpers shared across modules

**Decision: shared helpers stay as inherent methods on Codegen, in `shared/`.**

Functions like `emit_memcpy_call`, `global_string`, `unpack_view_if_needed` are used everywhere. They stay as `impl Codegen<'ctx>` methods (no trait), defined in `shared/*.rs` modules. Lots of `impl<'ctx> Codegen<'ctx>` blocks across files — fine in Rust.

### Naming convention for module traits

- Stdlib: `<Namespace>Stdlib` (e.g. `CryptoStdlib`, `IoTcpStdlib`)
- Locus: `Locus<Aspect>` (e.g. `LocusLifecycle`, `LocusReturnPath`)
- Bus: `Bus<Aspect>` (e.g. `BusPublish`, `BusWire`)
- Form: `Form<Kind>` (e.g. `FormHashmap`, `FormVec`)
- Channels: `Channel<Kind>` (e.g. `ChannelFallible`, `ChannelStructural`)
- Types: `Type<Aspect>` (e.g. `TypeLayout`, `TypeInterface`)

Each trait re-exported from its `mod.rs` so callers can `use crate::stdlib::*` or pick targeted traits.

---

## Migration strategy: 7 rounds

Each round is a session, ships a PR, leaves tests green.

| Round | Theme | Risk | Est. lines lifted | Sub-rounds |
|---|---|---|---|---|
| **1** | stdlib/ extraction | LOW | ~10,000 | ~25 namespaces |
| **2** | form/ extraction | MEDIUM | ~7,000 | 4 form kinds |
| **3** | bus/ extraction | MEDIUM-HIGH | ~5,000 | 5 aspects |
| **4** | locus/ extraction | HIGH | ~9,000 | 7 aspects |
| **5** | types/ extraction | MEDIUM | ~6,000 | 4 aspects |
| **6** | channels/ extraction | MEDIUM | ~3,000 | 3 aspects |
| **7** | residual cleanup + shared/ | LOW | ~2,000 | — |

Total estimated: ~42,000 lines moved into module files; ~3,500 lines remain in `codegen.rs` as orchestration.

---

## Round 1 detail: stdlib/ extraction

**Goal**: pull all 142 `lower_std_*` functions out of `codegen.rs` into `stdlib/*.rs`, one file per `std::*` namespace.

**Why first**: the stdlib lowering functions are the most self-contained group. Each takes `(args, scope)`, type-checks args, emits one or two `build_call`s, returns. They don't call each other; they only call shared Codegen helpers. The refactor is mechanical — close to copy-paste with import adjustments.

### Sub-rounds (each its own commit, all in one PR)

In approximate order of independence (least cross-cutting first):

1. **1.1 stdlib/process.rs** — `std::process::{pid, exit, run, spawn, wait, kill, write_stdin, read_stdout, read_stderr, dump_arena_residency, rss_bytes}` (~12 fns)
2. **1.2 stdlib/env.rs** — `std::env::{args_count, arg, arg_or, var, var_exists}` (5 fns)
3. **1.3 stdlib/time.rs** — `std::time::{monotonic, monotonic_ns, sleep, now, time_from_unix}` (5 fns)
4. **1.4 stdlib/decimal.rs** — `std::decimal::{to_float}` (1 fn — smallest)
5. **1.5 stdlib/math.rs** — `std::math::{sqrt, exp, log, floor, ceil, pow, tanh, sin, cos, tan, asin, acos, atan, atan2, nan, inf, is_nan}` (~17 fns)
6. **1.6 stdlib/crypto.rs** — `std::crypto::{sha1, sha256, hmac_sha256, crc32}` (4 fns)
7. **1.7 stdlib/rand.rs** — `std::rand::{seed_from_time, next_int}` (2 fns)
8. **1.8 stdlib/text.rs** — `std::text::{base64::encode, base64::decode, is_alpha, is_digit, is_alnum, is_whitespace, is_word_char, tokenize_words_into, md_to_html, Sink}` (~10 fns)
9. **1.9 stdlib/sockopt.rs** — `std::io::sockopt::{30+ named-constant getters}` (~30 fns, all 1-liners)
10. **1.10 stdlib/io_fs.rs** — `std::io::fs::{read_file, write_file, write_file_append, read_bytes, file_size, mkdir, rename, unlink, mktemp, file_exists, list_dir_count, list_dir_at}` (~12 fns)
11. **1.11 stdlib/io_stdin.rs** — `std::io::stdin::{read_line, read_line_status}` (2 fns)
12. **1.12 stdlib/io_file.rs** — `std::io::file::*` and `File` locus methods (~6 fns)
13. **1.13 stdlib/io_tcp.rs** — `std::io::tcp::{listen_socket, connect, accept_one, set_recv_timeout, set_send_timeout, close_fd, send, send_bytes, recv, recv_bytes, recv_into, shutdown_listen_socket}` (~12 fns)
14. **1.14 stdlib/io_udp.rs** — `std::io::udp::{bind, send, recv, recv_into, close, join_group, leave_group, set_multicast_ttl, set_multicast_loop, set_multicast_iface, set_option_int, set_option_bool, get_option_int, recv_with_source, last_source_host, last_source_port, set_recv_timeout, set_send_timeout, send_to_str}` (~19 fns)
15. **1.15 stdlib/io_tls.rs** — `std::io::tls::{connect, send_bytes, recv_bytes, recv_into, close}` (5 fns)
16. **1.16 stdlib/str.rs** — `std::str::{parse_int, parse_float, parse_decimal, can_parse_int, can_parse_float, can_parse_decimal, range_eq, range_parse_int, range_parse_decimal, byte_at_unchecked, index_of, lower, upper, trim, substring, replace, repeat, pad_left, pad_right, from_bytes, clone, builder_new, builder_append, builder_len, builder_finish}` (~25 fns)
17. **1.17 stdlib/bytes.rs** — `std::bytes::{at, slice, from_string, from_int, concat, clone}` + BytesBuilder methods (~12 fns)
18. **1.18 stdlib/json.rs** — `std::json::{find_field_raw, find_string_field, find_int_field, find_bool_field, array_first, array_next, array_first_span, array_next_span, iter_find_field_raw, iter_find_string_field, iter_find_int_field, iter_find_bool_field, iter_substring, find_field_raw_in, Builder}` (~20 fns)
19. **1.19 stdlib/log.rs** — `std::log::{Logger, sinks, builders}` (~6 fns)
20. **1.20 stdlib/http.rs** — `std::http::{Server, Handler, Request, Response, parse_request, write_response, header}` lowering hooks (~10 fns)
21. **1.21 stdlib/test.rs** — `std::test::*` framework (if any path-call surface)
22. **1.22 stdlib/cli.rs** — `std::cli::Resolver` (if any)
23. **1.23 stdlib/yaml.rs** — `std::yaml::*` (if any)
24. **1.24 stdlib/bus.rs** — `std::bus::{Adapter, __local_dispatch}` (these are NOT the bus codegen — those go in Round 3. This is just the stdlib-side dispatch entry.)
25. **1.25 stdlib/mod.rs** — top-level dispatch + the two big `match path { ... }` tables collapsed into per-namespace dispatch fns.

### Dispatch consolidation

Before:
```rust
match path {
    ["std", "crypto", "sha1"] => self.lower_std_crypto_sha1(args, scope),
    ["std", "crypto", "sha256"] => self.lower_std_crypto_sha256(args, scope),
    ["std", "crypto", "hmac_sha256"] => self.lower_std_crypto_hmac_sha256(args, scope),
    ["std", "crypto", "crc32"] => self.lower_std_crypto_crc32(args, scope),
    // ... 138 more
}
```

After:
```rust
match path {
    ["std", "crypto", ..] => self.dispatch_std_crypto(path, args, scope),
    ["std", "io", "tcp", ..] => self.dispatch_std_io_tcp(path, args, scope),
    // ... ~25 namespaces
    _ => Err(CodegenError::Unsupported(format!("unknown stdlib path: {:?}", path))),
}
```

Each namespace module's `dispatch_*` fn does the per-symbol match locally. This both shrinks the top-level dispatch and colocates the dispatch with the implementations.

### Round 1 deliverable

A single PR titled `refactor(codegen): extract stdlib lowering into stdlib/*.rs (Round 1)` containing:

- 25 new files under `src/stdlib/`
- `codegen.rs` shrinks by ~10,000 lines
- Each stdlib module file: 100-1500 lines
- All 160 tests pass unchanged
- No public API change (`hale_codegen::build_executable` works as before)

### Round 1 risk + mitigations

- **Risk: missed import**. Mitigation: `cargo build` after each sub-round catches it within 30 seconds.
- **Risk: typo in trait method signature**. Mitigation: trait method signatures are copy-pasted from the inherent method; compile-time check.
- **Risk: dispatch table miss**. Mitigation: keep the old table commented out alongside the new one for the first sub-round; remove after green tests.
- **Risk: tests fail**. Mitigation: test suite runs in 5-10 minutes; run after every sub-round commit.

### Round 1 validation

After each sub-round commit:
```sh
cargo build --release -p hale-codegen
cargo test --release --workspace --lib --tests -- --test-threads=1
```

After the PR is ready:
```sh
wc -l crates/hale-codegen/src/codegen.rs    # expect ~35,000 (down from 45,509)
wc -l crates/hale-codegen/src/stdlib/*.rs    # each file < 2,000 lines
```

---

## Round 2 detail: form/ extraction

**Goal**: extract `@form(...)` synthesis into `form/*.rs`.

### Sub-rounds

1. **2.1 form/ring_buffer.rs** — `@form(ring_buffer)` synthesis (smallest, ~500 lines)
2. **2.2 form/shm_ring.rs** — `@form(shm_ring)` Form K synthesis
3. **2.3 form/vec.rs** — `@form(vec)` synthesis (get, push, pop, sort, sort_by, etc.)
4. **2.4 form/hashmap.rs** — `@form(hashmap)` synthesis (the biggest — get/set/has/remove/keys/len/iter, plus the F.32 sync mode variants: plain, serialized, striped)

### Round 2 risk

Higher than Round 1 because @form synthesis touches:
- Type system (cell types must resolve)
- Locus codegen (forms are embedded in locus params)
- Bus codegen (form-typed payloads)

But the synthesis functions themselves are self-contained — they emit IR for the get/set/etc. methods. The cross-references are *to* form synthesis, not from. Lifting them out doesn't break callers.

### Round 2 deliverable

PR titled `refactor(codegen): extract @form synthesis into form/*.rs (Round 2)`. `codegen.rs` down to ~28,000 lines.

---

## Round 3 detail: bus/ extraction

**Goal**: extract bus codegen into `bus/*.rs`.

### Sub-rounds

1. **3.1 bus/topic.rs** — topic decl registration + payload type resolution
2. **3.2 bus/wire.rs** — serialize / deserialize codegen (the #7 bound-check work lives here)
3. **3.3 bus/publish.rs** — `lower_send`, dispatch sites, closed-world optimization
4. **3.4 bus/subscribe.rs** — subscriber registration emit (called from `emit_locus_birth`)
5. **3.5 bus/transport.rs** — `LOTUS_BUS_CONFIG` wiring, `register_remote`, adapter dispatch
6. **3.6 bus/routing_keys.rs** — Phase 3 keyed publish/subscribe + on_unmatched policies

### Round 3 risk

Bus is the heart of Hale. Interlocks with:
- Locus lifecycle (subscribers register in `birth`)
- Form (form-typed payloads)
- Channels (fallible publishes via `on_unmatched: fail`)
- Closed-world optimization (consumes bus-graph topology)

Mitigation: extract in dependency order — wire first (zero outward refs), then publish/subscribe (uses wire), then transport (uses register), routing keys last (touches everything).

### Round 3 deliverable

PR titled `refactor(codegen): extract bus codegen into bus/*.rs (Round 3)`. `codegen.rs` down to ~23,000 lines.

---

## Round 4 detail: locus/ extraction

**Goal**: extract the locus codegen — the trickiest because it interlocks with everything.

### Sub-rounds

1. **4.1 locus/param.rs** — params + default-value lowering
2. **4.2 locus/arena.rs** — per-locus arena alloc helpers
3. **4.3 locus/lifecycle.rs** — birth / run / dissolve method emit
4. **4.4 locus/method.rs** — user `fn` member methods (includes m49 method-with-scratch)
5. **4.5 locus/return_path.rs** — m49/m90 sret + return-arena routing (**the #9 issue's home**)
6. **4.6 locus/dissolve.rs** — dissolve cascade, m82 scope-exit, deferred-dissolves frame
7. **4.7 locus/mode.rs** — bulk / harmonic / resolution mode methods
8. **4.8 locus/placement.rs** — pinned, cooperative pool registration + start
9. **4.9 locus/closure.rs** — closure declarations inside locus (capture + violate)

### Round 4 risk

Highest. Locus codegen is the substrate's heaviest concept. Lifecycle interlocks with every method emit; deferred-dissolves frame stack threads through every fn body; m49 calling convention threads through every fn return.

Mitigation:
- Sub-rounds in strict dependency order (smaller, more isolated pieces first).
- Each sub-round is a separate commit reviewed independently.
- The Codegen struct's deferred-dissolves stack stays as a Codegen field — trait methods access it as `self.deferred_dissolves`.

After Round 4, issue #9 (m90 return-slot ABI) can be tackled with the work localized to `locus/return_path.rs` + `locus/method.rs`. That's the strategic case for getting through Round 4 even though it's the hardest.

### Round 4 deliverable

PR titled `refactor(codegen): extract locus codegen into locus/*.rs (Round 4)`. `codegen.rs` down to ~14,000 lines. Probably the longest PR review of the series — worth proposing as a multi-commit PR that reviewers can read commit-by-commit.

---

## Round 5 detail: types/ extraction

**Goal**: extract type-level codegen into `types/*.rs`.

### Sub-rounds

1. **5.1 types/primitives.rs** — Int/Float/Bool/Decimal/Time/Duration lowering
2. **5.2 types/composite.rs** — structs, enums, tuples, arrays
3. **5.3 types/views.rs** — F.30 BytesView / StringView coercions
4. **5.4 types/interface.rs** — F.20 interface dispatch + vtables
5. **5.5 types/generics.rs** — monomorphization (`synthesize_generic_*`)

### Round 5 risk

Medium. Types are referenced everywhere but type lowering itself is self-contained. The risk is privacy — every other module needs read access to type info.

Mitigation: type-info fields stay `pub(crate)` on Codegen. Lookup methods (`fn type_of(&self, ...)`) stay as inherent methods.

### Round 5 deliverable

PR titled `refactor(codegen): extract type codegen into types/*.rs (Round 5)`. `codegen.rs` down to ~8,000 lines.

---

## Round 6 detail: channels/ extraction

**Goal**: extract channel-related codegen.

### Sub-rounds

1. **6.1 channels/fallible.rs** — `fallible(E)` lowering + `or` dispositions
2. **6.2 channels/structural.rs** — `↑` channel, `violate` statement, `on_failure` dispatch
3. **6.3 channels/closure_assert.rs** — runtime closure-test check codegen

### Round 6 risk

Medium. Channels thread through every fallible call site. The codegen functions themselves are self-contained, but they're called from everywhere.

### Round 6 deliverable

PR titled `refactor(codegen): extract channel codegen into channels/*.rs (Round 6)`. `codegen.rs` down to ~5,000 lines.

---

## Round 7 detail: residual cleanup

**Goal**: reduce `codegen.rs` to orchestration only.

### Tasks

- Move shared helpers into `shared/*.rs`
- Move pass-A / pass-B orchestration into clearly-named functions
- Move `Codegen<'ctx>` struct into its own file (`shared/codegen_state.rs` or stay in `codegen.rs` as just-the-struct)
- Update `lib.rs` to re-export the public API
- Audit `pub(crate)` decisions
- Run a final `cargo clippy --release -- -D warnings` pass

### Round 7 deliverable

PR titled `refactor(codegen): residual cleanup + shared/ (Round 7)`. `codegen.rs` final size ~2,000-3,000 lines. The codegen crate is fully model-organized.

---

## Validation gates (every round)

After each sub-round commit:
1. `cargo build --release -p hale-codegen` (catches missing imports + signature mismatches)
2. `cargo test --release --workspace --lib --tests -- --test-threads=1` (full test suite, ~10 minutes)
3. `wc -l crates/hale-codegen/src/codegen.rs` (track shrinkage)

After each round's PR is ready:
1. CI green (push + pull_request triggers)
2. Manual smoke test: build one of the example .hl programs and run it
3. PR review (each round is structural — invite a careful look)

---

## Out of scope (this plan)

- **C runtime refactor** (`lotus_arena.c` at 12,017 lines). Same kind of monolith on the C side — `runtime/{bus, locus, form, stdlib, sched}/*.c` would mirror the Rust structure. Worth doing eventually but lower priority than the Rust side because the C code is less "loaded by agent" pain (it's direct calls, not orchestration through a big struct). Track as `notes/refactor-lotus-arena.md` after Round 1 of this plan ships, if appetite remains.

- **Other crates** (`hale-syntax`, `hale-types`, `hale-runtime`, `hale-cli`, `hale-ts-shim`). All under 1500 lines per file already. No refactor needed.

- **Reorganizing tests**. Test files stay as-is (160 files, each ~150-1000 lines). They consume the public `hale_codegen::build_executable` API and aren't affected.

- **Reorganizing `runtime/stdlib/*.hl`** (the Hale-side stdlib source files). These are already organized by namespace (`runtime/stdlib/{bus, http, io_tcp, ...}.hl`). No change needed.

- **Public API change**. The `hale_codegen` crate's public API (`build_executable`, etc.) stays identical. Consumers of the crate see no diff.

- **Renaming functions**. Function names stay the same to keep the diff small and reviewable. Renaming is a separate cleanup pass after the refactor lands.

- **Performance optimization**. The refactor preserves codegen output bit-for-bit (modulo source-file location info in panics). Build time MAY change up or down; we measure but don't optimize during the refactor.

---

## Open questions

These should be settled before Round 1 starts:

1. **Trait method signatures**: should they be `&mut self` like the inherent methods are today, or `&mut Codegen<'ctx>` directly? `&mut self` is what trait extension idiom expects — sticking with it.

2. **One trait per module or one trait per submodule?** Probably one per submodule (`crypto.rs` defines `CryptoStdlib`; `io_tcp.rs` defines `IoTcpStdlib`). Re-export from `mod.rs` so consumers can `use stdlib::*`. Confirmed in decision matrix above.

3. **`shared/` vs putting helpers in `codegen.rs`?** Resolving in favor of `shared/`. Reason: the orchestration file (`codegen.rs`) should be load-by-itself readable after Round 7. Helpers polluting it work against that.

4. **Per-round PRs or one big PR?** Per-round. Each round is a session of work and a reviewable change. One big PR would be unreadable.

5. **What happens if Round 1 surfaces an unexpected gotcha?** Fall back to a smaller sub-round-only PR (e.g., extract just `stdlib/crypto.rs` as a 1-file proof of concept first, learn from it, then continue). Acceptable cost; signals the approach needs adjustment.

6. **Documentation update**: after the refactor lands, `AGENTS.md` should mention the new structure. Probably a one-paragraph addition to `agents/compiler-dev.md`. Defer to Round 7.

7. **Spec language**: should this be reflected in spec/ at all? The refactor is implementation, not language semantics. Probably the right answer is "spec/design-rationale.md gets an F.34 entry noting the codegen layout post-refactor, for orientation." Defer until after Round 1 ships.

8. **Issue filing**: file a tracking issue (`refactor: model-organized codegen` or similar) with checkboxes for each round. Use it to discuss + track progress. Matches the issue-first workflow established this session.

---

## Why this matters strategically

Three concrete payoffs:

1. **Working set per task drops by an order of magnitude.** Today's "work on the bus" loads ~3,000 lines of monolith. Post-refactor it loads ~2,000 lines of focused bus/ modules. Multiplied across every future contributor session, this is a massive reduction in cognitive load.

2. **Issue #9 (m90 return-slot ABI) becomes tractable.** Today's m90 work would scatter across `synthesize_*`, `emit_locus_*`, the m49 return-arena routing, the deferred-dissolves frame, and at least three other regions of `codegen.rs`. Post-Round-4, it's `locus/return_path.rs` + `locus/method.rs`. The implementation effort drops because the surface drops.

3. **Issue #18 (formal verification roadmap) becomes implementable.** Memory-bound proofs need to enumerate allocation sites. Bus-graph property checks need to walk topology. Resource-budget tracking needs to enumerate fd/socket/thread sites. None of these are feasible on a monolith because the audit is too expensive to do reliably. Post-refactor, each pass lives over its domain's directory.

These compound. The refactor isn't an investment in aesthetics — it's the precondition for the next two strategic features. Without it, both #9 and #18 are blocked behind a 45,509-line wall.

---

## Status / next action

After this plan is reviewed: file the tracking issue, then start Round 1 (stdlib extraction). Round 1 alone is a complete deliverable — even if the appetite for Rounds 2-7 fades, Round 1's payoff in agent ergonomics is worth the session's work.
