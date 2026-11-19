# Hashing, encoding & randomness

Hale provides cryptographic digests, HMAC authentication, digital signatures (ECDSA P-256, RSA-PSS, Ed25519 and secp256k1), hex and Base64 codecs, and random number generation across `std::crypto`, `std::hex`, `std::text::base64`, `std::os`, and `std::rand`.

```hale
fn main() {
    let data = std::bytes::from_string("hello");
    let key  = std::bytes::from_string("secret");

    let digest = std::crypto::sha256(data);            // 32 Bytes
    let tag    = std::crypto::hmac_sha256(key, data);  // 32 Bytes

    println(std::hex::encode(digest));                 // 64 lowercase hex digits
    println(std::text::base64::encode(tag));           // padded Base64 string
}
```

---

## API Reference

| Function / Type | Signature | Description |
|---|---|---|
| `std::crypto::sha256` | `fn(Bytes) -> Bytes` | 32-byte SHA-256 digest (built-in, no OpenSSL dependency) |
| `std::crypto::Sha256` | `Sha256 { }`: `update(Bytes)`, `finish() -> Bytes` | SHA-256 fed in pieces: the same 32 bytes as `sha256` of the pieces joined, holding a fixed 112-byte state |
| `std::crypto::sha256_file` | `fn(String) -> Bytes fallible(IoError)` | 32-byte SHA-256 of a file, read 1 MiB at a time |
| `std::crypto::sha512` | `fn(Bytes) -> Bytes` | 64-byte SHA-512 digest (built-in, no OpenSSL dependency) |
| `std::crypto::sha1` | `fn(Bytes) -> Bytes` | 20-byte SHA-1 digest for legacy protocol interop (built-in) |
| `std::crypto::hmac_sha256` | `fn(Bytes, Bytes) -> Bytes` | 32-byte HMAC-SHA256 authentication tag (`(key, data)`) |
| `std::crypto::hmac_sha384` | `fn(Bytes, Bytes) -> Bytes` | 48-byte HMAC-SHA384 authentication tag (`(key, data)`) |
| `std::crypto::hmac_sha512` | `fn(Bytes, Bytes) -> Bytes` | 64-byte HMAC-SHA512 authentication tag (`(key, data)`) |
| `std::crypto::keccak256` | `fn(Bytes) -> Bytes` | 32-byte Keccak-256 digest, Ethereum's (Keccak padding, not SHA3-256) |
| `std::crypto::crc32` | `fn(Bytes) -> Int` | IEEE 802.3 / zlib CRC-32 checksum |
| `std::crypto::ecdsa_p256_sign` | `fn(Bytes, Bytes) -> Bytes fallible(CryptoError)` | Signs `(pem_key, message)` using ECDSA P-256 + SHA-256 (`ES256`, 64-byte `r \|\| s`, OpenSSL) |
| `std::crypto::ecdsa_p256_verify` | `fn(Bytes, Bytes, Bytes) -> Bool` | Verifies `(pem_pubkey, message, sig)` for a 64-byte `ES256` signature |
| `std::crypto::ed25519_keypair` | `fn(Bytes) -> (Bytes, Bytes) fallible(CryptoError)` | Derives `(public: 32B, secret: 64B)` from a 32-byte seed (Monocypher, `wasm32`-compatible) |
| `std::crypto::ed25519_sign` | `fn(Bytes, Bytes) -> Bytes fallible(CryptoError)` | Deterministically signs `(secret_or_seed, message)` -> 64-byte Ed25519 signature |
| `std::crypto::ed25519_verify` | `fn(Bytes, Bytes, Bytes) -> Bool` | Verifies `(public, message, sig)` for a 64-byte Ed25519 signature |
| `std::crypto::rsa_pss_sha256_sign` | `fn(Bytes, Bytes) -> Bytes fallible(CryptoError)` | Signs `(pem_key, message)` with RSA-PSS, SHA-256, MGF1-SHA256, 32-byte salt (`PS256`, OpenSSL) |
| `std::crypto::rsa_pss_sha256_verify` | `fn(Bytes, Bytes, Bytes) -> Bool` | Verifies `(pem_pubkey, message, sig)` for an `rsa_pss_sha256_sign` signature |
| `std::crypto::secp256k1_public_key` | `fn(Bytes) -> Bytes fallible(CryptoError)` | The 65-byte uncompressed public key of a 32-byte secret key |
| `std::crypto::secp256k1_sign` | `fn(Bytes, Bytes) -> (Bytes, Bytes, Int) fallible(CryptoError)` | Signs `(secret, digest32)` -> `(r, s, recovery_id)`: RFC 6979 nonce, low `s` |
| `std::crypto::secp256k1_verify` | `fn(Bytes, Bytes, Bytes) -> Bool` | Verifies `(public, digest32, r \|\| s)`; a high `s` is `false` |
| `std::crypto::secp256k1_recover` | `fn(Bytes, Bytes, Bytes, Int) -> Bytes fallible(CryptoError)` | The 65-byte public key that signed `(digest32, r, s, recovery_id)` (`ecrecover`) |
| `std::hex::encode` | `fn(Bytes) -> String` | Encodes bytes as lowercase hexadecimal (`2` chars per byte) |
| `std::hex::decode` | `fn(String) -> Bytes fallible(ParseError)` | Strictly decodes hex string (case-insensitive, no `0x` or whitespace) |
| `std::hex::digit` | `fn(Int) -> Int` | Converts an ASCII hex byte to `0..15`, or `-1` if not a hex digit |
| `std::text::base64::encode` | `fn(Bytes) -> String` | Standard RFC 4648 Base64 encoding (`+/` alphabet, `=` padding) |
| `std::text::base64::decode` | `fn(String) -> Bytes` | Decodes standard padded Base64 (skips whitespace; returns `b""` on invalid input) |
| `std::text::base64::url_encode` | `fn(Bytes) -> String` | RFC 4648 §5 URL-safe Base64 encoding (`-_` alphabet, unpadded) |
| `std::os::getrandom` | `fn(Int) -> Bytes fallible(IoError)` | Cryptographically secure random bytes (CSPRNG) from the OS kernel |
| `std::rand::next_int` | `fn(Int) -> Int` | Fast, non-cryptographic random integer in `[0, n)` |
| `std::rand::Rng` | `Rng { seed: Int }` | Deterministic, isolated SplitMix64 PRNG (`range`, `next_int`, `next_float`) |

