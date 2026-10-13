# Hashing, encoding & randomness

A grab-bag of the cryptographic and byte-wrangling helpers real
programs reach for: digests, message authentication, base64 and hex, and
random numbers. They live under `std::crypto`, `std::text`,
`std::hex`, and `std::rand` / `std::os`.

## Hashes & checksums — `std::crypto`

Digests take `Bytes` and return `Bytes`:

```hale,fragment
let data = std::bytes::from_string("hello");
let key  = std::bytes::from_string("secret");

let digest = std::crypto::sha256(data);            // 32 bytes
let tag    = std::crypto::hmac_sha256(key, data);  // 32 bytes
let sum    = std::crypto::crc32(data);             // Int (IEEE 802.3 / zlib)

let digest512 = std::crypto::sha512(data);            // 64 bytes
let tag512    = std::crypto::hmac_sha512(key, data);  // 64 bytes
```

`sha1` (20 bytes) is there too for legacy interop; reach for
`sha256` by default. The 64-bit-word SHA-512 siblings —
`sha512` / `hmac_sha512` (64-byte) — are the same non-fallible
shape, for services that sign requests with HMAC-SHA512. The
hashes and `crc32` are hand-rolled — no OpenSSL
dependency.

Raw hash bytes aren't printable, so encode one to show or
transport it:

```hale,fragment
println(std::text::base64::encode(digest));
println(std::hex::encode(digest));   // 64 lowercase hex digits
```

## Hex — `std::hex`

```hale,fragment
let text  = std::hex::encode(digest);            // Bytes -> String, lowercase
let back  = std::hex::decode(text) or raise;     // String -> Bytes, either case
let value = std::hex::digit(std::str::byte_at_unchecked("f", 0));  // 15
```

`decode` is strict: two digits a byte, no whitespace, no `0x`. An
odd length or a byte that is not a hex digit is a `ParseError`
(kind `"hex"`, carrying the input), so it takes an `or` like
`parse_int`. `digit` is the value of one digit byte, or `-1`.

## Base64 — `std::text::base64`

```hale,fragment
let enc = std::text::base64::encode(data);     // Bytes -> String
let dec = std::text::base64::decode(enc);      // String -> Bytes
let url = std::text::base64::url_encode(data);  // URL-safe, unpadded
```

`url_encode` is RFC 4648 §5 (the `-_` alphabet, no `=` padding) —
the form JWTs, OAuth, and webhook signatures use. `decode` reads
the standard, padded alphabet only (`+/`, `=`): it skips whitespace
and returns an empty `Bytes` for anything else — `url_encode`'s
output included. There is no URL-safe decoder.

## Signing — ECDSA P-256 (`ES256`)

For JWT / API-request signing, `std::crypto` ships ECDSA over NIST P-256
with SHA-256 (the `ES256` JWS algorithm), OpenSSL-backed:

```hale,fragment
// key: PEM EC private key (SEC1 or PKCS#8); message: Bytes
let sig = std::crypto::ecdsa_p256_sign(key, message) or raise;

// pubkey: PEM SPKI; sig: raw r‖s, 64 bytes (the JWS/COSE form)
let ok = std::crypto::ecdsa_p256_verify(pubkey, message, sig);
```

`ecdsa_p256_sign` can fail (a bad key, a curve other than P-256):
it is `fallible(CryptoError)`, so `or raise` / `or fail err` /
`or handle(err)` propagate a structured `CryptoError { kind,
detail }` like any other error, and `or b""` takes an empty
`Bytes` instead. A bare call is a compile error.

## Random numbers — `std::rand` and `std::os`

`std::rand` is a fast, **non-cryptographic** PRNG — fine for
jitter, sampling, shuffling, game logic:

```hale,fragment
let roll = std::rand::next_int(6) + 1;   // a die roll, [1, 6]
```

When a run has to be repeatable — a simulation, a fuzz case, a test
that shuffles — give it a generator of its own. A `std::rand::Rng`
with the same `seed` gives the same sequence on every run and every
target, and shares nothing with the global generator or another
`Rng`:

```hale,fragment
let rng  = std::rand::Rng { seed: 42 };
let roll = rng.range(1, 7);        // [1, 7): a die roll
let pick = rng.next_int(10);       // [0, 10)
let p    = rng.next_float();       // [0.0, 1.0)
```

It is SplitMix64 — fast and statistically sound, and just as
**non-cryptographic** as `next_int`.

For anything security-sensitive (tokens, nonces, keys), use the
CSPRNG instead:

```hale,fragment
let nonce = std::os::getrandom(16) or raise;   // 16 random Bytes
```

Next: reading configuration — [CLI & config](./cli-config.md).
