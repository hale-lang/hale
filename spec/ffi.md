# spec/ffi.md — Foreign-function interface (`@ffi("c")`)

User-extensible bindings to external C-ABI libraries. Library
authors declare extern symbols in `.ap` source via an `@ffi("c")`
annotation; the compiler emits LLVM `declare` for the signature
and the linker resolves against C source files supplied at build
time. No stdlib expansion is required to bind a new library.

## Syntax

```aperio
@ffi("c") fn raylib_init_window(w: Int, h: Int, title: String) -> ();
@ffi("c") fn raylib_should_close() -> Bool;
@ffi("c") fn raylib_clear_background(c: Color) -> ();
```

Grammar:

```
ffi_annotation ::= '@' 'ffi' '(' STRING ')'
ffi_fn_decl    ::= ffi_annotation 'fn' Ident '(' params ')' ('->' type_expr)? ';'
```

The annotation precedes the `fn` keyword. The fn body MUST be
absent — the declaration terminates with `;`. The compiler
synthesizes an empty body internally so downstream passes keep
the same `FnDecl` shape; user code MAY NOT write a `{...}` block.

The ABI string is the literal `"c"`. Other ABI strings are
reserved for future extensions and are rejected at parse time.

## Position

`@ffi("c")` is valid only on **top-level free fn declarations**.
The annotation is rejected on:

- Locus methods (`locus L { fn ...; }`).
- Mode bodies (`mode bulk { ... }`, `mode harmonic { ... }`, ...).
- Perspective method signatures.
- Interface method signatures.
- Closure declarations.

The position restriction matches the substrate's expectation that
the C-ABI boundary crosses at top-level program scope only; locus
and perspective methods carry implicit Aperio-side context
(`self`, scratch arena, lifecycle hooks) that doesn't translate
to C.

## Restrictions

An `@ffi("c")` fn declaration MUST NOT be:

- **Generic.** Type parameters require monomorphization; the
  C-ABI boundary is monomorphic by definition. Declare separate
  `@ffi` fns per type if needed.
- **Fallible.** `fallible(E)` is an Aperio internal channel; C
  functions report failure via error sentinels in the return
  value, and the Aperio wrapper above translates to `fallible(E)`
  if exposed to user code.
- **Defaulted.** Parameter defaults are not portable across the
  C-ABI boundary; the wrapper layer applies defaults before the
  call.

The parser rejects all three with a diagnostic at the annotation
or marker position.

## Type marshalling

The typechecker validates `@ffi("c")` parameter and return types
against a portable subset. LLVM lowers each Aperio type to a
matching C-ABI representation at the call boundary:

| Aperio type | LLVM type | C type | Notes |
|---|---|---|---|
| `Int` | `i64` | `int64_t` | 64-bit signed throughout. |
| `Float` | `double` | `double` | 64-bit IEEE 754. |
| `Bool` | `i32` | `int32_t` | Aperio's i1 zero-extends to i32 at the call, truncates back at the return. Avoids C `_Bool` cross-platform ambiguity. |
| `String` | `ptr` | `const char *` | NUL-terminated. Caller owns; callee MUST NOT retain past the call. |
| `Bytes` | `ptr` | `void *` (header) | Points at Aperio's `[int64 len][payload]` header — callee uses `lotus_bytes_len(p)` / `lotus_bytes_data(p)` (declared in `lotus_arena.h`) to inspect. Caller owns. |
| `BytesView` / `StringView` | `{ ptr, i64 }` (struct by value) | `lotus_view_t` | 16-byte F.30b view layout. C glue MAY use `lotus_view_data` to recover the payload pointer + length. |
| `Duration` / `Time` | `i64` | `int64_t` | Both are 64-bit nanosecond counts under the hood. |
| `()` (unit) | `void` | `void` | Return-position only — declared as `-> ()` or omitted entirely. Empty-tuple return type accepted but normalized to `()`. |

Reserved at Stage 1 (typecheck rejects with a clear diagnostic):

- `Decimal` — i128 mantissa with platform-variable ABI. Marshal as
  `Int` (raw mantissa) or `Float` (lossy conversion) at the
  Aperio side; the wrapper handles the scale.
- `Uint` — Aperio-internal type; declare as `Int` at the FFI
  signature.
- Projections / fixed-size arrays / tuples — no portable C struct
  layout for these v0 shapes.
- `fallible(E)` — internal channel; see Restrictions above.
- Function-pointer types — wrap as a struct/handle at the C side.
- `LocusRef`, `Cell` — Aperio-internal.

User-type structs (`type Color { r: Int = 0; ... }`) pass through
the typechecker but Stage 1 codegen does not yet wire C-ABI struct
marshalling. Library authors who need struct-by-value parameters
should wait for the struct-passing PR; the typecheck-allowed-but-
codegen-rejects path surfaces a clear `Unsupported` error at
build time.

## Calling convention

`@ffi` fns differ from regular Aperio free fns at the LLVM ABI
level:

- **No implicit `__caller_arena` first parameter.** Regular free
  fns receive the caller's `current_arena_ptr()` as an implicit
  prefix; `@ffi` fns do not.