---

## Hashes, HMACs & Checksums (`std::crypto`)

Digest and MAC functions take `Bytes` and return `Bytes` (or `Int` for `crc32`). All hash, HMAC, and CRC functions are self-contained in the runtime with no external library dependencies and cannot fail:

```hale,fragment
let data = std::bytes::from_string("hello");
let key  = std::bytes::from_string("secret");

let digest    = std::crypto::sha256(data);            // 32 bytes
let tag       = std::crypto::hmac_sha256(key, data);  // 32 bytes
let sum       = std::crypto::crc32(data);             // Int (IEEE 802.3 / zlib)

let digest512 = std::crypto::sha512(data);            // 64 bytes
let tag512    = std::crypto::hmac_sha512(key, data);  // 64 bytes
let tag384    = std::crypto::hmac_sha384(key, data);  // 48 bytes
let eth_hash  = std::crypto::keccak256(data);         // 32 bytes
```

Use `sha256` / `hmac_sha256` (or `sha512` / `hmac_sha512`, `hmac_sha384` for APIs requiring those tags) for new code. `std::crypto::sha1` (20 bytes) is provided for legacy protocol interoperability.

`keccak256` is the original Keccak-256, the hash Ethereum uses for transaction hashes, addresses and `eth_sign`. It is **not** SHA3-256: FIPS 202 changed the padding byte from `0x01` to `0x06`, so the two answer differently for every input (`keccak256(b"")` is `c5d24601…5d85a470`, SHA3-256's is `a7ffc6f8…80f8434a`).

### Hashing a stream or a large file

`sha256(b)` needs all of `b` in memory. To verify a download against a pinned digest as it arrives, or a file larger than memory, feed a `std::crypto::Sha256` the pieces instead, or hash the file by its path:

```hale,fragment
let h = std::crypto::Sha256 { };
h.update(b"hello, ");
h.update(b"world");
let digest = h.finish();               // == std::crypto::sha256(b"hello, world")

let on_disk = std::crypto::sha256_file("/models/weights.bin") or raise;
```

- `update(piece)` absorbs any number of bytes, an empty piece included; the digest does not depend on how the input was split.
- `finish()` answers the 32-byte digest of everything absorbed since the hasher was made or last finished, and starts it over, so one hasher can serve one input after another.
- The hasher holds a fixed 112-byte state whatever it has absorbed; `sha256_file` holds one 1 MiB read buffer. Hashing a 10 GiB file either way peaked at about 1 to 2 MB above a program that hashes nothing, as fast as `sha256sum`.
- `sha256_file` fails with `IoError` (`not_found`, `permission_denied`, …, the path in `err.path`) when the file cannot be opened or read.
- In a long-lived locus, each piece can be made in the loop body of `run()` itself: a loop iteration frees what it made when it ends, as long as it keeps nothing in a binding declared before the loop ([Memory & Lifetime](../systems/memory.md#per-iteration-loop-regions)).

On x86-64 CPUs with the SHA extensions, the compression runs on them. On aarch64 Linux and macOS (a Grace CPU, Apple Silicon), it runs on the ARMv8 SHA-256 instructions when the OS reports them. Everywhere else, wasm32 included, it runs the portable C rounds (about 350 MB/s on the x86-64 machine measured above). All four entry points (`sha256`, `hmac_sha256`, `Sha256`, `sha256_file`) share the one implementation.

---

## Hex & Base64 Encoding

### Hexadecimal (`std::hex`)

```hale,fragment
let text  = std::hex::encode(digest);                              // Bytes -> String (lowercase)
let back  = std::hex::decode(text) or raise;                       // String -> Bytes (either case)
let value = std::hex::digit(std::str::byte_at_unchecked("f", 0));  // 15
```

- `std::hex::decode` is strict: input must have an even number of hex digits with no whitespace and no `0x` prefix. Odd-length strings or non-hex bytes fail with `ParseError` (`kind == "hex"`, carrying the offending input).
- `std::hex::digit` returns the integer value (`0..15`) of a single ASCII byte, or `-1` if the byte is not a valid hexadecimal digit.

### Base64 (`std::text::base64`)

```hale,fragment
let enc = std::text::base64::encode(data);      // Bytes -> String (standard, padded)
let dec = std::text::base64::decode(enc);       // String -> Bytes
let url = std::text::base64::url_encode(data);  // Bytes -> String (URL-safe, unpadded)
```

- `encode` and `decode` use the standard RFC 4648 alphabet (`+/` with `=` padding). `decode` ignores ASCII whitespace and returns empty `Bytes` (`b""`) for any non-standard input (including `url_encode` output; there is no built-in URL-safe decoder).
- `url_encode` implements RFC 4648 §5 (`-_` alphabet with `=` padding stripped), matching the format required by JWTs, OAuth, and webhook signatures.

---

## Digital Signatures

### ECDSA P-256 / `ES256` (`std::crypto::ecdsa_p256_*`)

For JWT (`ES256`), COSE, and API request signing, `std::crypto` provides OpenSSL-backed ECDSA over NIST P-256 with SHA-256:

```hale,fragment
// key: PEM EC private key (SEC1 or PKCS#8); message: Bytes
let sig = std::crypto::ecdsa_p256_sign(key, message) or raise;

// pubkey: PEM SPKI; sig: raw r||s, 64 bytes (JWS/COSE format)
let ok = std::crypto::ecdsa_p256_verify(pubkey, message, sig);
```

`ecdsa_p256_sign` is `fallible(CryptoError)` and fails with a structured `CryptoError { kind, detail }` if the PEM key cannot be parsed or uses a curve other than P-256.

### RSA-PSS / `PS256` (`std::crypto::rsa_pss_sha256_*`)

Some HTTP APIs sign each request with an RSA private key in PSS mode. `rsa_pss_sha256_sign` is RSASSA-PSS over SHA-256 with MGF1-SHA256 and a 32-byte salt, the parameters of JWS `PS256` and of `openssl dgst -sha256 -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:32`:

```hale,fragment
// key: PEM RSA private key, PKCS#8 ("BEGIN PRIVATE KEY") or PKCS#1 ("BEGIN RSA PRIVATE KEY")
let sig = std::crypto::rsa_pss_sha256_sign(key, message) or raise;   // as long as the modulus

// pubkey: PEM SPKI ("BEGIN PUBLIC KEY")
let ok = std::crypto::rsa_pss_sha256_verify(pubkey, message, sig);
```

- PSS draws a fresh salt for every signature, so signing the same message twice gives two different signatures, and both verify.
- Signing fails with `CryptoError { kind: "rsa_pss_sha256_sign" }` for a PEM it cannot read, an encrypted key (it never prompts for a passphrase), a key that is not RSA, or a modulus too small for the encoding. The detail never quotes the key.
- Verification demands the same parameters, the 32-byte salt included, and is `false` for anything it cannot read.

### Ed25519 (`std::crypto::ed25519_*`)

Ed25519 (RFC 8032) is used by NATS NKeys, SSH keys, and modern JWT/webhook schemes. The implementation uses Monocypher compiled directly into the runtime (no system library dependency, and supported on `--target wasm32`):

```hale,fragment
let seed = std::os::getrandom(32) or raise;                      // 32-byte seed
let (public, secret) = std::crypto::ed25519_keypair(seed) or raise;

let sig = std::crypto::ed25519_sign(secret, message) or raise;   // 64-byte signature
let ok  = std::crypto::ed25519_verify(public, message, sig);     // Bool
```

- **Key formats**: `public` is 32 bytes. `secret` is 64 bytes (`seed || public`), matching the key layout used by libsodium and Go's `crypto/ed25519`.
- **Signing**: `ed25519_sign` accepts either the 64-byte `secret` or the 32-byte `seed` directly, and is deterministic (the same key and message always produce the same 64-byte signature). Passing any other length—or a 64-byte key whose second half does not match the seed's derived public key—raises `CryptoError`.
- **Verification**: `ed25519_verify` returns `false` if the signature, message, public key, or byte lengths are invalid.

### secp256k1 (`std::crypto::secp256k1_*`)

secp256k1 ECDSA is what signs an EVM transaction or message. It signs a 32-byte **digest**, so hash first: an Ethereum transaction hashes its RLP signing payload with `keccak256`.

```hale,fragment
// secret: the 32-byte private key (big-endian), from the vault (std::secret)
let digest = std::crypto::keccak256(signing_payload);
let (r, s, recovery_id) = std::crypto::secp256k1_sign(secret, digest) or raise;

let public = std::crypto::secp256k1_public_key(secret) or raise;              // 65 bytes, 04 || X || Y
let ok     = std::crypto::secp256k1_verify(public, digest, std::bytes::concat(r, s));
let signer = std::crypto::secp256k1_recover(digest, r, s, recovery_id) or raise;  // == public

// An Ethereum address: the last 20 bytes of keccak256 of the key without its 04 byte.
let address = std::bytes::slice(std::crypto::keccak256(std::bytes::slice(public, 1, 65)), 12, 32);
```

- **Deterministic**: the nonce is RFC 6979's, so the same key and digest always give the same `(r, s)`; no entropy is read for it.
- **Low `s`**: `s` is always in the lower half of the curve order (EIP-2), and `recovery_id` (0 or 1) is flipped with it, so it always names the signer. Write `27 + recovery_id` for a legacy `v`, `chain_id * 2 + 35 + recovery_id` for an EIP-155 one, and the id itself as a typed transaction's `y_parity`.
- **Verification** takes the 65-byte uncompressed or 33-byte compressed public key and a 64-byte `r || s`, and returns `false` for a high `s` (its twin `(r, n - s)` signs the same digest, so accepting both makes signatures malleable).
- **Recovery** is Ethereum's `ecrecover`: it accepts ids 0 to 3 (subtract 27 from a legacy `v` first) and, like `ecrecover`, a high `s`.
- **Refusals**: a secret key that is not 32 bytes, is zero or is not below the curve order, a digest that is not 32 bytes, or an `r`, `s` or id that names no point is a `CryptoError` whose `kind` is the function's name. No error detail ever contains key material.

---

## Random Number Generation

### Cryptographic Randomness (`std::os::getrandom`)

For security-sensitive values (cryptographic keys, nonces, salts, session IDs, and tokens), always use the operating system CSPRNG via `std::os::getrandom`:

```hale,fragment
let nonce = std::os::getrandom(16) or raise;   // 16 cryptographically random Bytes
```

### Non-Cryptographic PRNG (`std::rand`)

For jitter, sampling, shuffling, simulations, and property testing, `std::rand` provides a fast SplitMix64 pseudo-random generator:

```hale,fragment
// Global non-cryptographic PRNG
let roll = std::rand::next_int(6) + 1;   // integer in [1, 6]

// Isolated, reproducible PRNG instance
let rng  = std::rand::Rng { seed: 42 };
let die  = rng.range(1, 7);              // Int in [1, 7)
let pick = rng.next_int(10);             // Int in [0, 10)
let prob = rng.next_float();             // Float in [0.0, 1.0)
```

An `Rng` initialized with a given `seed` produces the exact same sequence across all runs and target architectures without sharing state with the global generator or other `Rng` instances.

> [!WARNING]
> Never use `std::rand::next_int` or `std::rand::Rng` to generate secrets, keys, or nonces; SplitMix64 is not cryptographically secure. Use `std::os::getrandom` instead.

Next: [Structured Logging](./logging.md).
