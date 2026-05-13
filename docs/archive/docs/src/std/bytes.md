# `Bytes`

The binary-safe sibling of `String`. m89 shipped the type plus
`read_bytes` / `send_bytes` / `len`. Phase 2g (2026-05-11)
filled in the rest: binary-safe receive, byte-level inspection,
slice, and explicit Bytes ↔ String conversions.

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
- **`Stream.recv_bytes(max: Int) -> Bytes`** — Phase 2g —
  reads up to `max` bytes from a connected TCP fd into a
  length-prefixed blob. Embedded NULs survive, unlike `recv`'s
  String shape. Returns a zero-length `Bytes` on EOF or read
  error.
- **`std::bytes::from_string(s: String) -> Bytes`** — Phase 2g
  — copies the source string's bytes (strlen-measured) into a
  fresh length-prefixed blob. The inverse of
  `std::str::from_bytes`.
- **`std::bytes::slice(b: Bytes, lo: Int, hi: Int) -> Bytes`**
  — Phase 2g — half-open range copy `[lo, hi)`. Out-of-range
  bounds clamp to the source length; `hi <= lo` yields an
  empty `Bytes`. The result is a copy, not a view, so it
  composes with deep-copy lifetime conventions.

### Bytes literals

The lexer recognizes `b"..."` literals (per
`spec/tokens.md`), but codegen lowering for them is **not
shipped**. Writing `let b = b"abc";` will fail at compile
time. Use `read_bytes` or `std::bytes::from_string("...")`
to get a real `Bytes` value.

`b"..."` literal codegen lands in a future milestone alongside
the other gaps (Bytes in struct fields, Bytes equality).

## Consuming a Bytes

- **`len(b: Bytes) -> Int`** — number of bytes in the blob.
  Bare-name builtin (no `std::*` path).
- **`std::bytes::at(b: Bytes, i: Int) -> Int`** — Phase 2g —
  byte-as-Int accessor; returns the i-th byte's unsigned value
  (0..255). Out-of-range (i < 0 or i >= len) returns -1 as a
  clean sentinel. Use this for byte-level protocol parsing
  (WebSocket frame headers, framing length fields, etc.).
- **`Stream.send_bytes(b: Bytes)`** — sends the blob's bytes
  through a TCP Stream. Length-preserving — embedded NULs
  survive. m89.
- **`std::str::from_bytes(b: Bytes) -> String`** — Phase 2g —
  copies the body into a NUL-terminated String. Embedded NULs
  persist in the buffer but the resulting String's strlen-based
  view will truncate at the first one — callers who need
  NUL-safe handling should stay in Bytes.
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

Parse a two-byte frame header on a TCP Stream — the
WebSocket / HTTP/2 / custom-RPC shape:

```aperio
fn read_frame_header(s: std::io::tcp::Stream) {
    let hdr = s.recv_bytes(2);
    if len(hdr) < 2 {
        println("short read");
        return;
    }
    let b0 = std::bytes::at(hdr, 0);
    let b1 = std::bytes::at(hdr, 1);
    let fin = b0 / 128;           // top bit of byte 0
    let opcode = b0 % 16;         // low nibble
    let masked = b1 / 128;        // top bit of byte 1
    let len7 = b1 % 128;          // low 7 bits
    println("fin=", fin, " opcode=", opcode,
            " masked=", masked, " len=", len7);
}
```

Round-trip a String through Bytes (e.g. to ship a known-text
payload via the binary-safe `send_bytes` surface):

```aperio
fn main() {
    let body = std::bytes::from_string("hello world");
    println("blen=", len(body));
    let back = std::str::from_bytes(body);
    println("back=", back);
}
```

## Limitations

- **No `Bytes` literals.** `b"..."` parses but does not
  codegen. Use `read_bytes`, `recv_bytes`, or
  `std::bytes::from_string`.
- **No `Bytes` equality / hashing.** Equality is not yet
  shipped — compare via `std::bytes::at` byte-by-byte for now.
- **No `Bytes` in struct fields, no `Bytes` arrays** — the
  field-storage / array-element story for `Bytes` is not
  implemented yet.
- **Println summary not actual data.** `println(b)` prints
  `<bytes len=N>`. Inspect raw bytes with platform tools
  after writing them out via `write_file` / `send_bytes`.

## See Also

- [`std::io::fs`](./io/fs.md) — `read_bytes` lives here.
- [`std::io::tcp`](./io/tcp.md) — `Stream.send_bytes` /
  `Stream.recv_bytes` ship raw bytes over TCP.
- [`std::str`](./str.md) — `from_bytes` for the inverse
  conversion.
- [Roadmap](./roadmap.md) — `Bytes` literals, equality, and
  field-storage parity with `String` are tracked under language
  paper-cuts.
