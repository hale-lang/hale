# `std::io::fs`

Filesystem operations for Aperio programs. Phase 1 (m75) shipped
four one-shot synchronous functions: `read_file`, `write_file`,
`file_size`, `file_exists`. m89 added `read_bytes` (binary-safe
read). m90 added `list_dir` (directory listing).

Each is a path-call function — no locus wrapping, no streaming
handle, no buffering layer. The shape mirrors `std::process::pid`
rather than the Listener-style locus pattern because file ops
are inherently one-shot: there's no lifetime-of-a-stream concept
to manage.

A future milestone that needs streaming reads (large files,
line-by-line processing, tailing logs) adds a separate
streaming family alongside these without disturbing them.

## Functions

### `std::io::fs::read_file`

#### Synopsis

```aperio
fn read_file(path: String) -> String
```

Reads the entire file at `path` and returns its contents as a
String. If the path is missing or unreadable, returns the empty
String. To distinguish "missing" from "empty," probe with
`file_exists` first.

#### Semantics

- Two-phase: stats the file to learn its size, allocates a
  (size + 1)-byte buffer in the lazy global payload arena
  (so the result outlives the call frame), reads into it,
  NUL-terminates at the actual bytes-read offset.
- Returns the empty String on any error (missing path,
  permissions, IO failure). The C substrate's -1 return is
  clamped to 0 in the Aperio surface so callers don't have
  to handle the negative case.
- The returned String references arena memory that lives for
  the program's duration. Repeated reads accumulate; on a
  long-running process this is unbounded growth (acceptable
  for v1 since subscribers run for bounded duration —
  mirrors the m70 deserialize convention).

#### Examples

```aperio
fn main() {
    if std::io::fs::file_exists("config.toml") {
        let contents = std::io::fs::read_file("config.toml");
        println("loaded: ", contents);
    } else {
        println("no config; using defaults");
    }
}
```

### `std::io::fs::write_file`

#### Synopsis

```aperio
fn write_file(path: String, content: String) -> Int
```

Writes `content` to `path`, truncating any existing file.
Returns 0 on success, -1 on error.

#### Semantics

- Opens the path with `O_WRONLY | O_CREAT | O_TRUNC`, mode
  `0644`. Existing files are replaced wholesale; existing
  permissions are preserved (POSIX `open` doesn't change mode
  on existing files).
- Length is computed from the content's String pointer via
  `strlen`. Aperio Strings are NUL-terminated in memory, so
  embedded NULs in payloads silently truncate the write at
  the first NUL. (This mirrors the m70 String wire-format
  contract.)
- Checks `close()`'s return so deferred filesystem errors —
  NFS write-back, ENOSPC surfacing on flush — produce a -1
  rather than being silently dropped.

#### Examples

```aperio
fn main() {
    let log = "request from 127.0.0.1\n";
    let r = std::io::fs::write_file("audit.log", log);
    if r == 0 {
        println("logged");
    } else {
        println("log write failed");
    }
}
```

### `std::io::fs::write_file_append`

#### Synopsis

```aperio
fn write_file_append(path: String, content: String) -> Int
```

Append `content` to the file at `path`. Creates the file with
mode 0644 if it doesn't exist; otherwise opens existing for
append. Returns 0 on success, -1 on error.

#### Semantics

- Companion to `write_file` (which truncates). Opens with
  `O_WRONLY | O_CREAT | O_APPEND` and no `O_TRUNC`, so each call
  extends the file rather than replacing it.
- Bounded buffering pattern dissolves: a long-running sink can
  call `write_file_append` per event instead of accumulating in
  memory and flushing at dissolve.
- Length passed via `lotus_str_len`; same NUL-truncation caveat
  as `write_file` applies (use binary-safe primitives if you
  need to write embedded NULs).
- Returns -1 on any IO error; errno is set on the C side but
  not surfaced at the Aperio level (consistent with the v0
  errno-collapse convention).

#### Examples

A log sink that writes each event as it arrives:

```aperio
fn append_event(path: String, line: String) {
    let r = std::io::fs::write_file_append(path, line + "\n");
    if r != 0 {
        eprintln("append failed: ", path);
    }
}
```

### `std::io::fs::mkdir`

#### Synopsis

```aperio
fn mkdir(path: String) -> Int
```

Create the directory at `path` with mode 0755. Returns 0 on
success, -1 on error. Single-level only — *not* recursive.

#### Semantics

- Wraps libc `mkdir(path, 0755)`. If the parent directory does
  not exist, the call fails with `-1` (errno set to ENOENT on
  the C side).
- If the directory already exists, the call returns -1 (errno
  EEXIST). Test via `file_exists` first if "create-or-noop"
  semantics are desired.
