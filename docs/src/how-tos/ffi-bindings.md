# Bind a C library

Aperio's stdlib intentionally stays narrow — libc + OpenSSL is
the link floor. When you need to bind to a third-party C
library (raylib, sqlite, curl, SDL, ...) the compiler exposes a
user-extensible FFI mechanism: declare extern symbols with an
`@ffi("c")` annotation, ship a thin C glue file, declare the
link surface in an `aperio.toml`. Consumers `import` the lib like
any other; the build picks up the link line automatically.

This recipe walks the minimum end-to-end. For the full authoring
contract (when to wrap vs leave bare, error-sentinel
translation, layout discipline for struct types) read
[`agents/binding-packages.md`](../../../agents/binding-packages.md).
For the substrate spec — type marshalling rules, lifetime
contract, diagnostic surface — read
[`spec/ffi.md`](../../../spec/ffi.md).

## Minimum example — a `doubler` lib

The library lives in its own directory (anywhere — `pond/`, a
hand-vendored `lib/`, a workspace-internal helper, etc.). Three
files:

```
my-repo/
├── apps/
│   └── main/
│       └── main.ap
└── lib/
    └── doubler/
        ├── aperio.toml         ← [ffi] declares link surface
        ├── doubler.ap          ← the Aperio user surface
        └── glue.c              ← the C-side adapter
```

**`lib/doubler/doubler.ap`** — the Aperio surface:

```aperio
@ffi("c") fn doubler_double(x: Int) -> Int;
```

**`lib/doubler/glue.c`** — the C-side adapter:

```c
#include <stdint.h>
int64_t doubler_double(int64_t x) { return x * 2; }
```

**`lib/doubler/aperio.toml`** — link surface:

```toml
[ffi]
csrc = ["glue.c"]
```

(No `link = [...]` line because this glue doesn't pull any
shared library. A typical binding to libraylib / libsqlite3 /
libcurl would add `link = ["raylib"]` etc.)

**`apps/main/main.ap`** — the consumer:

```aperio
import "../../lib/doubler" as d;

fn main() {
    println("result=", d::doubler_double(21));
}
```

Build + run:

```bash
$ aperio build apps/main/main.ap
built: apps/main/main
$ apps/main/main
result=42
```

No `--link` / `--csrc` flags on the command line — the build
reads `lib/doubler/aperio.toml`'s `[ffi]` section automatically.

## What gets wrapped

`@ffi("c") fn name(args) -> ret;` declarations sit at the top
level of a `.ap` file. The compiler emits an LLVM `declare` for
each; the linker resolves against the C glue at link time. The
fn name IS the C symbol — no mangling, no prefixing applied by
Aperio.

Convention: prefix every `@ffi` fn name with the library's
identifier (`doubler_double`, `raylib_init_window`,
`sqlite3_open_v2`). This is C-side discipline — the symbol
namespace is flat across the entire program, so cross-library
collisions are the binding author's problem to prevent. Stdlib
itself follows the convention with `lotus_*` and
`__StdLangMorpheme*` prefixes for its own primitives.

## What types can cross the boundary

| Aperio type | C type | Notes |
|---|---|---|
| `Int` | `int64_t` | Always 64-bit signed. Narrow at the C side if upstream wants `int32_t` / `uint8_t`. |
| `Float` | `double` | 64-bit IEEE 754. |
| `Bool` | `int32_t` | Avoids C `_Bool` ambiguity. |
| `String` | `const char *` | NUL-terminated. Caller owns; C side MUST NOT retain past the call. |
| `Bytes` | Aperio's `[int64 len][payload]` header pointer | Use `lotus_bytes_len(p)` / `lotus_bytes_data(p)` on the C side to inspect. |
| `BytesView` / `StringView` | 16-byte `{ ptr, i64 }` struct by value | Use `lotus_view_data(v)` to recover the payload pointer + length. |
| `Duration` / `Time` | `int64_t` | Both are 64-bit nanosecond counts. |
| User struct (`type T { ... }`) | `const T *` (param) / `T *out` (return) | Passed by pointer; struct returns use a hidden sret slot. The C side dereferences. Layout match is your responsibility. |

Rejected at typecheck with a clear diagnostic: `Decimal` (i128
ABI is platform-variable), `Uint` (Aperio-internal), projections,
fixed-size arrays, tuples, function-pointer types, fallible
types. Cross-boundary errors flow via error sentinels (a
distinguished return value) which the Aperio wrapper translates
to `fallible(E)` at the user-facing layer.

## Where the canonical contract lives

- [`spec/ffi.md`](../../../spec/ffi.md) — the substrate's
  authoritative contract. Syntax, ABI, lifetime rules,
  diagnostic surface.
- [`agents/binding-packages.md`](../../../agents/binding-packages.md)
  — authoring brief. File layout, three-layer Aperio surface,
  C glue skeleton, naming conventions, optional helpers (idempotent
  init, error-sentinel translation), when `@ffi` is the wrong
  answer (callbacks, vararg, inline / macros).
- [`notes/ffi-design.md`](../../../notes/ffi-design.md) — the
  design memo with the rollout history.
