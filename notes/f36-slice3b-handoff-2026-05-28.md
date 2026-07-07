# F.36 Slice 3b — handoff (2026-05-28)

> **Update (2026-05-28 PM): Slice 3b landed.** Approach diverged
> from the plan below — instead of synthesizing thunks that
> bridge to the Slice 3a runtime ABI (`void*(*)(void*, void*)`),
> the thunks match the existing m70 `lotus_serialize_fn` /
> `lotus_deserialize_fn` shapes and are substituted at codegen
> time at the publish-site and subscribe-register-site. The
> runtime dispatch path is untouched. The Slice 3a register
> call + remote-entry codec slots remain in place but are
> vestigial. See `spec/design-rationale.md § F.36` for the
> shipped surface and
> `crates/hale-codegen/tests/codec_dispatch_roundtrip.rs` for
> the XOR round-trip acceptance test. The brief below preserves
> the pre-implementation plan for historical reference.

Companion brief to `spec/design-rationale.md § F.36`. Captures the
state at end-of-session 2026-05-28 with concrete pointers for
picking up Slice 3b in a fresh session.

## What landed (F.36 Slices 1, 2, 3a)

All on `main`, all CI-green.

| Slice | Commit | Surface |
|---|---|---|
| Sketch | `09bf81b` | `spec/design-rationale.md § F.36` (~250 lines) |
| Slice 1 | `2ea33c4` | `crates/hale-types/src/purity.rs` (~600 lines) — per-method purity inference, dormant |
| Slice 2 | `0e88aac` | `crates/hale-types/src/check.rs::check_binding_codec` — `codec(L)` parse + AST + typecheck assertion (signatures + purity), diagnostic surface live |
| Slice 3a | `87701b4` | `crates/hale-codegen/src/codegen.rs::emit_codec_binding_register` + C runtime `lotus_bus_register_codec` — codec instantiated at main prelude with method ptrs registered; dispatch still falls through to m70 |

## The Slice 3b blocker

The runtime expects function pointers of shape:

```c
void   *(*encode_fn)(void *codec_self, void *value);
ssize_t (*decode_fn)(void *codec_self, const void *wire_buf,
                     size_t wire_n, void *struct_out, size_t cap);
```

User-authored Hale (Slice 2 enforces these signatures):

```hale
fn encode(v: Tick) -> Bytes fallible(EncErr)
fn decode(b: Bytes) -> Tick fallible(DecErr)
```

These don't have matching LLVM calling conventions. Bridging
requires per-binding synthesized thunks. The fallible-method
calling convention is what stopped the session — too deep to
reverse-engineer safely without dedicated archaeology.

## Goal

Replace m70 `__serialize_T` / `__deserialize_T` with codec method
calls on the publish + receive dispatch paths for bindings
carrying `codec(L)`. Round-trip a custom-format payload through
`unix(role: listen/connect)` and verify both encode and decode
fire.

## Concrete plan

### Step 1 — ABI archaeology (~30 min)

Investigate how existing fallible methods lower in codegen.
Concrete reference cases:

- Search `crates/hale-codegen/src/codegen.rs` for
  `FallibleCallResult` and `lower_*_fallible`
- Cleanest example: `std::process::run(argv) -> ProcessOutput
  fallible(IoError)` in `crates/hale-codegen/src/stdlib/process.rs`
- Document the convention in a code comment: success-channel
  slot, err-channel slot, discriminator shape (bool return?
  sentinel pointer? extra out-arg?)

This step's output: a code comment + 1-page understanding of
the fallible ABI. Don't write codegen yet.

### Step 2 — Encode thunk synthesis (~1 hour)

Per binding with `codec(L)`, codegen emits an LLVM fn next to
the codec locus:

```llvm
define ptr @__codec_encode_thunk_<L>_<TopicSubject>(
        ptr %codec_self, ptr %value_ptr) {
    ; Allocate fallible result slots in current arena
    ; Call L::encode(codec_self, *value_ptr) via Hale's
    ;   fallible calling convention
    ; If success: return the Bytes ptr (raw, m70 wire-shape)
    ; If err: return null (caller treats null as "encode failed,
    ;   drop this publish")
}
```

