# FFI design — `@ffi` declarations for external `.hl` libraries

**Status:** SHIPPED — `@ffi` landed 2026-05-22; the normative
contract now lives in `spec/ffi.md`. This note is retained as the
originating design record. It captures the agreement from the
2026-05-22 session that pivoted away from "ship `std::raylib` /
`std::pty` in stdlib" toward a user-extensible FFI mechanism.

## Why

Today every C-ABI binding lives inside the compiler in three
hardcoded places: (1) C symbol in `crates/hale-codegen/runtime/`,
(2) lowering function in `codegen.rs`, (3) path-rename entry in
`STDLIB_PATH_RENAMES`. There is no user-extensible mechanism.

The cost: every new third-party C library requires a compiler
change. Iris asks for raylib + pty, other downstream apps want
sqlite + a PostgreSQL client, curl / SDL / ffmpeg / etc. —
each one becomes a "wait for the compiler team" handoff and a
permanent expansion of stdlib's link surface.

A user-extensible FFI lets library authors land their own
bindings in pond (or any community repo), keeping stdlib's
"libc + OpenSSL only" link floor and decoupling app development
from compiler release cadence.

`std::http::recv_chunk` (iris's third ask) stays a stdlib
addition — HTTP is already in stdlib, this is just an extension.

## Mechanism

A library author declares external C functions in a `.hl` file:

```hale
// pond/raylib/raylib.hl

@ffi("c") fn raylib_init_window(w: Int, h: Int, title: String) -> ();
@ffi("c") fn raylib_close_window() -> ();
@ffi("c") fn raylib_should_close() -> Bool;
@ffi("c") fn raylib_clear_background(c: Color) -> ();

type Color { r: Int = 0; g: Int = 0; b: Int = 0; a: Int = 255; }

locus Window {
    params { width: Int = 1280; height: Int = 720; title: String = "iris"; }
    birth()    { raylib_init_window(self.width, self.height, self.title); }
    dissolve() { raylib_close_window(); }
}
```

And declares the link surface in `hale.toml`:

```toml
[ffi]
link = ["raylib"]
csrc = ["src/raylib_glue.c"]
```

`src/raylib_glue.c` is hand-written by the library author —
thin wrappers from `raylib_*` snake_case names to raylib's
upstream `InitWindow` / `CloseWindow` / etc. Or skip the rename
and call upstream symbols directly (the `@ffi("c")` name IS the
symbol the linker resolves).

The build (`hale build`) reads `hale.toml` for every
imported lib, accumulates `link` and `csrc` lists, and includes
them in the clang invocation. Conditional on imports — a binary
that doesn't `import "pond/raylib"` doesn't link libraylib.so.

## ABI contract

Type marshalling at the FFI boundary (Hale side → C side):

| Hale type | C type | Notes |
|---|---|---|
| `Int` | `int64_t` | 64-bit signed throughout. |
| `Float` | `double` | 64-bit IEEE 754. |
| `Bool` | `int32_t` | Avoids C `_Bool` cross-platform ambiguity. |
| `String` | `const char *` | NUL-terminated. Caller owns; callee MUST NOT retain past the call. |
| `Bytes` | `const void *` | Points at Hale's `[int64 len][payload]` header — callee uses `lotus_bytes_len` / `lotus_bytes_data` if it needs to inspect (header defined in `runtime/lotus_arena.c`). |
| `BytesView` / `StringView` | `{ ptr, i64 epoch }` struct by value | The compact 16-byte view layout from F.30b. |
| `Decimal` | rejected at typecheck | i128 ABI is too platform-variable for an FFI; library author should marshall via Int/Float at the Hale side. |
| `Duration` / `Time` | `int64_t` | Both are i64-ns under the hood. |
| User `type` (struct by value) | C struct, same field order + layout | Library author guarantees layout match. Compiler emits a layout assertion at compile time when it can compute sizes. |
| `LocusRef` / `fallible` / `Cell` | rejected at typecheck | Not portable across the FFI boundary. |

Lifetime rules:

- **Arena-owned pointers may NOT outlive the call.** C code receiving
  a `String` / `Bytes` / view pointer must NOT cache it across calls.
  If C needs to retain content, it must copy into its own storage.
- **Returns by value or into caller-provided buffers.** C functions
  returning `String` / `Bytes` MUST allocate via `lotus_arena_alloc`
  using `lotus_caller_arena_or_global()` — same contract as stdlib
  primitives that return heap types today.
- **No exceptions cross the boundary.** C code that fails must return
  an error sentinel; the Hale-side wrapper translates to a
  `fallible(E)` shape if needed.

## Staging

| Stage | Scope | Output | Status |
|---|---|---|---|
| 1 | Parser + typecheck + LLVM `declare` emit for `@ffi` decls. Manual link flags via `hale build --link X --csrc Y.c`. | Library authors can ship `.c` glue separately and consumers add link flags by hand. Validates the mechanism end-to-end. | shipped 2026-05-22 (a5f71c7) |
| 2 | `hale.toml [ffi]` section read by `hale build`. Per-lib `link` + `csrc` automatically accumulated from imported libs. | `import "pond/raylib"` is sufficient. No manual flags. | shipped 2026-05-22 |
| 3 | pond conventions — pattern doc for "how to write an FFI binding lib." Recommended `glue.c` shape, what to wrap, what to leave bare, layout-stability guidance for value-type marshalling. | Ecosystem ergonomics. Reproducible binding-package shape. | shipped 2026-05-22 (`agents/binding-packages.md`) |

## First-PR scope (Stage 1)

Minimum slice that compiles + links + runs an FFI fn end-to-end:

1. **Parser** — `@ffi("c")` annotation accepted on free fn decls.
   Body must be absent (`;` terminator instead of `{ ... }`).
2. **Typecheck** — `@ffi` fns skip body verification. Param types
   restricted to the ABI-allowed set above. `Decimal` / locus /
   fallible / cell rejected with a clear diagnostic.
3. **Codegen** — for `@ffi` fns:
   - Emit LLVM `declare` (not `define`) with the C-compatible
     signature derived from the Hale types per the ABI table.
   - At call sites, lower as a direct call to the LLVM declared
     symbol. No path-rename table entry, no `__std_*` munging.
   - Strings pass as raw NUL-terminated char pointers (unpack the
     Hale String to its data pointer).
   - User-type structs pass by value per SysV (the codegen already
     does this for bus payloads — reuse).
4. **Build** — `hale build` accepts `--link <name>` (repeatable)
   and `--csrc <path>` (repeatable). Each `-l<name>` goes to the
   clang link line; each `--csrc` file compiles alongside the rest
   of the runtime. No `hale.toml` integration yet.
5. **Test** — one fixture in `crates/hale-codegen/tests/` that:
   - Declares `@ffi("c") fn ffi_test_double(x: Int) -> Int;`
   - Ships a tiny `.c` file with `int64_t ffi_test_double(int64_t x) { return x * 2; }`
   - Builds, runs, asserts the return value.
6. **Spec** — `spec/ffi.md` documents the ABI contract + the
   `@ffi("c")` syntax. Cross-link from `spec/types.md` and
   `spec/stdlib.md`.

## Open questions

- **Annotation syntax.** `@ffi("c")` is one option; alternatives
  are `extern "c" fn`, `@extern("c")`, a `ffi { ... }` block. The
  string arg is a forward hook for future ABIs (`@ffi("system")`,
  `@ffi("rust")`) but YAGNI today; could just be `@ffi`.
- **Symbol naming.** Should `@ffi("c") fn raylib_init_window` resolve
  to the C symbol `raylib_init_window` (default match) or require an
  explicit `@ffi("c", symbol = "InitWindow")` for renames? Default-
  match is friendlier; rename-annotation can be added later.
- **Struct ABI assertions.** Compiler knows Hale struct layout; it
  COULD emit a `_Static_assert(sizeof(Color) == 16)` into the
  generated C glue header. Useful safety net but requires the
  build to surface a header file to the lib's `.c`. Defer to Stage 3.
- **Pre-1.0 stability.** FFI surfaces ARE the most stable thing in
  any language. Once `@ffi` ships, breaking it in pre-1.0 is more
  painful than breaking arbitrary syntax. Worth a v1.0-blocker
  marker so the syntax gets a final pass before lock-in.

## Coordination

- Iris's `iris/COMPILER_FFI.md` needs an update: pivot from
  "std::raylib in stdlib" to "pond/raylib via @ffi". Their stub
  surface stays unchanged (Window locus, Color type, etc. all
  match the same Hale surface either way).
- This memo lives in `notes/` while the design is provisional.
  Once Stage 1 ships, the contract graduates to `spec/ffi.md` and
  this file gets pruned.
- The memory note `project_sqlite_deferred_to_pond` (2026-05-17)
  is unblocked as a side effect — sqlite becomes the second
  consumer of `@ffi`, validating it for non-graphics use.

## Tradeoff acknowledgement

This pushes the user-visible "ship raylib/pty" timeline out. The
fast path (write `std::raylib` lowerings, ship in 2-3 sessions)
solves iris's near-term need but doesn't solve the next FFI need
or the one after that — each is another 2-3 sessions and another
expansion of stdlib's link surface. The `@ffi` path costs 1-2
weeks upfront but every subsequent FFI binding becomes
zero-compiler-cost.

The tradeoff is fine because iris is not blocked — they have
working stubs against the same surface contract. Their production
code doesn't change when the bindings flip from stub to real.
