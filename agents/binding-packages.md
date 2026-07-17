# FFI binding-package conventions

Recommended shape for an Hale library that binds to an external
C-ABI library (raylib, sqlite, curl, ...). Use this brief when
authoring a binding lib for `pond` or any community / vendored
location.

The substrate contract is in [`spec/ffi.md`](../spec/ffi.md);
this file is the *authoring guide* — what file structure to use,
what to wrap, what to leave bare, what conventions earn the
binding its keep across multiple consumers.

## File layout

```
pond/<libname>/
├── hale.toml          # [ffi] section: link libs + csrc files
├── <libname>.hl         # the Hale-side user surface
├── glue.c               # the C-side adapter (often thin)
└── README.md            # one-screen intro (optional but helps)
```

For larger bindings, split the surface across multiple `.hl`
files per topic (`window.hl`, `drawing.hl`, `input.hl`, ...);
the per-dir seed model bundles them as one lib. The `.c` glue
can also split if useful (`glue_drawing.c`, `glue_input.c`).
List each csrc in `hale.toml`'s `csrc = [...]`.

## `hale.toml`

```toml
[ffi]
link = ["raylib"]
csrc = ["glue.c"]
```

- `link` — repeatable list of library names. Each becomes
  `-l<name>` on the consumer's clang link line. Use the system-
  visible name (`raylib`, `curl`, `sqlite3`, ...). Order
  matters only if the libraries themselves have ordering
  dependencies — when in doubt, leave the order as-listed.
- `csrc` — repeatable list of `.c` paths relative to this
  `hale.toml`'s directory. Each compiles as a translation
  unit alongside the runtime. Headers (`*.h`) don't go here —
  use `#include` from the csrc instead.

The section is picked up automatically when this lib is
imported. No consumer-side `--link` / `--csrc` flags needed.

## The Hale surface (`<libname>.hl`)

Three layers, ordered from inside-out:

### 1. `@ffi("c")` declarations (the raw extern surface)

```hale
@ffi("c") fn raylib_init_window(w: Int, h: Int, title: String) -> ();
@ffi("c") fn raylib_close_window() -> ();
@ffi("c") fn raylib_should_close() -> Bool;
@ffi("c") fn raylib_clear_background(c: Color) -> ();
```

Convention: prefix every `@ffi` fn name with `<libname>_`
(`raylib_*`, `sqlite3_*`, `curl_*`). The C-ABI symbol is the
literal fn name; this prefix prevents collisions across multiple
binding libs in the same binary.

Match the C library's API names in snake_case so upstream
reference material translates directly: raylib's `InitWindow`
→ `raylib_init_window`, sqlite3's `sqlite3_prepare_v2` →
`sqlite3_prepare_v2` (already snake_case).

Type signatures use the FFI-portable subset (see
[`spec/ffi.md`](../spec/ffi.md) marshalling table): scalars,
`String`, `Bytes`, views, user-type structs (by pointer).
`Decimal` / `Uint` / projections / tuples / arrays / fallibles
are rejected; convert at the Hale side if needed.

### 2. Hale-side value types (`type Foo { ... }`)

Mirror the C struct's field order + types. The substrate passes
user structs **by pointer** at the boundary, so the Hale
layout must match what the C side sees through the pointer
dereference.

```hale
type Color { r: Int = 0; g: Int = 0; b: Int = 0; a: Int = 255; }
type Vec3  { x: Float = 0.0; y: Float = 0.0; z: Float = 0.0; }
```

**Layout stability is YOUR responsibility.** The compiler does
not yet emit a compile-time layout assertion (`spec/ffi.md`
open question — future spec iteration may add `@ffi_layout("c")`).
For now: write the C struct in glue.c with the same field order
and types; document the layout match at the top of `<libname>.hl`
so future-you doesn't reorder fields without thinking. If the
upstream C library uses smaller integer widths
(`uint8_t` / `int32_t` / `float`), keep the Hale side as
`Int` / `Float` and let the glue do the narrowing — Hale's
single `Int` (i64) and single `Float` (f64) span the upstream
shape with room to spare, and the glue's narrowing is explicit.

### 3. The user-facing locus / wrapper fns

The `@ffi` decls + value types are the raw surface. Most users
shouldn't touch them — wrap in idiomatic Hale:

```hale
locus Window {
    params {
        width:  Int    = 1280;
        height: Int    = 720;
        title:  String = "app";
        target_fps: Int = 60;
    }
    birth() {
        raylib_init_window(self.width, self.height, self.title);
        raylib_set_target_fps(self.target_fps);
    }
    dissolve() { raylib_close_window(); }
}
```

- Lifecycle goes on a `locus` when there's setup / teardown.
- Free fns wrap stateless calls.
- Use Hale idiomatic naming on the wrapper layer (no `raylib_`
  prefix); the prefix is a C-side identifier-uniqueness concern,
  not a user-facing one.

## The C glue (`glue.c`)

Thin adapter. Each exported function:

1. Takes the Hale-side ABI (pointers to user structs, `int64_t`
   for `Int`, `int32_t` for `Bool`, `double` for `Float`,
   `const char *` for `String`).
2. Translates to the upstream C library's actual API
   (narrowing widths, repacking structs, allocating temporary
   state, etc.).
3. Returns via the appropriate ABI shape (scalar return /
   `void` + sret slot for struct returns).

