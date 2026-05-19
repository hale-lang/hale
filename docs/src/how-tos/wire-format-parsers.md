# Write a wire-format parser

The Aperio stdlib ships flat-JSON (`std::json`) but nothing
binary-shaped — there's no protobuf parser, no MsgPack codec, no
RFC 4180 CSV. When you need to ingest someone else's wire format
(market-data feeds, IoT framing, a custom binary protocol, etc.),
you write the parser in Aperio against the byte-walking
primitives in `std::bytes` and the string helpers in `std::str`.

This page walks four common framing patterns. Each one is a
working sketch — adapt the field types and offsets to your wire.

## Substrate

Every parser combines three primitives:

| What you need              | Surface                                                                              |
| -------------------------- | ------------------------------------------------------------------------------------ |
| Per-byte access            | `std::bytes::at(b: Bytes, i: Int) -> Int fallible(IndexError)`                       |
| Bytes ↔ String conversion  | `std::bytes::from_string` / `std::str::from_bytes`                                   |
| Length                     | `len(b)` polymorphic over `Bytes` / `String` / `@form(vec)` cells                    |
| Substrings                 | `s[lo..hi]` slice (no copy semantics in v1; treat as read-only view)                 |
| Index search               | `std::str::index_of(s: String, needle: String) -> Int` (returns -1 if absent)        |
| Parse a number             | `std::str::parse_int(s) or substitute(err)` / `std::str::parse_float(s) or ...`      |

For most wire formats you'll wrap `std::bytes::at` in a
one-liner that swallows the out-of-bounds error so a "default
zero on read past end" feels native:

```aperio
fn byte_at(b: Bytes, i: Int) -> Int {
    return std::bytes::at(b, i) or 0;
}
```

The guard rail then lives in your explicit length checks
(`if off + N > total { fail BadFrame { ... } }`), not in the
read itself. Same shape `pond/trade/marketdata/itch.ap` uses
for its ITCH 5.0 parser; reach there for a worked-out example.

## Pattern 1 — Line-delimited records

Newline-separated text records (CSV-style, log lines, RFC 5424
syslog). Walk the body, slice at every `\n`, hand each line to a
record parser.

```aperio
fn drive_lines(body: String, sink: SinkLocus) {
    let total = len(body);
    let mut cursor = 0;
    while cursor < total {
        let rel = std::str::index_of(body[cursor..total], "\n");
        let mut line_end = total;
        if rel >= 0 {
            line_end = cursor + rel;
        }
        let line = body[cursor..line_end];
        if len(line) > 0 {
            sink.on_record(parse_record(line));
        }
        if rel < 0 {
            cursor = total;
        } else {
            cursor = line_end + 1;
        }
    }
}
```

`pond/trade/backtest/feed.ap` is a worked example: it loads a CSV
file with `std::io::fs::read_file`, scans newlines once into an
offsets table, then `row_at(i)` does an O(1) slice + parse.

## Pattern 2 — Length-prefix framing

A 4-byte big-endian length, then `length` bytes of payload, then
the next length, and so on. Every binary-shaped protocol in the
wild does some flavor of this (Postgres wire, Redis RESP3
multi-bulk, ITCH 5.0, every websocket subprotocol).

```aperio
fn drive_lp(b: Bytes, sink: SinkLocus) {
    let total = len(b);
    let mut off = 0;
    while off + 4 <= total {
        let msg_len = read_be_u32(b, off);
        let payload_off = off + 4;
        if payload_off + msg_len > total {
            return;       // partial frame at tail; caller buffers
        }
        sink.on_frame(b, payload_off, msg_len);
        off = payload_off + msg_len;
    }
}

fn read_be_u32(b: Bytes, off: Int) -> Int {
    let a  = byte_at(b, off);
    let b1 = byte_at(b, off + 1);
    let c  = byte_at(b, off + 2);
    let d  = byte_at(b, off + 3);
    return a * 16777216 + b1 * 65536 + c * 256 + d;
}
```

