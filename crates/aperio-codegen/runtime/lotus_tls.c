/*
 * Aperio TLS substrate — `std::io::tls::*`.
 *
 * Lives in its own translation unit so helper test binaries (the
 * ones that include `lotus_arena.c` directly to exercise pieces
 * of the C runtime in isolation) don't pick up an unconditional
 * libssl/libcrypto dependency. The main `aperio build` link line
 * compiles both `lotus_arena.c` and `lotus_tls.c` and adds
 * `-lssl -lcrypto`.
 *
 * Client-only scope for v1:
 *   - lotus_tls_connect(host, port) -> int handle
 *   - lotus_tls_send_bytes(handle, bytes) -> int 0/-1
 *   - lotus_tls_recv_bytes(handle, max_bytes) -> Bytes
 *   - lotus_tls_close(handle) -> int 0/-1
 *
 * Handle semantics mirror lotus_tcp's raw fd shape: a small
 * non-negative integer that the Aperio caller threads through
 * each call until close. Internally a process-global table maps
 * handle -> (SSL*, raw_fd). The table only grows; closed slots
 * stay reserved with NULL ssl + raw_fd = -1 (handle exhaustion is
 * unlikely for v1's outbound-only scope).
 *
 * Verification: SSL_VERIFY_PEER + system default trust store +
 * SNI hostname matching via SSL_set1_host. Minimum negotiated
 * version is TLS 1.2. Any handshake failure logs through
 * ERR_print_errors_fp(stderr) and returns -1; the Aperio surface
 * wraps these as IoError via std::io::tls path-call routing.
 */

#include <openssl/ssl.h>
#include <openssl/err.h>

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <stdint.h>
#include <limits.h>

/* Forward decls — bodies live in lotus_arena.c. */
int   lotus_tcp_connect(const char *host, uint16_t port);
int64_t lotus_bytes_len(const void *b);
void *lotus_bytes_data(void *b);
/* Phase 1: builder-reserve helpers for recv_into. Keep the
 * lotus_bytes_builder_t layout opaque in this TU; let
 * lotus_arena.c manage the growth + len bump. */
void *lotus_bytes_builder_reserve(void *handle, int64_t n);
void  lotus_bytes_builder_advance(void *handle, int64_t n);

/* lotus_arena.c exposes the lazy-inited bus payload arena via
 * this getter so we can build Bytes blobs whose lifetime is the
 * program (mirrors the tcp_recv_bytes shape — the returned Bytes
 * must outlive the call frame). */
typedef struct lotus_arena lotus_arena_t;
lotus_arena_t *lotus_bus_payload_arena_get(void);
void          *lotus_bytes_create(lotus_arena_t *a, int64_t len);

/* Local equivalent of lotus_arena.c's static lotus_bytes_empty_global:
 * the canonical "empty Bytes" return for error / EOF paths. Inline
 * here because the original is `static` in its TU. */
static void *lotus_tls__bytes_empty(void) {
    lotus_arena_t *parena = lotus_bus_payload_arena_get();
    if (!parena) return NULL;
    return lotus_bytes_create(parena, 0);
}

typedef struct lotus_tls_entry {
    SSL *ssl;
    int  raw_fd;
} lotus_tls_entry_t;

static lotus_tls_entry_t *g_tls_entries = NULL;
static size_t            g_tls_count    = 0;
static size_t            g_tls_cap      = 0;
static pthread_mutex_t   g_tls_mutex    = PTHREAD_MUTEX_INITIALIZER;
static SSL_CTX          *g_tls_ctx      = NULL;