Skeleton:

```c
#include <stdint.h>
#include "raylib.h"

// Hale-side struct shape — MUST match the `type Color`
// declared in raylib.hl. Field order, field types, no padding.
typedef struct {
    int64_t r, g, b, a;
} HaleColor;

void raylib_init_window(int64_t w, int64_t h, const char *t) {
    InitWindow((int)w, (int)h, t);
}
void raylib_close_window(void) { CloseWindow(); }

// Struct passed BY POINTER from Hale side.
void raylib_clear_background(const HaleColor *c) {
    Color rl = {
        (uint8_t)c->r, (uint8_t)c->g,
        (uint8_t)c->b, (uint8_t)c->a,
    };
    ClearBackground(rl);
}

// Struct returns use SRET: hidden out-pointer first arg, void
// return. Hale caller allocates the slot in its arena.
void raylib_get_mouse_position(HaleVec2 *out) {
    Vector2 v = GetMousePosition();
    out->x = (double)v.x;
    out->y = (double)v.y;
}
```

Three things to keep in mind:

- **NUL-terminate every String you pass to the upstream lib.**
  Hale Strings are already NUL-terminated, so this is the
  default — but if you're constructing one from a sliced
  view, make sure the NUL survives.
- **Don't retain Hale pointers past the call.** Every
  `String` / `Bytes` / struct pointer the Hale caller passed
  is owned by an Hale arena; the arena may destroy at
  method exit (scratch subregion) or at locus dissolve. If
  the upstream library needs to retain content, copy via the
  upstream's own allocator.
- **Return heap-allocated `String` / `Bytes` via the caller's
  arena.** If a wrapper needs to return new heap content,
  call `lotus_arena_alloc(lotus_caller_arena_or_global(),
  size, align)` (header in `crates/hale-codegen/runtime/`)
  to land the storage in the caller's arena. Returning a
  malloc'd pointer would leak.

## Optional helpers

### Idempotent init

Many C libs have an init function that's safe to call once but
crashes on second call. Wrap with a static guard:

```c
static int s_inited = 0;
void mylib_init(void) {
    if (s_inited) return;
    UpstreamInit();
    s_inited = 1;
}
```

### Error sentinels → fallible at the Hale wrapper

The C ABI can't carry `fallible(E)` directly. If the upstream
returns an error code:

```c
int64_t mylib_open(const char *path) {
    int fd = upstream_open(path);
    if (fd < 0) return (int64_t)errno * -1;
    return (int64_t)fd;
}
```

Then the Hale wrapper translates:

```hale
@ffi("c") fn mylib_open(path: String) -> Int;

fn open(path: String) -> Int fallible(IoError) {
    let r = mylib_open(path);
    if r < 0 {
        fail IoError { errno: 0 - r, ... };
    }
    return r;
}
```

The user-facing surface is `fallible(IoError)`; the raw `@ffi`
fn returns a plain `Int` per the C ABI.

## Testing

Write a small program inside the binding lib's directory (e.g.
`pond/raylib/smoketest/main.hl`) that exercises the minimal
surface:

```hale
import "../" as ray;

fn main() {
    let w = ray::Window { width: 320, height: 240, title: "smoke" };
    while !ray::raylib_should_close() {
        ray::raylib_begin_frame();
        ray::raylib_clear_background(ray::Color { r: 26 });
        ray::raylib_end_frame();
    }
}
```

`hale build pond/raylib/smoketest/` should produce a binary
that, when run, opens the upstream library's window and
exercises the call paths. Manual smoke verification — full
integration tests inside the binding lib are usually a
diminishing-returns investment.

## Naming + tagging

- Repo or directory name: `<libname>` matching the upstream
  C library's common name (`raylib`, `sqlite3`, `curl`,
  `libpq`).
- Hale alias the consumer picks: their call (`import
  "vendor/raylib" as ray;`). Default to the lib's short
  natural name.
- `@ffi("c")` symbol names: prefixed with the C library's
  identifier. Even when the upstream uses unprefixed names
  (`fopen`, `read`), prefix at the Hale side
  (`libc_fopen` if you're binding pieces of libc) to avoid
  collisions in the consuming binary.

## When `@ffi` is the wrong answer

Some C surfaces don't translate cleanly:

- **Callbacks.** Hale's FnPtr doesn't currently cross the FFI
  boundary. If the upstream library demands `void
  (*callback)(int)`, the binding has to manage state at the
  C side (typically via a global registry of Hale-side
  handlers) and the Hale surface exposes "subscribe to
  events" rather than "register a callback."
- **vararg fns.** C `printf`-style variadic surfaces have no
  Hale-side analogue. Bind a fixed-arity wrapper at the C
  side and expose that.
- **Inline functions / macros.** Anything not exported as a
  real symbol can't be reached from `@ffi`. Wrap in a real
  function in glue.c.

## See also

- [`spec/ffi.md`](../spec/ffi.md) — canonical contract
  (syntax, ABI, lifetime rules, diagnostic surface).
- [`notes/ffi-design.md`](../notes/ffi-design.md) — design
  memo with the 3-stage rollout history.
- [`spec/styleguide.md`](../spec/styleguide.md) §1 + §4 — the
  hot-path memory shapes a binding lib's Hale wrapper may
  encounter when persisting upstream state in `self.X` fields
  (absorbed the old `agents/memory-patterns.md`).