Two things to watch:

1. **Always range-check before reading.** Aperio's `std::bytes::at`
   is fallible per-byte; the `or 0` shim is for read-past-end
   safety, but you don't want to *use* that zero as data. The
   `payload_off + msg_len > total` guard is the rail.
2. **Big-endian for network, little-endian for most domestic
   protocols.** `read_be_u32` above is big-endian. Flip the
   multiplications for little-endian:

   ```aperio
   return a + b1 * 256 + c * 65536 + d * 16777216;
   ```

## Pattern 3 — Framed JSON (newline-delimited or length-prefixed)

A common shape for "JSON but streamable": each message is a
length-prefix or newline-delimited JSON object. You combine
pattern 1 or 2 with `std::json`.

```aperio
fn drive_ndjson(body: String, sink: SinkLocus) {
    let total = len(body);
    let mut cursor = 0;
    while cursor < total {
        let rel = std::str::index_of(body[cursor..total], "\n");
        let mut end = total;
        if rel >= 0 {
            end = cursor + rel;
        }
        let frame = body[cursor..end];
        if len(frame) > 0 {
            let name  = std::json::find_string_field(frame, "name");
            let value = std::json::find_int_field(frame, "value");
            sink.on_record(name, value);
        }
        if rel < 0 {
            cursor = total;
        } else {
            cursor = end + 1;
        }
    }
}
```

The `std::json` reader is flat — single-level objects only. If
your protocol nests, you handle the nesting layer yourself (slice
out the substring, then re-enter `std::json`). For top-level
arrays use `std::json::array_first` + `array_next` from
[`json.md`](./json.md).

## Pattern 4 — Partial-read accumulator

When you're reading from a socket and frames arrive split across
`recv` calls, the parser needs to *resume*. The canonical shape
is a held-open buffer on the locus that accumulates inbound
bytes; the drive loop reads as many complete frames as it can,
keeps the tail.

```aperio
locus Reader {
    params {
        sock: Int     = -1;
        buf:  Bytes;
    }

    fn pump() {
        let chunk = std::io::tcp::recv_bytes(self.sock, 4096)
            or self.handle_recv(err);
        if len(chunk) == 0 {
            return;                  // EOF or err — birth state
        }
        self.buf = std::bytes::concat(self.buf, chunk);

        // Consume as many length-prefix frames as the buffer holds.
        let mut off = 0;
        let total = len(self.buf);
        while off + 4 <= total {
            let n = read_be_u32(self.buf, off);
            let payload_off = off + 4;
            if payload_off + n > total {
                break;                // partial frame — keep tail
            }
            self.handle_frame(self.buf, payload_off, n);
            off = payload_off + n;
        }
        // Slice off the consumed prefix; tail carries to next pump.
        if off > 0 {
            self.buf = std::bytes::slice(self.buf, off, total);
        }
    }
}
```

The pattern: each `pump()` extends the buffer, peels complete
frames, leaves the partial tail in `self.buf`. The next call
picks up where this one left off. `pond/websocket` and any TLS-
record-walking parser look exactly like this — see
[`std::io::tcp`](../reference/stdlib.md) and `std::io::tls` for
the socket surface.

## When to reach for a contrib library

Wire-format parsers are not one-size-fits-all — RFC 6455
(websocket) layers control-frame masking + fragmentation on top
of length-prefix framing, RFC 4180 (CSV) has corner cases around
embedded quotes that a naive line scanner misses. The pond contrib
catalog already ships parsers for:

- **ITCH 5.0 market-data**: `pond/trade/marketdata` (`ItchParser`)
- **HTTP/1.1**: `pond/http/client` (request builder + response
  reader)
- **ANSI SQL**: `pond/sqlite` (round-tripped through libsqlite,
  not parsed in Aperio)

For anything bespoke or one-off, the patterns above are enough
substrate to write the parser inline. The stdlib stays small on
purpose; cross-cutting protocols live in pond/.
