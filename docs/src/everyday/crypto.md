# Hashing, encoding & randomness

A grab-bag of the cryptographic and byte-wrangling helpers real
programs reach for: digests, message authentication, base64, and
random numbers. They live under `std::crypto`, `std::text`, and
`std::rand` / `std::os`.

## Hashes & checksums — `std::crypto`

Digests take `Bytes` and return `Bytes`:

```hale
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
shape, for venues that sign with HMAC-SHA512 (e.g. Kraken,
Gate.io). The hashes and `crc32` are hand-rolled — no OpenSSL
dependency.

Raw hash bytes aren't printable, so encode one to show or
transport it:

```hale
println(std::text::base64::encode(digest));
```

## Base64 — `std::text::base64`

```hale
let enc = std::text::base64::encode(data);     // Bytes -> String
let dec = std::text::base64::decode(enc);      // String -> Bytes
let url = std::text::base64::url_encode(data);  // URL-safe, unpadded
```

`url_encode` is RFC 4648 §5 (the `-_` alphabet, no `=` padding) —
the form JWTs, OAuth, and webhook signatures use. `decode` accepts
both alphabets.

## Signing — ECDSA P-256 (`ES256`)

For JWT / venue auth, `std::crypto` ships ECDSA over NIST P-256
with SHA-256 (the `ES256` JWS algorithm), OpenSSL-backed:

```hale
// key: PEM EC private key (SEC1 or PKCS#8); message: Bytes
let sig = std::crypto::ecdsa_p256_sign(key, message) or raise;

// pubkey: PEM SPKI; sig: raw r‖s, 64 bytes (the JWS/COSE form)
let ok = std::crypto::ecdsa_p256_verify(pubkey, message, sig);
```

`ecdsa_p256_sign` has two faces: a bare call returns an empty
`Bytes` on failure (check `len(sig) == 0`), and in an `or` context
it is `fallible(CryptoError)`, so `or raise` / `or fail err` /
`or handle(err)` propagate a structured `CryptoError { kind,
detail }` like any other error.

## Random numbers — `std::rand` and `std::os`

`std::rand` is a fast, **non-cryptographic** PRNG — fine for
jitter, sampling, shuffling, game logic:

```hale
let roll = std::rand::next_int(6) + 1;   // a die roll, [1, 6]
```

For anything security-sensitive (tokens, nonces, keys), use the
CSPRNG instead:

```hale
let nonce = std::os::getrandom(16) or raise;   // 16 random Bytes
```

Next: reading configuration — [CLI & config](./cli-config.md).
