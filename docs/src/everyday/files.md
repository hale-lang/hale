# Files

Filesystem operations in Hale live under `std::io::fs` (whole-file reads, writes, and directory operations) and `std::io::file` (streaming file handles). Every operation that can fail returns `fallible(IoError)` and must be addressed at the call site with `or`.

```hale
fn main() {
    std::io::fs::write_file("greeting.txt", "hello\n") or raise;

    let body = std::io::fs::read_file("greeting.txt") or "(empty)";
    println(body);
}
```

## `std::io::fs` and `std::io::file` Reference

All functions below are `fallible(IoError)` except `file_exists(path)` and `f.read_line()`:

| Function / Method | Signature | Description |
|---|---|---|
| `std::io::fs::read_file(path)` | `(String) -> String fallible(IoError)` | Reads an entire UTF-8 text file into a `String` |
| `std::io::fs::read_bytes(path)` | `(String) -> Bytes fallible(IoError)` | Reads an entire binary file into `Bytes` |
| `std::io::fs::write_file(path, text)` | `(String, String) -> () fallible(IoError)` | Creates or truncates `path` and writes `text` |
| `std::io::fs::write_bytes(path, b)` | `(String, Bytes) -> () fallible(IoError)` | Creates or truncates `path` and writes binary `Bytes` |
| `std::io::fs::write_file_append(path, text)` | `(String, String) -> () fallible(IoError)` | Appends `text` to `path` (creating it if absent) |
| `std::io::fs::file_size(path)` | `(String) -> Int fallible(IoError)` | Returns file size in bytes |
| `std::io::fs::file_exists(path)` | `(String) -> Bool` | Infallible test for path existence |
| `std::io::fs::mkdir(path)` | `(String) -> () fallible(IoError)` | Creates directory `path` |
| `std::io::fs::rename(from, to)` | `(String, String) -> () fallible(IoError)` | Moves or renames a file or directory |
| `std::io::fs::unlink(path)` | `(String) -> () fallible(IoError)` | Deletes a file |
| `std::io::fs::mktemp(prefix, suffix)` | `(String, String) -> String fallible(IoError)` | Creates a unique temporary file and returns its path |
| `std::io::fs::list_dir_count(path)` | `(String) -> Int fallible(IoError)` | Returns number of entries in directory `path` |
| `std::io::fs::list_dir_at(path, i)` | `(String, Int) -> String fallible(IoError)` | Returns the name of the `i`th directory entry |
| `std::io::file::open(path, mode)` | `(String, String) -> File fallible(IoError)` | Opens a streaming `File` locus (`"r"`, `"w"`, `"a"`) |
| `f.read_line()` | `() -> String` | Reads the next line from `File` `f` (`""` at EOF) |

## Handling `IoError`

When a filesystem call fails, its `IoError` payload provides four fields:

| Field | Type | Description |
|---|---|---|
| `kind` | `String` | Stable cross-platform category: `"not_found"`, `"permission_denied"`, `"already_exists"`, `"is_dir"`, `"invalid"`, etc. |
| `path` | `String` | Path argument that triggered the error |
| `errno` | `Int` | Raw OS error code |
| `detail` | `String` | Human-readable OS message (such as `"No such file or directory"`), suitable for logging |

Branch on `e.kind` (rather than parsing `e.detail`) to recover from specific failure conditions:

```hale
fn handle_io(e: IoError) -> String {
    if e.kind == "not_found" {
        return "";                     // treat missing file as empty
    }
    eprintln("io error on ", e.path, ": ", e.kind, " (", e.detail, ")");
    return "";
}

fn load(path: String) -> String {
    return std::io::fs::read_file(path) or handle_io(err);
}

fn main() {
    let body = std::io::fs::read_file("config.toml") or {
        eprintln("no config.toml; using defaults");
        return;
    };
    println("config: ", len(body), " bytes");
}
```

### Idempotent Directory Setup

Because `mkdir` returns `()`, you can use `or discard` when ensuring a directory exists without failing if it was already created:

```hale,fragment
std::io::fs::mkdir("cache") or discard;
```

*(Using `or raise` inside an infallible `fn main()` halts the process with `Hale panic: unhandled IoError escaping main locus` if the call fails.)*

## Streaming Files (`std::io::file::File`)

`read_file` and `write_file` load or write an entire file in one call. To read a large file incrementally line by line, open a `std::io::file::File` locus:

```hale,fragment
let f = std::io::file::open("log.txt", "r") or raise;
let line = f.read_line();              // returns "" at end of file
```

Because `File` is a locus, its `dissolve()` hook automatically closes the underlying OS file descriptor when `f` goes out of scope—no manual `close()` call is required.

## Compression and Tar Archives (`std::compress` and `std::tar`)

`std::compress` and `std::tar` operate over `Bytes` buffers in memory and fail with `IoError`:

```hale
fn ship(report: Bytes) fallible(IoError) {
    let a = std::tar::pack(std::bytes::from_string(""),
        "out/report.json", report) or raise;
    let archive = std::tar::finish(a) or raise;
    let tgz = std::compress::gzip(archive) or raise;
    std::io::fs::write_bytes("report.tar.gz", tgz) or raise;
}
```

| Function | Description |
|---|---|
| `std::compress::gzip(b)` / `gunzip(b)` | Built-in gzip compression and decompression (`Bytes -> Bytes fallible(IoError)`) |
| `std::compress::zstd(b)` / `unzstd(b)` | Zstandard compression and decompression (loads `libzstd` on first use; fails with `kind == "not_found"` if unavailable) |
| `std::tar::pack(buf, name, data)` / `finish(buf)` | Appends an entry to an in-memory tar buffer and finalizes the archive |
| `std::tar::entries(b)` / `entry_name(b, i)` / `entry_data(b, i)` | Inspects entry count, file path, and payload of an unpacked tar buffer |

> [!NOTE]
> **Decompression Safety Bounds:** Corrupt or truncated archives fail with `err.kind == "invalid"` rather than crashing, and decompression refuses to expand past `1 GiB` in a single call so a zip bomb returns an `IoError` instead of exhausting memory.

Next: [JSON](./json.md).