- **No fallible sret slots.** `@ffi` fns can't be `fallible(E)`,
  so the sret-pair the substrate emits for fallible returns is
  absent.
- **No monomorphization.** `@ffi` fns can't be generic.

The LLVM symbol name is the literal Aperio fn name as written.
There is no `__std_*` mangling, no per-import alias prefix, no
generic-instantiation suffix. The library author's C glue
exports a function with that exact name; the linker resolves
directly.

## Lifetime rules

The Aperio-side caller of an `@ffi` fn owns every pointer it
passes. The C-side callee MUST:

- NOT retain `String` / `Bytes` / view pointers past the call
  boundary. If C needs persistent storage, it must copy into its
  own malloc'd memory.
- NOT free or write through any pointer received from Aperio.
  Arena-owned pointers are read-only at the C side.

If a C function needs to RETURN heap-allocated `String` or
`Bytes`, the convention matches stdlib primitives that allocate
return values: call `lotus_arena_alloc(lotus_caller_arena_or_global(),
size, align)` to land the storage in the caller's arena, then
return the pointer. The caller's arena outlives the C-side
function frame, so the returned pointer survives.

Exceptions MUST NOT cross the FFI boundary. C code that fails
returns an error sentinel (NULL, -1, etc.); the Aperio-side
wrapper translates to a `fallible(E)` shape if the error needs
to propagate.

## Build surface

The `aperio build` CLI accepts repeatable flags that thread the
library author's C glue + link surface through to clang:

```
aperio build mydir/ --link raylib --csrc pond/raylib/glue.c \
                    --link curl   --csrc pond/curl/glue.c
```

- `--link <name>` — appended as `-l<name>` to the clang link
  line. The system's dynamic linker resolves at runtime.
- `--csrc <path>` — passed directly to clang as a translation
  unit compiled alongside the C runtime. The library author's
  `.c` glue file goes here. May be repeated for multiple files.

Both flags are optional; programs that don't use `@ffi`
declarations don't need either.

Stage 2 (future work) will read these from imported libs'
`aperio.toml [ffi]` sections automatically — `import "pond/raylib"`
will be sufficient. Stage 1 wires the CLI flags only.

## Library-author surface

A binding library typically ships:

1. A `.ap` file with `@ffi("c") fn ...;` declarations + the
   user-facing Aperio wrapper (locus, types, idiomatic
   signatures).
2. A `.c` file exporting the C-side symbols declared in the
   `.ap`. Often a thin shim from Aperio's snake_case to upstream
   C naming.
3. (Stage 2) An `aperio.toml [ffi]` section declaring
   `link = [...]` and `csrc = [...]`.

Example skeleton (pond/raylib):

```aperio
// pond/raylib/raylib.ap
@ffi("c") fn raylib_init_window(w: Int, h: Int, title: String) -> ();
@ffi("c") fn raylib_close_window() -> ();

locus Window {
    params { width: Int = 1280; height: Int = 720; title: String = ""; }
    birth()    { raylib_init_window(self.width, self.height, self.title); }
    dissolve() { raylib_close_window(); }
}
```

```c
// pond/raylib/glue.c
#include <stdint.h>
#include "raylib.h"
void raylib_init_window(int64_t w, int64_t h, const char *t) {
    InitWindow((int)w, (int)h, t);
}
void raylib_close_window(void) { CloseWindow(); }
```

```toml
# pond/raylib/aperio.toml (Stage 2)
[ffi]
link = ["raylib"]
csrc = ["glue.c"]
```

## Diagnostic surface

Parser errors:

- `expected ; (an @ffi fn declaration has no body), got LBrace`
  — body block written after the signature; convert to `;`.
- `unsupported FFI ABI "<x>" — Stage 1 accepts only "c"`
- `\`@ffi\` fn must not be generic — the C-ABI boundary is
  monomorphic`
- `\`@ffi\` fn must not be \`fallible(...)\` — C functions
  return an error sentinel, the Aperio wrapper above translates
  to \`fallible(E)\` if needed`
- `expected \`fn\` after \`@ffi(...)\` annotation`

Typecheck errors:

- `\`@ffi\` fn \`<name>\` parameter \`<p>\` has type Decimal —
  Decimal (i128) has platform-variable ABI; marshal as Int/Float
  at the Aperio side instead`
- `\`@ffi\` is only valid on top-level free fns at Stage 1, not
  on locus methods`

Codegen errors:

- `@ffi fn \`<name>\` parameter \`<p>\`: type <T> is not yet
  wired for FFI codegen at Stage 1` — user-type structs, arrays,
  etc. fall here.
- `@ffi fn \`<name>\`: parameter defaults are not supported
  across the C-ABI boundary`

## Cross-references

- `notes/ffi-design.md` — design memo capturing the agreement
  the Stage 1 surface graduated from, plus the Stage 2/3 staging
  plan still pending implementation.
- `spec/stdlib.md` — `std::*` paths are NOT the only way to
  bind C libraries; this spec is the user-extensible alternative.
- `spec/runtime.md` — the C-runtime helpers (`lotus_bytes_*`,
  `lotus_arena_alloc`, `lotus_caller_arena_or_global`, etc.)
  that library authors typically call from C glue.
