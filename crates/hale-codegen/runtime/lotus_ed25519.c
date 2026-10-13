/* lotus_ed25519.c — `std::crypto`'s Ed25519 (RFC 8032), over Monocypher's
 * optional Ed25519 (SHA-512; runtime/third_party/monocypher, BSD-2 or
 * CC0).
 *
 * Sans-I/O: bytes in, bytes out. No thread, no syscall, no clock, no
 * entropy (signing is deterministic in RFC 8032), and no allocation but
 * the result blob, which lands where every runtime Bytes result does
 * (`lotus_caller_or_global_bytes_create`). Built, like nghttp2, by the
 * runtime's cc step and linked only into a program that reaches
 * `lotus_ed25519_*` (`ed25519_runtime.rs`), on wasm32 too: Monocypher
 * calls no libc, and this file calls none either (copies are loops).
 *
 * Keys are RFC 8032's: a 32-byte seed is the private key, and the
 * 64-byte form NaCl, libsodium and Go call the private key is the seed
 * followed by its public key. Signing takes either; a 64-byte key whose
 * public half is not the seed's is refused rather than used, since
 * signing with a wrong public half leaks the private scalar. Secret
 * intermediates are wiped with `crypto_wipe` before return. */
#include <stddef.h>
#include <stdint.h>
#include "monocypher-ed25519.h"

int64_t lotus_bytes_len(const void *b);
void *lotus_bytes_data(void *b);
void *lotus_caller_or_global_bytes_create(int64_t len);

static const uint8_t *lotus_ed25519_bytes(const void *b) {
    return b ? (const uint8_t *)b + sizeof(int64_t) : (const uint8_t *)"";
}

static void lotus_ed25519_copy(uint8_t *dst, const uint8_t *src, size_t n) {
    for (size_t i = 0; i < n; i++) dst[i] = src[i];
}

static void *lotus_ed25519_blob(const uint8_t *src, int64_t n) {
    void *out = lotus_caller_or_global_bytes_create(n);
    if (!out) return NULL;
    lotus_ed25519_copy((uint8_t *)lotus_bytes_data(out), src, (size_t)n);
    return out;
}

/* The secret key (seed + public key) of a 32-byte seed in `sk`, its
 * public key in `pk`. 0, or -1 when the seed is not 32 bytes. */
static int lotus_ed25519_derive(const void *seed_b, uint8_t sk[64], uint8_t pk[32]) {
    if (lotus_bytes_len(seed_b) < 32) return -1;
    uint8_t seed[32];
    lotus_ed25519_copy(seed, lotus_ed25519_bytes(seed_b), 32);
    crypto_ed25519_key_pair(sk, pk, seed); /* wipes `seed` */
    return 0;
}

/* `std::crypto::__ed25519_public_key(seed)`: the 32-byte public key of a
 * 32-byte seed, or an empty Bytes when the seed is any other length
 * (`ed25519_keypair` refuses that with its CryptoError). */
void *lotus_ed25519_public_key(const void *seed_b) {
    uint8_t sk[64], pk[32];
    if (lotus_bytes_len(seed_b) != 32 || lotus_ed25519_derive(seed_b, sk, pk) != 0) {
        return lotus_caller_or_global_bytes_create(0);
    }
    crypto_wipe(sk, sizeof sk);
    return lotus_ed25519_blob(pk, 32);
}

/* `std::crypto::ed25519_sign(secret, message)`: the 64-byte signature,
 * or NULL (the CryptoError) when `secret` is neither a 32-byte seed nor
 * a 64-byte seed + public key whose public half is the seed's. */
void *lotus_ed25519_sign_or_null(const void *secret_b, const void *msg_b) {
    int64_t n = lotus_bytes_len(secret_b);
    if (n != 32 && n != 64) return NULL;
    uint8_t sk[64], pk[32];
    if (lotus_ed25519_derive(secret_b, sk, pk) != 0) return NULL;
    if (n == 64 && crypto_verify32(pk, lotus_ed25519_bytes(secret_b) + 32) != 0) {
        crypto_wipe(sk, sizeof sk);
        return NULL;
    }
    uint8_t sig[64];
    crypto_ed25519_sign(sig, sk, lotus_ed25519_bytes(msg_b), (size_t)lotus_bytes_len(msg_b));
    crypto_wipe(sk, sizeof sk);
    return lotus_ed25519_blob(sig, 64);
}

/* `std::crypto::ed25519_verify(public, message, signature)`: 1 when the
 * signature is the public key's over the message, else 0 — a public key
 * that is not 32 bytes, a signature that is not 64, a non-canonical S or
 * a point that does not decode included (Monocypher's check is RFC
 * 8032's, with S < L enforced). */
int64_t lotus_ed25519_verify(const void *pub_b, const void *msg_b, const void *sig_b) {
    if (lotus_bytes_len(pub_b) != 32 || lotus_bytes_len(sig_b) != 64) return 0;
    return crypto_ed25519_check(lotus_ed25519_bytes(sig_b), lotus_ed25519_bytes(pub_b),
                                lotus_ed25519_bytes(msg_b), (size_t)lotus_bytes_len(msg_b)) == 0;
}
