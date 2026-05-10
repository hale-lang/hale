# `Bytes`

The binary-safe sibling of `String`. m89 ships the type plus
two production paths (`read_bytes`, `send_bytes`) and one
inspection primitive (`len`).

`Bytes` exists because Aperio Strings are NUL-terminated in
memory: any payload containing an embedded `\x00` silently
truncates if shipped through the String surface. `Bytes`
sidesteps that by storing an explicit length prefix.

## Memory representation

A `Bytes` value is a single pointer (same ABI as `String`)
into a `[i64 len][u8 data[len]]` blob in the caller's arena.
The length is fetched via the prefix, so embedded NULs are
fine.

A returned `Bytes` is allocated in the lazy global payload
arena, so it outlives the call frame. This mirrors the m70
deserialize convention used by `read_file` for Strings.

## Producing a Bytes

The shipped sources of `Bytes` values:

- **`std::io::fs::read_bytes(path: String) -> Bytes`** — reads
  a file as raw bytes, length-preserved. Returns a zero-length
  `Bytes` on error (missing file, etc.). m89.
- **`Stream.recv_bytes(n)`** — *not yet shipped.* The Stream
  surface today returns `String` from `recv`; binary-safe
  receive is a follow-up. Use `read_bytes` for now if you need
  binary-safe input.

### Bytes literals

The lexer recognizes `b"..."` literals (per
`spec/tokens.md`), but codegen lowering for them is **not
shipped**. Writing `let b = b"abc";` will fail at compile
time. Use `read_bytes` to get a real `Bytes` value.

This will land in a future milestone alongside the other
gaps (Bytes in struct fields, Bytes equality).

## Consuming a Bytes

- **`len(b: Bytes) -> Int`** — number of bytes in the blob.
  Bare-name builtin (no `std::*` path).
- **`Stream.send_bytes(b: Bytes)`** — sends the blob's bytes
  through a TCP Stream. Length-preserving — embedded NULs
  survive. m89.
- **`println(..., b)`** — prints `<bytes len=N>` rather than
  the byte content, so logs stay readable when binary data is
  involved.

## Examples

Read a binary file and report its length:

```aperio
fn main() {
    let b = std::io::fs::read_bytes("photo.jpg");
    println("read ", len(b), " bytes");
}
```

Serve a binary asset over HTTP — `Stream.send_bytes` for the
body so embedded NULs in (e.g.) a PNG don't truncate:

```aperio
fn handle(s: std::io::tcp::Stream) {
    let body = std::io::fs::read_bytes("logo.png");

    let header = "HTTP/1.1 200 OK\r\n"
               + "Content-Type: image/png\r\n"
               + "Content-Length: " + to_string(len(body)) + "\r\n"
               + "Connection: close\r\n\r\n";
    s.send(header);
    s.send_bytes(body);
}
```

(Note: `std::http::write_response` currently takes a String
body, so binary-content responses route through raw
`Stream.send` + `Stream.send_bytes` until a `Bytes`-aware
response writer ships. Phase 3 v1.0 follow-up.)

Round-trip a binary file with embedded NULs:

```aperio
fn main() {
    // Bytes returned from read_bytes preserves all bytes
    // including embedded NULs. The same payload through
    // String would truncate at the first NUL.
    let b = std::io::fs::read_bytes("data.bin");
    let n = len(b);
    println("loaded ", n, " bytes");
}
```

## Limitations (m89)

- **No `Bytes` literals.** `b"..."` parses but does not
  codegen. Use `read_bytes`.
- **No `Bytes` indexing or slicing.** Strings support
  `s[start..end]`; Bytes does not. Pending milestone.
- **No `Bytes` ↔ `String` conversion in source.** If you have
  a `Bytes` and know it's UTF-8, there's no `to_string(b)`
  path that gives you a `String` — the bare-name `to_string`
  on `Bytes` prints the `<bytes len=N>` summary instead. Read
  with `read_file` if you want a `String`, with `read_bytes`
  if you want a `Bytes`.
- **No `Bytes` in struct fields, no `Bytes` arrays** — the
  field-storage / array-element story for `Bytes` is not
  implemented yet.
- **Println summary not actual data.** `println(b)` prints
  `<bytes len=N>`. Inspect raw bytes with platform tools
  after writing them out via `write_file` / `send_bytes`.

## See Also

- [`std::io::fs`](./io/fs.md) — `read_bytes` lives here.
- [`std::io::tcp`](./io/tcp.md) — `Stream.send_bytes` ships
  raw bytes over TCP.
- [Roadmap](./roadmap.md) — `Bytes` literals and full surface
  parity with `String` are tracked under language paper-cuts.
