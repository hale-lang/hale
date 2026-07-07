# pond/sqlite is a library, not a language primitive — the `@ffi` recipe

**Verdict (WS4, 2026-06-11).** The pond/sqlite FRICTION § F.1 premise —
"architecturally blocked behind a missing stdlib primitive
(`std::db::sqlite::*`)" — is **incorrect**. Hale already ships the general
C-ABI binding surface a database driver needs: `@ffi("c")` (see
`spec/ffi.md`, opening line: *"No stdlib expansion is required to bind a new
library."*). A SQLite driver is therefore a **pure pond library**, built the
same way `pond/term` / `pond/tui` once bound their C glue and the downstream tool demo
binds raylib. **The language repo should NOT add `std::db::sqlite`.**

Why the friction read as a language block: AGENTS.md's *"Don't edit
`crates/` — compiler territory"* was taken to mean "any C binding needs the
compiler." But `@ffi` is a **library-author** surface — it never touches
`crates/`. The author missed it (or wrote F.1 before it landed).

Every capability the driver needs is verified working at HEAD:

| Need | `@ffi` support | Gate |
|---|---|---|
| Opaque `sqlite3*` / `sqlite3_stmt*` handles | `Int` ⇄ `int64_t` | `ffi_basic` |
| SQL text, paths in | `String` ⇄ `const char *` | `ffi_basic` |
| `column_text(...) -> String` out | `String` return ⇄ `const char *` | **`ffi_string_return`** (added this pass) |
| Error reporting | C returns a sentinel code; the Hale wrapper maps to `fallible(SqliteError)` | — |
| Link libsqlite3 | `hale.toml` `[ffi] link = ["sqlite3"]` → `-lsqlite3` | CLI `link_libs` |

(The one prerequisite is `libsqlite3-dev` on the build box / CI — a deploy
concern, identical to the existing `-lssl`/`-lcrypto` system deps.)

## The recipe (build this in `pond/sqlite/`, no compiler change)

### 1. `glue.c` — thin `lotus_sqlite_*` wrappers (csrc)

```c
#include <sqlite3.h>
#include <stdint.h>
#include <string.h>

// Handles cross the boundary as int64 addresses.
int64_t lotus_sqlite_open(const char *path) {
    sqlite3 *db = NULL;
    if (sqlite3_open(path, &db) != SQLITE_OK) {
        // db is non-NULL even on error; keep it so errmsg is readable,
        // but a real driver may sqlite3_close + return 0. Simplest:
        // return 0 on failure, stash code/msg in thread-locals (below).
        sqlite3_close(db);
        return 0;
    }
    return (int64_t)(intptr_t)db;
}
int64_t lotus_sqlite_exec(int64_t h, const char *sql) {
    return sqlite3_exec((sqlite3 *)(intptr_t)h, sql, 0, 0, 0); // 0 = SQLITE_OK
}
int64_t lotus_sqlite_prepare(int64_t h, const char *sql) {
    sqlite3_stmt *st = NULL;
    if (sqlite3_prepare_v2((sqlite3 *)(intptr_t)h, sql, -1, &st, 0) != SQLITE_OK)
        return 0;
    return (int64_t)(intptr_t)st;
}
int64_t lotus_sqlite_step(int64_t st) {
    return sqlite3_step((sqlite3_stmt *)(intptr_t)st); // 100=ROW 101=DONE
}
// column_text: sqlite owns the pointer until the next step/finalize, so the
// Hale wrapper must clone it immediately (std::str::clone) before stepping.
const char *lotus_sqlite_column_text(int64_t st, int64_t col) {
    const unsigned char *t = sqlite3_column_text((sqlite3_stmt *)(intptr_t)st, (int)col);
    return t ? (const char *)t : "";
}
int64_t lotus_sqlite_column_int(int64_t st, int64_t col) {
    return sqlite3_column_int64((sqlite3_stmt *)(intptr_t)st, (int)col);
}
int64_t lotus_sqlite_finalize(int64_t st) {
    return sqlite3_finalize((sqlite3_stmt *)(intptr_t)st);
}
int64_t lotus_sqlite_changes(int64_t h)  { return sqlite3_changes((sqlite3 *)(intptr_t)h); }
int64_t lotus_sqlite_last_rowid(int64_t h){ return sqlite3_last_insert_rowid((sqlite3 *)(intptr_t)h); }
int64_t lotus_sqlite_close(int64_t h)    { return sqlite3_close((sqlite3 *)(intptr_t)h); }
// For SqliteError.detail, expose the last errmsg:
const char *lotus_sqlite_errmsg(int64_t h){ return sqlite3_errmsg((sqlite3 *)(intptr_t)h); }
```

### 2. `ffi.hl` — the extern declarations

```hale
@ffi("c") fn lotus_sqlite_open(path: String) -> Int;
@ffi("c") fn lotus_sqlite_exec(h: Int, sql: String) -> Int;
@ffi("c") fn lotus_sqlite_prepare(h: Int, sql: String) -> Int;
@ffi("c") fn lotus_sqlite_step(st: Int) -> Int;
@ffi("c") fn lotus_sqlite_column_text(st: Int, col: Int) -> String;
@ffi("c") fn lotus_sqlite_column_int(st: Int, col: Int) -> Int;
@ffi("c") fn lotus_sqlite_finalize(st: Int) -> Int;
@ffi("c") fn lotus_sqlite_changes(h: Int) -> Int;
@ffi("c") fn lotus_sqlite_close(h: Int) -> Int;
@ffi("c") fn lotus_sqlite_errmsg(h: Int) -> String;
```

### 3. `hale.toml` — link the system library

```toml
[ffi]
csrc = ["glue.c"]
link = ["sqlite3"]
```

### 4. `db.hl` — the Hale wrapper applies fallibility

`@ffi` fns can't be `fallible` (spec rule). The library wraps them: check the
sentinel, and on failure `fail SqliteError { kind, sqlite_code, detail:
std::str::clone(lotus_sqlite_errmsg(h)) }`. `column_text`'s result is cloned
immediately so it survives the next `step`. This is exactly the
`DbError`-translation layer pond/sqlite's CONTRACTS.md already describes —
the bodies just call the `@ffi` fns instead of returning `kind:"unsupported"`.

## Hand-off

- **pond:** un-stub `pond/sqlite/` using the recipe above; drop the
  `conn_handle = 0` markers; restore method-shaped surfaces. No wait on the
  language. Ensure CI has `libsqlite3-dev`.
- **language repo:** done — `ffi_string_return` gates the String-return
  capability the driver leans on. A docs item (WS5) should make `@ffi`
  discoverable so the next driver author doesn't re-file F.1.