Modify Slice 3a's `emit_codec_binding_register` to point the
registration at the THUNK ptr, not the bare method ptr.

### Step 3 — Decode thunk synthesis (~1 hour)

```llvm
define i64 @__codec_decode_thunk_<L>_<TopicSubject>(
        ptr %codec_self, ptr %wire_buf, i64 %wire_n,
        ptr %struct_out, i64 %cap) {
    ; Allocate a Hale Bytes header in current frame:
    ;   { i64 wire_n, [body...] }
    ; memcpy wire_buf -> body
    ; Call L::decode(codec_self, &bytes) via Hale's fallible
    ;   calling convention
    ; If err: return -1
    ; If success: memcpy result T into *struct_out,
    ;   return sizeof(T)
}
```

Tricky bit: `struct_out` is a caller-provided buffer in the
reader thread's stack. Hale's `decode` returns a fresh T
allocated in the payload arena. Need a memcpy from
payload-arena-T into struct_out. Get `sizeof(T)` from
`user_types.get(T_name).struct_ty.size_of()`.

### Step 4 — Publish-site rewrite (~30 min)

Find `lower_send` (or wherever `Topic <- value` lowers):

```sh
grep -rn "build_call.*serialize\|__serialize_" crates/hale-codegen/src/
```

Add a codec-aware branch:

- Build a `BTreeMap<topic_name, codec_subject>` from main's
  bindings during the prelude pass
- When emitting send for a topic in that map: emit
  `encode_thunk(codec_self_global, &value)` to get Bytes ptr;
  pass bytes (length + body) to existing transport dispatch
- When topic isn't in the codec map: fall through to m70
  `__serialize_T` (unchanged path)

### Step 5 — Reader-thread rewrite (~30 min)

In C runtime, find the call sites of `__deserialize_T`:

```sh
grep -n "deserialize(wire_buf" crates/hale-codegen/runtime/lotus_arena.c
```

Approximate locations:
- Unix reader thread (around line ~5260)
- UDP reader thread (around line ~8780)

At each site: check `entry->codec_decode_fn != NULL`. If yes,
call it; if no, fall through to m70. The `entry` is the
`lotus_bus_remote_entry_t` for the receiving binding.

### Step 6 — End-to-end test (~1 hour)

Round-trip a value through a custom-format codec via
`unix(role: listen/connect)` in a single binary. Use an XOR
codec so the wire bytes are NOT the raw value — if m70 were
running, the wire format would survive without XOR and the
round-trip would also work, masking the bug. The XOR ensures
the test fails if either encode or decode wasn't called.

```hale
type Msg { tag: Int = 0; }
type EncErr { kind: String = ""; }
type DecErr { kind: String = ""; }

topic MsgTopic { payload: Msg; subject: "msgs"; }

locus XorCodec {
    fn encode(v: Msg) -> Bytes fallible(EncErr) {
        // XOR v.tag with 0xAAAA_AAAA, emit 4-byte big-endian
        ...
    }
    fn decode(b: Bytes) -> Msg fallible(DecErr) {
        // Reverse the XOR
        ...
    }
}

main locus App {
    bus {
        publish   MsgTopic;
        subscribe MsgTopic as on_msg;
    }
    bindings {
        MsgTopic: unix("/tmp/codec_e2e.sock")
                  codec(XorCodec { });
    }
    fn on_msg(m: Msg) {
        println("[sub] tag=", m.tag);
    }
    run() {
        MsgTopic <- Msg { tag: 42 };
        std::time::sleep(200ms);
    }
}
fn main() { App { }; }
```

Expected stdout: `[sub] tag=42`. The codec methods MUST have
run (encode + decode) for the round-trip to surface the
original value — XOR + un-XOR yields 42 only if both fired.