static SSL_CTX *lotus_tls__ctx_get(void) {
    /* Caller must hold g_tls_mutex. SSL_CTX_new + the trust store
     * setup are idempotent-but-not-cheap; we cache one process-
     * global client context. OpenSSL 1.1+ auto-initializes the
     * library on first use; older versions are out of scope. */
    if (g_tls_ctx) return g_tls_ctx;
    SSL_CTX *ctx = SSL_CTX_new(TLS_client_method());
    if (!ctx) {
        fprintf(stderr, "lotus_tls: SSL_CTX_new failed\n");
        ERR_print_errors_fp(stderr);
        return NULL;
    }
    /* 2026-05-22 PM: SSL_MODE_RELEASE_BUFFERS — release the
     * per-connection read+write buffers (each ~16-32 KiB) back to
     * libc malloc when no record is in flight, rather than holding
     * them for the lifetime of the SSL object. The cost is a
     * libc malloc/free pair per TLS record on the active path,
     * negligible at typical WS-frame rates; the win is flat memory
     * on long-running TLS clients. Without this, fathom-shaped
     * workloads (a handful of long-lived TLS streams, sporadic
     * record traffic) accumulated ~0.12 MB/min in the [heap]
     * segment outside Aperio's arena allocator — well-known
     * OpenSSL behavior and well-known fix. See
     * https://www.openssl.org/docs/man3.0/man3/SSL_CTX_set_mode.html
     */
    SSL_CTX_set_mode(ctx, SSL_MODE_RELEASE_BUFFERS);
    /* System trust store (e.g. /etc/ssl/certs on Debian/Ubuntu,
     * the macOS Keychain via OpenSSL's adapter). */
    if (SSL_CTX_set_default_verify_paths(ctx) != 1) {
        fprintf(stderr,
                "lotus_tls: SSL_CTX_set_default_verify_paths warning\n");
        ERR_print_errors_fp(stderr);
        /* Non-fatal — verification will still attempt against any
         * explicit trust roots set in the environment. */
    }
    SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, NULL);
    SSL_CTX_set_min_proto_version(ctx, TLS1_2_VERSION);
    g_tls_ctx = ctx;
    return ctx;
}

int lotus_tls_connect(const char *host, uint16_t port) {
    if (!host) {
        errno = EINVAL;
        return -1;
    }
    int raw_fd = lotus_tcp_connect(host, port);
    if (raw_fd < 0) {
        /* lotus_tcp_connect already set errno + logged. */
        return -1;
    }
    pthread_mutex_lock(&g_tls_mutex);
    SSL_CTX *ctx = lotus_tls__ctx_get();
    if (!ctx) {
        pthread_mutex_unlock(&g_tls_mutex);
        close(raw_fd);
        errno = ENOMEM;
        return -1;
    }
    SSL *ssl = SSL_new(ctx);
    if (!ssl) {
        pthread_mutex_unlock(&g_tls_mutex);
        close(raw_fd);
        errno = ENOMEM;
        return -1;
    }
    pthread_mutex_unlock(&g_tls_mutex);

    /* SNI + hostname-verification both go through the SSL object,
     * not the context — they're per-connection. */
    if (SSL_set_tlsext_host_name(ssl, host) != 1) {
        ERR_print_errors_fp(stderr);
    }
    if (SSL_set1_host(ssl, host) != 1) {
        fprintf(stderr, "lotus_tls_connect: SSL_set1_host failed\n");
        ERR_print_errors_fp(stderr);
    }
    SSL_set_fd(ssl, raw_fd);

    int r = SSL_connect(ssl);
    if (r != 1) {
        int err = SSL_get_error(ssl, r);
        fprintf(stderr,
                "lotus_tls_connect: handshake failed (host=%s port=%u err=%d)\n",
                host, (unsigned)port, err);
        ERR_print_errors_fp(stderr);
        SSL_free(ssl);
        close(raw_fd);
        errno = ECONNREFUSED;
        return -1;
    }

    /* Allocate a handle. The table grows by doubling. Closed slots
     * are not currently reclaimed; v1 outbound-only TLS isn't
     * expected to churn enough handles to need a free-list. */
    pthread_mutex_lock(&g_tls_mutex);
    if (g_tls_count == g_tls_cap) {
        size_t new_cap = g_tls_cap == 0 ? 8 : g_tls_cap * 2;
        lotus_tls_entry_t *grown = (lotus_tls_entry_t *)
            realloc(g_tls_entries, new_cap * sizeof(*g_tls_entries));
        if (!grown) {
            pthread_mutex_unlock(&g_tls_mutex);
            SSL_shutdown(ssl);
            SSL_free(ssl);
            close(raw_fd);
            errno = ENOMEM;
            return -1;
        }
        g_tls_entries = grown;
        g_tls_cap     = new_cap;
    }
    int handle = (int)g_tls_count;
    g_tls_entries[handle].ssl    = ssl;
    g_tls_entries[handle].raw_fd = raw_fd;
    g_tls_count++;
    pthread_mutex_unlock(&g_tls_mutex);
    return handle;
}

