# runtime/stdlib — where the stdlib lives (and where it doesn't)

The `.hl` files in this directory are the stdlib modules written
in Hale itself. **They are not the whole stdlib.** A number of
namespaces exist only as compiler builtins — lowered directly in
`crates/hale-codegen/src/` (the `["std", "<ns>", "<fn>"]` match
arms, mostly `codegen.rs` and `src/stdlib/*.rs`) over C
primitives in `../lotus_arena.c` / `../lotus_tls.c` — and have
**no `.hl` file here**. Grepping this directory for them finds
nothing, which has repeatedly read as "doesn't exist" downstream
(Crumb batch-4 reported `std::crypto` missing this way; the same
wall was hit earlier for `std::str`).

Builtin-only namespaces (no `.hl` in this directory):

| namespace | what | implementation |
|---|---|---|
| `std::crypto` | sha1/sha256/sha512, hmac_sha256/512, crc32, ecdsa_p256_sign/verify | C in `lotus_arena.c` (hashes, hand-rolled) + `lotus_tls.c` (ECDSA, OpenSSL); lowering in `src/stdlib/crypto.rs` |
| `std::str` | index_of, substring, byte_at_unchecked, range_* family, parse_int/float, from_bytes, … | C + codegen arms |
| `std::math` | sqrt, pow, trig, floor/ceil/round, int↔float | libm + codegen arms |
| `std::time` | sleep, monotonic, monotonic_ns, now, mock_clock | C + codegen (`src/stdlib/time.rs`) |
| `std::bytes` | from_string, read_*/append_* binary pack | C + codegen |
| `std::text::base64` | encode/decode (+ url variants) | C in `lotus_tls.c` |
| `std::rand` | random numbers | C + codegen |
| `std::env` / `std::os` / `std::diag` | args, env vars, platform, diag counters | C + codegen |
| `std::decimal` | Decimal helpers | codegen (i128 mantissa ops) |
| `std::compress` / `std::tar` | gzip/deflate, ustar | C (zlib) + codegen |
| `std::io::fs` / `std::io::stdin` / `std::io::stdout` | filesystem + stdio | C + codegen |
| `std::io::tls` | TLS client (connect/upgrade/send/recv) | C in `lotus_tls.c` |
| `std::ring` | SPSC ring primitive | C + codegen |

The authoritative name list for *every* namespace — `.hl`-backed
or builtin — is `crates/hale-types/src/stdlib_surface.rs`, and
`hale doc --stdlib` renders the whole surface with signatures.
The user-facing contract lives in `spec/stdlib.md`.

If you add a builtin-only namespace, add a row here.