- For `mkdir -p`-style recursive creation, walk the path and
  call `mkdir` on each segment yourself. v0 keeps the surface
  minimal.

#### Examples

Self-bootstrapping output directory in a CLI:

```aperio
fn ensure_output_dir(out: String) {
    if std::io::fs::file_exists(out) == 0 {
        let r = std::io::fs::mkdir(out);
        if r != 0 {
            eprintln("mkdir failed: ", out);
            std::process::exit(1);
        }
    }
}
```

### `std::io::fs::file_size`

#### Synopsis

```aperio
fn file_size(path: String) -> Int
```

Returns the size of `path` in bytes, or -1 on error. Follows
symlinks (uses `stat`, not `lstat`).

#### Semantics

- Stats the path; returns `st.st_size` cast to Int.
- Errors (missing file, permission denied) collapse to -1.
  Callers that need to distinguish use `file_exists` plus
  errno (the latter not currently surfaced in the Aperio
  layer; a future error-introspection milestone fills that in).

#### Examples

```aperio
fn main() {
    let n = std::io::fs::file_size("CHANGELOG.md");
    if n > 0 {
        println("CHANGELOG is ", n, " bytes");
    }
}
```

### `std::io::fs::file_exists`

#### Synopsis

```aperio
fn file_exists(path: String) -> Bool
```

Returns `true` if `path` exists, `false` otherwise. Follows
symlinks; non-existent symlink targets report `false`.

#### Semantics

- Probes via `stat`. Any error (ENOENT, EACCES, etc.) returns
  `false`. The function does not distinguish between
  "definitively absent" and "couldn't tell."

#### Examples

```aperio
fn main() {
    if std::io::fs::file_exists("/etc/hostname") {
        let h = std::io::fs::read_file("/etc/hostname");
        println("hostname: ", h);
    }
}
```

### `std::io::fs::extension`

#### Synopsis

```aperio
fn extension(path: String) -> String
```

Returns the path's file extension, including the leading dot
(`.go`, `.md`), or the empty string when the path has no
extension. Mirrors the conventional split used by Python's
`os.path.splitext` and Rust's `Path::extension`.

#### Semantics

- Inspects the basename only: a dot inside an earlier
  directory segment (`a.b/c`) does not count as the file's
  extension.
- A leading-dot file (`.bashrc`, `src/.config`) has no
  extension by this rule.
- Multiple dots resolve to the last one: `archive.tar.gz`
  returns `.gz`.
- The returned String lives in the lazy global payload arena
  (same lifetime convention as `read_file` / `list_dir`), so
  it is safe to stash past the call frame.

#### Examples

```aperio
fn main() {
    if std::io::fs::extension("main.go") == ".go" {
        println("a Go source file");
    }
}
```

Replaces the per-app `__ends_with_source` / `__ends_with_go`
helper that several extractors hand-rolled before this
primitive landed.

### `std::io::fs::read_bytes`

#### Synopsis

```aperio
fn read_bytes(path: String) -> Bytes
```

Reads the entire file at `path` and returns its contents as a
`Bytes` value (length-preserved; embedded NULs survive). Returns
a zero-length `Bytes` on any error. m89.

#### Semantics

- Two-phase like `read_file`: stats the file to learn its
  size, allocates a `[i64 len][u8 data[len]]` blob in the
  lazy global payload arena, reads into it.
- Returns a zero-length `Bytes` on missing path, permissions,
  or IO failure. To distinguish "missing" from "empty," probe
  with `file_exists` first.
- The returned `Bytes` references arena memory that lives for
  the program's duration. Same accumulation caveat as
  `read_file`.

#### Examples

```aperio
fn main() {
    let body = std::io::fs::read_bytes("logo.png");
    println("loaded ", len(body), " bytes");
}
```

Use this instead of `read_file` whenever the payload may
contain binary content (images, archives, compiled artifacts).
String-mediated reads will silently truncate at the first NUL.

### `std::io::fs::list_dir`

#### Synopsis

```aperio
fn list_dir(path: String) -> String
```

Lists the entries of directory `path` as a single
newline-separated String. Returns an empty String on error.
m90.

#### Semantics

- Two-pass `opendir` / `readdir`: first pass measures total
  bytes, second pass writes into an arena-allocated buffer.
- Skips `.` and `..` entries.
- Order matches `readdir` order — i.e., **not** sorted. If
  you need a sorted listing, sort the lines after splitting.
- Subdirectory recursion is not performed; one level only.
- Returns the empty String on any error (missing path, not a
  directory, permission denied, etc.).

#### Examples

