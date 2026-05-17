# Read & write files

`std::io::fs::*` is the filesystem surface. Every operation
except `file_exists` returns `fallible(IoError)` — the call
site has to address the failure with an `or` clause before the
typechecker will let you consume the value. This page covers
the filesystem surface; for the full `or`-clause vocabulary
(`or raise` / substitute / `or self.handler(err)` / `or fail` /
`or discard`) see [Error handling](../concepts/error-handling.md)
§ "The value channel".

## The surface

| Call | Returns | Notes |
|---|---|---|
| `read_file(p)` | `String fallible(IoError)` | UTF-8 text |
| `read_bytes(p)` | `Bytes fallible(IoError)` | binary |
| `write_file(p, s)` | `() fallible(IoError)` | overwrites |
| `write_file_append(p, s)` | `() fallible(IoError)` | appends |
| `file_size(p)` | `Int fallible(IoError)` | bytes |
| `mkdir(p)` | `() fallible(IoError)` | parents must exist |
| `rename(src, dst)` | `() fallible(IoError)` | POSIX `rename(2)`; atomic on same fs, EXDEV across |
| `unlink(p)` | `() fallible(IoError)` | removes a regular file or symlink |
| `mktemp(prefix, suffix)` | `String fallible(IoError)` | race-free `mkstemps(3)`; assembles `prefix + "XXXXXX" + suffix`, returns the path; caller owns cleanup |
| `file_exists(p)` | `Bool` | **NOT fallible** — predicate |
| `list_dir_count(p)` | `Int fallible(IoError)` | entry count |
| `list_dir_at(p, i)` | `String fallible(IoError)` | i-th entry name |

`IoError` carries:

- `kind: String` — `"not_found"`, `"permission_denied"`,
  `"is_dir"`, `"already_exists"`, `"broken_pipe"`, etc.
  (errno-derived; `"io"` is the catch-all.)
- `errno: Int` — raw platform errno.
- `path: String` — the file path the call was made against.

## A worked example: copy + count

A small CLI that reads every `.md` file in a directory, counts
total bytes, and writes the count to `out.txt`. Every fallible
call is addressed; one helper propagates with `or raise`.

```aperio
fn count_markdown(dir: String) -> Int fallible(IoError) {
    let count = std::io::fs::list_dir_count(dir) or raise;
    let mut total = 0;
    let mut i = 0;
    while i < count {
        let name = std::io::fs::list_dir_at(dir, i) or raise;
        if std::str::index_of(name, ".md") > 0 {
            let path = dir + "/" + name;
            let sz   = std::io::fs::file_size(path) or 0;
            total = total + sz;
        }
        i = i + 1;
    }
    return total;
}

locus App {
    params { dir: String = "."; }

    fn handle_io(e: IoError) -> Int {
        eprintln("count failed at ", e.path, ": ", e.kind);
        return -1;
    }

    run() {
        let total = count_markdown(self.dir) or self.handle_io(err);
        if total < 0 { return; }
        std::io::fs::write_file("out.txt", f"total bytes: {to_string(total)}\n")
            or discard;
        println("counted ", to_string(total), " bytes in ", self.dir);
    }
}

fn main() {
    let dir = std::env::arg_or(1, ".");
    App { dir: dir };
}
```

## Why every call is fallible

Every filesystem call can fail for reasons outside the
caller's control: a directory disappears between
`list_dir_count` and `list_dir_at`; permissions change; the
disk fills. The two-channel rule (see
[Error handling](../concepts/error-handling.md)) puts these
on the value channel because the caller — not the parent
locus — is the right place to decide what to do (retry,
skip, escalate).

If you find yourself writing `or raise` on every line, your
function probably wants to be the propagation boundary —
declare it `-> T fallible(IoError)` and let the wrapper choose
the policy.

## See also

- [Error handling](../concepts/error-handling.md) — the full
  `or` vocabulary (the five motions), the structural channel,
  and the rules for bridging between them.
- [Read & write JSON](./json.md) — for parsing the strings
  `read_file` returns.
- [Standard library](../reference/stdlib.md#path-call-dispatch) —
  the canonical surface listing.
