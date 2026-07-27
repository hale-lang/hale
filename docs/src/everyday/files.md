# Files

> **Coming from Python / Node?** No `try/except`, no
> `.catch()`, no checking `err != nil`. Every filesystem call
> that can fail returns a [`fallible`](../basics/fallible.md)
> value, and the compiler makes you address it with `or` right
> where you call it. The failure is visible at the call site,
> always.

## Reading and writing

```hale
fn main() {
    // Write a file (creating or truncating it).
    std::io::fs::write_file("greeting.txt", "hello\n") or raise;

    // Read it back. read_file returns the whole contents as a String.
    let body = std::io::fs::read_file("greeting.txt") or "(empty)";
    println(body);
}
```

For `main` to use `or raise`, `main` would need to be fallible;
more often at the top level you substitute or report:

```hale
fn main() {
    let body = std::io::fs::read_file("config.toml") or {
        eprintln("no config; using defaults");
        return;
    };
    use_config(body);
}
```

## The surface

All of these live under `std::io::fs` and all are
`fallible(IoError)` except `file_exists`:

| Call | Does |
|---|---|
| `read_file(path) -> String` | whole-file read |
| `read_bytes(path) -> Bytes` | whole-file read, binary |
| `write_file(path, contents)` | create / truncate |
| `write_bytes(path, b: Bytes)` | create / truncate, binary |
| `write_file_append(path, contents)` | append |
| `file_size(path) -> Int` | size in bytes |
| `mkdir(path)` | create a directory |
| `rename(from, to)` | move / rename |
| `unlink(path)` | delete |
| `mktemp(prefix) -> String` | make a temp file |
| `list_dir(path) -> ...` | enumerate entries |
| `file_exists(path) -> Bool` | test (never fails) |

## The error tells you what happened

When a call fails, the `IoError` payload carries a `kind`
(`String`), the raw `errno` (`Int`), and the `path` (`String`).
`kind` is a stable tag derived from the OS error —
`"not_found"`, `"permission_denied"`, `"already_exists"`,
`"is_dir"`, and so on. So you can branch on the *kind* of
failure without parsing error strings:

```hale
fn handle_io(e: IoError) -> String {
    if e.kind == "not_found" {
        return "";                     // treat missing as empty
    }
    eprintln("io error on ", e.path, ": ", e.kind);
    return "";
}

fn load(path: String) -> String {
    return std::io::fs::read_file(path) or handle_io(err);
}
```

This is the `or handler(err)` motion from the basics, put to
work: one recovery function shared across every read.

## Idempotent setup

`or discard` is handy for "make sure this exists; don't care if
it already did" — it's allowed because the result type is `()`:

```hale,fragment
std::io::fs::mkdir("cache") or discard;
```

## Compression and archives

`std::compress` and `std::tar` work over `Bytes` in memory, so
building a `.tar.gz` is a pipeline of ordinary calls:

```hale
fn ship(report: Bytes) fallible(IoError) {
    let a = std::tar::pack(std::bytes::from_string(""),
        "out/report.json", report) or raise;
    let archive = std::tar::finish(a) or raise;
    let tgz = std::compress::gzip(archive) or raise;
    std::io::fs::write_bytes("report.tar.gz", tgz) or raise;
}
```

Reading goes the other way: `read_bytes` → `gunzip` →
`tar::entries` / `entry_name` / `entry_data` to walk what's
inside. `zstd` / `unzstd` are the faster-newer siblings — they
load libzstd at first use, and fail with `kind == "not_found"`
on a machine that doesn't have it (gzip always works). Corrupt
or truncated input fails with `kind == "invalid"` rather than
crashing, and decompression refuses to expand past 1 GiB in one
shot — a zip bomb is an error, not an OOM.

## Held-open files

`read_file` / `write_file` are whole-file, one-shot. When you
want a file *handle* you read from incrementally — line by line,
or seeking around — use `std::io::file::File`, a locus that holds
the open descriptor for its lifetime:

```hale,fragment
let f = std::io::file::open("log.txt", "r") or raise;
let line = f.read_line() or "";
// ... f closes when it goes out of scope
```

That "closes when it goes out of scope" is the locus lifecycle
quietly at work — `f` owns the descriptor and releases it when
its binding's scope ends. You'll see that mechanism in full at
the services level; here it just means you don't write a manual
`close`.

Next: structured data on disk and the wire — [JSON](./json.md).