```aperio
fn main() {
    let listing = std::io::fs::list_dir("docs/");
    let n = len(listing);

    // Iterate by walking newline-separated entries.
    let mut start: Int = 0;
    while start < n {
        let rest = listing[start..n];
        let nl = std::str::index_of(rest, "\n");
        if nl < 0 {
            // Last entry (no trailing newline).
            println("entry: ", rest);
            return;
        }
        let entry = listing[start..(start + nl)];
        println("entry: ", entry);
        start = start + nl + 1;
    }
}
```

The newline-separated `String` shape is the v0 representation
because Aperio doesn't yet have a generic `List<T>` for
returning `[String]`. A `[String]` overload lands when dynamic
arrays do; in the meantime the **index API** (`list_dir_count`
+ `list_dir_at` below) provides the canonical iteration shape.

### `std::io::fs::list_dir_count` / `list_dir_at`

#### Synopsis

```aperio
fn list_dir_count(path: String) -> Int
fn list_dir_at(path: String, idx: Int) -> String
```

Phase 2e (2026-05-11). Index-based iteration over the same
directory listing `list_dir` produces. `list_dir_count` returns
the number of entries (skipping `.` / `..`); `list_dir_at`
returns the `idx`-th entry (0-indexed) or the empty string if
out of range. Both walk the same global-arena cache, so the
directory read amortises across both calls — no re-stat per
entry.

#### Examples

The canonical iteration shape — 4 lines, no manual
newline-scanning, no conflation of "blank line" with "no more
entries":

```aperio
fn main() {
    let p = "docs/";
    let n = std::io::fs::list_dir_count(p);
    let mut i = 0;
    while i < n {
        let name = std::io::fs::list_dir_at(p, i);
        println("entry: ", name);
        i = i + 1;
    }
}
```

Out-of-range indexing is well-defined:

```aperio
fn main() {
    let p = "docs/";
    let n = std::io::fs::list_dir_count(p);
    let missing = std::io::fs::list_dir_at(p, n);
    // missing is "" — len(missing) == 0
}
```

A missing or non-directory `path` returns `count == 0`; no
errno surface yet (use `file_exists` first if you need to
disambiguate).

### `std::io::fs::read_file_status`

#### Synopsis

```aperio
fn read_file_status(path: String) -> Int
```

Phase 2f (2026-05-11). Returns 0 on success or the platform
errno on failure (ENOENT = 2 for missing, EACCES = 13 for
permission denied, EISDIR = 21 for path-is-dir, EIO for partial
read). Pairs with the existing `read_file(path)` for the content
itself — both calls share the kernel cache, so the cost of the
second call is the hot-cache stat + open + read.

Use this when `read_file` returning `""` could mean either
"intentionally empty file" or "the read failed" and the program
needs to branch on which.

#### Examples

```aperio
fn main() {
    let content = std::io::fs::read_file("config.toml");
    let status = std::io::fs::read_file_status("config.toml");
    if status != 0 {
        println("read failed: errno=", status);
        return;
    }
    if len(content) == 0 {
        println("config is intentionally empty");
        return;
    }
    println("loaded ", len(content), " bytes");
}
```

## Limitations (Phase 1 + m89/m90)

- **No streaming**: the entire file is read into memory in
  one call. Large files (hundreds of MB+) are uncomfortable.
  `read_bytes` has the same constraint. (`write_file_append`
  helps for the write side: long-running sinks can append
  per event without buffering.)
- **`mkdir` is single-level**: no recursive `mkdir -p` shape.
  Walk the path and call `mkdir` per segment for recursive
  creation.
- **`list_dir` returns String, not [String]**: the canonical
  iteration today uses `list_dir_count` + `list_dir_at` (Phase
  2e); a real `[String]` return waits on dynamic-array codegen
  support.
- **No recursive directory walk**: `list_dir` is one level.
  Recursion is hand-rolled by the caller.
- **No filesystem watch**: m94 (planned) will add
  `std::fs::watch` for inotify-style file-change events.
- **NUL-truncation on `write_file`**: Aperio Strings are
  NUL-terminated in memory, so writing binary data with
  embedded NULs truncates at the first NUL. Use
  `read_bytes` + (future) `write_bytes` for binary I/O.
  Currently there is no `write_bytes`.
- **Partial errno surface**: `read_file_status` (Phase 2f)
  disambiguates read failures; other primitives (`write_file`,
  `mkdir`, ...) still collapse to `-1` / `false` / `""`. A
  future milestone widens the errno surface to the rest.
- **Lazy global arena growth**: every read call (text or
  bytes) allocates from a process-lifetime arena. Long-running
  processes that re-read files repeatedly grow memory
  unbounded.

## See Also

- [Roadmap](../roadmap.md) — Phase 1+ stdlib build-out plan.
- [`std::io::tcp`](./tcp.md) — sibling I/O module for
  network sockets.
- `crates/aperio-codegen/runtime/lotus_arena.c` (in the
  language repo) — POSIX wrappers backing this module.