For extra rigor, add `println` inside `encode` and `decode` and
assert both appear in stdout. But Slice 2's purity check
rejects println in codec methods. Two options:
- Skip typecheck for the test build (parse-only + direct
  `build_executable` call)
- Add a `__force_unsafe_codec` build flag that suppresses
  purity verification (probably overkill)

Cleanest: the parse-only test path.

## Acceptance criteria

1. Existing tests still pass: `bindings_codec_clause` (7 cases),
   `codec_instantiation` (1 case)
2. New round-trip test passes
3. A bindings program WITHOUT a codec still works (regression
   check against existing `bus_routing_keys`, `http_hello`)
4. `cargo test --release -p hale-codegen` exits 0

## File pointers

### Codegen
- `crates/hale-codegen/src/codegen.rs`:
  - `emit_codec_binding_register` (added in Slice 3a) — modify
    to point at thunks instead of bare method ptrs
  - `lower_send` (or equivalent) — add codec-aware branch
  - NEW: synthesis pass for encode/decode thunks (mirror
    `synthesize_coop_pool_run_wrappers` shape from F.31)
- `crates/hale-codegen/src/shared/builtins.rs`:
  - `lotus_bus_register_codec` declaration (already added in
    Slice 3a, no changes needed)

### Runtime
- `crates/hale-codegen/runtime/lotus_arena.c`:
  - `lotus_bus_remote_entry_t::codec_self` / `codec_encode_fn`
    / `codec_decode_fn` fields (added Slice 3a)
  - `lotus_bus_register_codec` function (added Slice 3a)
  - Reader thread call sites — modify to consult
    `codec_decode_fn` before falling back to m70

### Tests
- New: `crates/hale-codegen/tests/codec_dispatch_roundtrip.rs`

### Reference / model
- F.31 cooperative-pool wrapper synthesis: `synthesize_coop_pool_run_wrappers`
  in `codegen.rs` — provides a clean pattern for "synthesize a
  thunk per X" codegen
- Existing fallible-call site: `std::process::run` in
  `stdlib/process.rs` — shows the fallible calling convention
  in action

## Don't forget

- Update `spec/design-rationale.md § F.36` status line from
  "design sketch" / "Slice 1/2/3a shipped" to "F.36 complete"
- Update `spec/grammar.ebnf` if any new grammar lands (likely
  none — Slice 3b is codegen-only)
- Run the doc audit: `docs/src/concepts/the-bus.md` should
  mention codec dispatch as a working feature, not a sketch

## Other open items (parked, lower priority)

### Held — waiting on others
- A downstream app udp:// silent-drop verdict (waiting on
  `LOTUS_BUS_LOG_DESERIALIZE_DROP=1` rebuild + report)
- A downstream app WS server bringup (compiler is ready; they're writing
  user-side WS protocol code)

### Compiler-substantial (deferred)
- `where async_io` on pool=main — Go-shape default; 3-5 days,
  lifecycle cliff (App.run() races with fn main() body
  semantics, drain-to-completion shutdown)

### Compiler-incremental (do-on-demand)
- Outbound `connect()` not wired through park (~10 lines)
- `accept4(SOCK_NONBLOCK)` micro-optimization

### Issue #18 — verification roadmap (5 candidates remaining)
- #2 Race-completeness on substrate primitives (TLA+/Loom)
- #1 Memory-bound proofs (biggest lift; foundation for #3 + #5)
- #4 Bus-graph property checks
- #5 Resource-budget tracking (builds on #1)
- #3 Closure-assertion lifting (builds on #1)

### Downstream substrate-side follow-ups
- `@form(hashmap).set` large-cell anchor (~200 MB/min leak) —
  partly fight-Hale (BookSignalState wants per-symbol locus
  tower); substrate fix still useful for legit big-value-typed
  cells
- m90 fallible-return with N≥3 form-children (100% repro;
  workaround in tree)
- `pond/metrics::Histogram.observe()` allocation-heavy