int lotus_tls_send_bytes(int handle, const void *bytes_ptr) {
    if (handle < 0 || (size_t)handle >= g_tls_count) {
        errno = EBADF;
        return -1;
    }
    if (!bytes_ptr) {
        errno = EINVAL;
        return -1;
    }
    SSL *ssl = g_tls_entries[handle].ssl;
    if (!ssl) {
        errno = EBADF;
        return -1;
    }
    int64_t total = lotus_bytes_len(bytes_ptr);
    if (total < 0) {
        errno = EINVAL;
        return -1;
    }
    const char *p = (const char *)bytes_ptr + sizeof(int64_t);
    size_t left = (size_t)total;
    while (left > 0) {
        int chunk = left > INT_MAX ? INT_MAX : (int)left;
        int n = SSL_write(ssl, p, chunk);
        if (n > 0) {
            p    += (size_t)n;
            left -= (size_t)n;
            continue;
        }
        int err = SSL_get_error(ssl, n);
        fprintf(stderr,
                "lotus_tls_send_bytes: SSL_write failed (err=%d)\n",
                err);
        ERR_print_errors_fp(stderr);
        errno = EIO;
        return -1;
    }
    return 0;
}

void *lotus_tls_recv_bytes(int handle, int max_bytes) {
    if (handle < 0 || (size_t)handle >= g_tls_count || max_bytes <= 0) {
        return lotus_tls__bytes_empty();
    }
    SSL *ssl = g_tls_entries[handle].ssl;
    if (!ssl) return lotus_tls__bytes_empty();

    lotus_arena_t *parena = lotus_bus_payload_arena_get();
    if (!parena) return lotus_tls__bytes_empty();
    void *blob = lotus_bytes_create(parena, (int64_t)max_bytes);
    if (!blob) return lotus_tls__bytes_empty();

    char *body = (char *)lotus_bytes_data(blob);
    int n = SSL_read(ssl, body, max_bytes);
    if (n <= 0) {
        /* Peer closed (0) or read error (<0). Hand back an empty
         * Bytes so the caller can detect "nothing read"; the
         * reserved arena memory leaks until program exit
         * (matches lotus_tcp_recv_bytes's convention). */
        int err = SSL_get_error(ssl, n);
        if (n < 0) {
            fprintf(stderr,
                    "lotus_tls_recv_bytes: SSL_read failed (err=%d)\n",
                    err);
            ERR_print_errors_fp(stderr);
        }
        return lotus_tls__bytes_empty();
    }
    /* Patch the length prefix down to the actual bytes read. The
     * blob layout is: [int64 len][body bytes], matching
     * lotus_bytes_create. */
    *(int64_t *)blob = (int64_t)n;
    return blob;
}

/* Phase 1: caller-provided destination for TLS. Mirrors
 * lotus_tcp_recv_into / lotus_udp_recv_into in lotus_arena.c —
 * SSL_read into the builder's tail, bump len, no allocation in
 * g_bus_payload_arena. Return semantics: > 0 bytes read, 0 peer
 * closed cleanly, < 0 fatal error. */
int64_t lotus_tls_recv_into(int handle, void *builder, int64_t max_bytes) {
    if (handle < 0 || (size_t)handle >= g_tls_count || max_bytes <= 0) {
        return -1;
    }
    SSL *ssl = g_tls_entries[handle].ssl;
    if (!ssl) return -1;
    char *tail = (char *)lotus_bytes_builder_reserve(builder, max_bytes);
    if (!tail) return -1;
    /* SSL_read returns >0 bytes, 0 on clean shutdown, <0 on
     * error. We don't retry on SSL_ERROR_WANT_READ here because
     * the underlying fd is blocking in pond/websocket's shape;
     * callers driving non-blocking sockets must wrap this with
     * their own poll() loop. */
    int n = SSL_read(ssl, tail, (int)max_bytes);
    if (n < 0) {
        int err = SSL_get_error(ssl, n);
        fprintf(stderr,
                "lotus_tls_recv_into: SSL_read failed (err=%d)\n",
                err);
        ERR_print_errors_fp(stderr);
        return -1;
    }
    if (n > 0) {
        lotus_bytes_builder_advance(builder, (int64_t)n);
    }
    return (int64_t)n;
}

int lotus_tls_close(int handle) {
    if (handle < 0 || (size_t)handle >= g_tls_count) {
        errno = EBADF;
        return -1;
    }
    pthread_mutex_lock(&g_tls_mutex);
    SSL *ssl = g_tls_entries[handle].ssl;
    int fd   = g_tls_entries[handle].raw_fd;
    g_tls_entries[handle].ssl    = NULL;
    g_tls_entries[handle].raw_fd = -1;
    pthread_mutex_unlock(&g_tls_mutex);
    if (ssl) {
        /* Send close_notify; ignore the return — peer may have
         * already closed. SSL_free releases the connection state
         * regardless. */
        SSL_shutdown(ssl);
        SSL_free(ssl);
    }
    if (fd >= 0) close(fd);
    return 0;
}
