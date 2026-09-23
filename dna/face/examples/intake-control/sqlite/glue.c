/* glue.c — thin lotus_sqlite_* shims over sqlite3_*.
 *
 * Per spec/ffi.md, @ffi("c") marshals Hale Int as int64_t and
 * String as a NUL-terminated const char *. Opaque sqlite3* /
 * sqlite3_stmt* pointers cross the boundary as int64 addresses.
 *
 * Error policy: every shim returns the raw SQLITE_* result code
 * (or a 0 handle on open/prepare failure); the Hale wrapper in
 * db.hl maps sentinels to `fail DbError { ... }`. Exceptions
 * never cross the FFI boundary (spec/ffi.md § Lifetime rules).
 *
 * String returns (errmsg / column_text) hand back pointers OWNED
 * BY SQLITE, valid only until the next step/finalize/close on the
 * same object. The Hale wrapper clones them immediately
 * (std::str::clone) — see db.hl.
 *
 * Build: consumers pick this file up automatically via the
 * sibling hale.toml `[ffi] csrc = ["glue.c"], link = ["sqlite3"]`
 * (spec/ffi.md § hale.toml [ffi] auto-pickup). Requires the
 * libsqlite3 development package (sqlite3.h + -lsqlite3).
 */

#include <sqlite3.h>
#include <stdint.h>
#include <string.h>

/* ---- connection lifecycle ---- */

/* Last open() failure message. sqlite3_open hands back a non-NULL
 * db even on failure; we read its errmsg, stash it here, close the
 * half-open handle, and return 0. Single static buffer — open
 * failures are rare and the Hale side reads it immediately in
 * birth()'s aftermath; not thread-safe, like sqlite3_errmsg itself
 * under SQLITE_THREADSAFE=0. */
static char g_open_err[512];
static int64_t g_open_errcode;

int64_t lotus_sqlite_open(const char *path) {
    sqlite3 *db = NULL;
    int rc = sqlite3_open(path, &db);
    if (rc != SQLITE_OK) {
        const char *m = db ? sqlite3_errmsg(db) : "out of memory";
        strncpy(g_open_err, m, sizeof(g_open_err) - 1);
        g_open_err[sizeof(g_open_err) - 1] = 0;
        g_open_errcode = rc;
        sqlite3_close(db);
        return 0;
    }
    g_open_err[0] = 0;
    g_open_errcode = 0;
    return (int64_t)(intptr_t)db;
}

const char *lotus_sqlite_open_error(void) { return g_open_err; }
int64_t lotus_sqlite_open_errcode(void) { return g_open_errcode; }

/* close: rc intentionally dropped — dissolve() (the only caller)
 * is a lifecycle method and cannot fail; a SQLITE_BUSY close
 * (unfinalized statements) leaks the handle rather than crashing. */
void lotus_sqlite_close(int64_t h) {
    sqlite3_close((sqlite3 *)(intptr_t)h);
}

/* ---- one-shot exec ---- */

int64_t lotus_sqlite_exec(int64_t h, const char *sql) {
    return sqlite3_exec((sqlite3 *)(intptr_t)h, sql, 0, 0, 0); /* 0 = SQLITE_OK */
}

/* ---- prepare / bind / step / column / finalize ---- */

int64_t lotus_sqlite_prepare(int64_t h, const char *sql) {
    sqlite3_stmt *st = NULL;
    if (sqlite3_prepare_v2((sqlite3 *)(intptr_t)h, sql, -1, &st, 0) != SQLITE_OK)
        return 0; /* errcode/errmsg readable from h */
    return (int64_t)(intptr_t)st;
}

/* SQLITE_TRANSIENT: Hale owns `val` only for the duration of the
 * call (spec/ffi.md lifetime rule — callee must not retain), so
 * sqlite must take its own copy. */
int64_t lotus_sqlite_bind_text(int64_t st, int64_t idx, const char *val) {
    return sqlite3_bind_text((sqlite3_stmt *)(intptr_t)st, (int)idx, val, -1,
                             SQLITE_TRANSIENT);
}

int64_t lotus_sqlite_bind_int(int64_t st, int64_t idx, int64_t val) {
    return sqlite3_bind_int64((sqlite3_stmt *)(intptr_t)st, (int)idx, val);
}

int64_t lotus_sqlite_step(int64_t st) {
    return sqlite3_step((sqlite3_stmt *)(intptr_t)st); /* 100=ROW 101=DONE */
}

int64_t lotus_sqlite_column_count(int64_t st) {
    return sqlite3_column_count((sqlite3_stmt *)(intptr_t)st);
}

/* Pointer owned by sqlite until the next step/finalize on `st`.
 * NULL columns read as "" (the tab-separated Row shape has no
 * NULL channel in v0). */
const char *lotus_sqlite_column_text(int64_t st, int64_t col) {
    const unsigned char *t =
        sqlite3_column_text((sqlite3_stmt *)(intptr_t)st, (int)col);
    return t ? (const char *)t : "";
}

int64_t lotus_sqlite_column_int(int64_t st, int64_t col) {
    return sqlite3_column_int64((sqlite3_stmt *)(intptr_t)st, (int)col);
}

int64_t lotus_sqlite_finalize(int64_t st) {
    return sqlite3_finalize((sqlite3_stmt *)(intptr_t)st);
}

/* ---- connection metadata / diagnostics ---- */

int64_t lotus_sqlite_changes(int64_t h) {
    return sqlite3_changes((sqlite3 *)(intptr_t)h);
}

int64_t lotus_sqlite_last_rowid(int64_t h) {
    return sqlite3_last_insert_rowid((sqlite3 *)(intptr_t)h);
}

int64_t lotus_sqlite_errcode(int64_t h) {
    return sqlite3_errcode((sqlite3 *)(intptr_t)h);
}

/* Pointer owned by sqlite, valid until the next API call on h —
 * the Hale wrapper clones immediately. */
const char *lotus_sqlite_errmsg(int64_t h) {
    return sqlite3_errmsg((sqlite3 *)(intptr_t)h);
}

/* App-local additions; see PROVENANCE.md. */
int64_t lotus_sqlite_open_existing(const char *path) {
    sqlite3 *db = NULL;
    int rc = sqlite3_open_v2(path, &db, SQLITE_OPEN_READWRITE | SQLITE_OPEN_FULLMUTEX, NULL);
    if (rc != SQLITE_OK) { if (db) sqlite3_close(db); return 0; }
    return (int64_t)(intptr_t)db;
}
int64_t lotus_sqlite_autocommit(int64_t h) { return sqlite3_get_autocommit((sqlite3 *)(intptr_t)h); }
int64_t lotus_sqlite_busy_timeout(int64_t h, int64_t ms) { return sqlite3_busy_timeout((sqlite3 *)(intptr_t)h, (int)ms); }
int64_t lotus_sqlite_column_type(int64_t st, int64_t col) { return sqlite3_column_type((sqlite3_stmt *)(intptr_t)st, (int)col); }
