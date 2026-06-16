/* lotus-wasm libc shim (self-contained, no external sysroot).
 * WASM plan, Phase 1. Replaces the POSIX/hosted include block of
 * lotus_arena.c when compiling for wasm32. Provides the freestanding
 * compiler headers + declarations for the bundled libc
 * (lotus_wasm_libc.c) + the handful of hosted functions the runtime
 * core touches. POSIX function FAMILIES (sockets, epoll, pthread, fs,
 * tls, ucontext, process, termios) are gated out of lotus_arena.c with
 * `#ifndef __wasm__`; what remains links against this shim. */
#ifndef LOTUS_WASM_SHIM_H
#define LOTUS_WASM_SHIM_H

/* Compiler-provided freestanding headers (available without a libc). */
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <stdarg.h>
#include <limits.h>
#include <stdatomic.h>

/* Bundled libc (lotus_wasm_libc.c). */
void *malloc(size_t);
void  free(void *);
void *calloc(size_t, size_t);
void *realloc(void *, size_t);
void *memcpy(void *, const void *, size_t);
void *memmove(void *, const void *, size_t);
void *memset(void *, int, size_t);
int   memcmp(const void *, const void *, size_t);
size_t strlen(const char *);
int   strcmp(const char *, const char *);
int   strncmp(const char *, const char *, size_t);

/* No environment in the browser: config getters return "unset" so the
 * runtime's built-in defaults apply. Defined inline (header-local) so
 * no extra TU is needed. */
static inline char *getenv(const char *name) { (void)name; return (void *)0; }

/* Fatal path. wasm has no abort(2); trap the module. */
static inline void abort(void) { __builtin_trap(); }

/* SIZE_MAX may not be defined without <stdint.h>'s hosted parts on some
 * setups; guard it. */
#ifndef SIZE_MAX
#define SIZE_MAX (~(size_t)0)
#endif

/* ---- single-threaded sync primitives -----------------------------
 * wasm (v1) is single-threaded: the cooperative scheduler is pumped by
 * the host event loop, never by worker threads. So the pervasive
 * pthread mutex/once/key usage in the arena + bus core becomes no-ops
 * (the genuinely-threading paths — pool workers, cond_wait — are gated
 * out with #ifndef __wasm__). Types are defined so structs that embed
 * them still lay out; only the primitives the core calls are stubbed. */
typedef int            pthread_mutex_t;
typedef int            pthread_mutexattr_t;
typedef int            pthread_once_t;
typedef unsigned long  pthread_t;
typedef unsigned int   pthread_key_t;
typedef struct { int _; } pthread_cond_t;
typedef struct { int _; } pthread_condattr_t;
typedef struct { int _; } pthread_attr_t;
#define PTHREAD_MUTEX_INITIALIZER 0
#define PTHREAD_ONCE_INIT 0

static inline int pthread_mutex_init(pthread_mutex_t *m, const pthread_mutexattr_t *a) { (void)a; if (m) *m = 0; return 0; }
static inline int pthread_mutex_lock(pthread_mutex_t *m) { (void)m; return 0; }
static inline int pthread_mutex_unlock(pthread_mutex_t *m) { (void)m; return 0; }
static inline int pthread_mutex_destroy(pthread_mutex_t *m) { (void)m; return 0; }
static inline int pthread_once(pthread_once_t *o, void (*fn)(void)) {
    if (o && *o == 0) { *o = 1; fn(); }
    return 0;
}
static inline pthread_t pthread_self(void) { return 0; }

/* TLS keys: single-threaded → one global slot per key (small fixed table). */
#define LOTUS_WASM_TLS_MAX 64
extern void *lotus_wasm_tls_slots[LOTUS_WASM_TLS_MAX];
extern unsigned lotus_wasm_tls_next;
static inline int pthread_key_create(pthread_key_t *k, void (*dtor)(void *)) {
    (void)dtor; if (lotus_wasm_tls_next >= LOTUS_WASM_TLS_MAX) return 1;
    *k = lotus_wasm_tls_next++; return 0;
}
static inline int pthread_setspecific(pthread_key_t k, const void *v) {
    if (k >= LOTUS_WASM_TLS_MAX) return 1; lotus_wasm_tls_slots[k] = (void *)v; return 0;
}
static inline void *pthread_getspecific(pthread_key_t k) {
    return k < LOTUS_WASM_TLS_MAX ? lotus_wasm_tls_slots[k] : (void *)0;
}

/* ---- stdio: diagnostics are inert in the browser (v1) -------------
 * The runtime's fprintf-to-stderr diagnostics (residency dumps, pool
 * stats) have no console sink yet; route them to no-ops. A host
 * `console.log` import replaces these in a later phase. */
typedef struct lotus_wasm_FILE lotus_wasm_FILE;
#define FILE lotus_wasm_FILE
extern lotus_wasm_FILE *const stderr;
extern lotus_wasm_FILE *const stdout;
static inline int fprintf(lotus_wasm_FILE *f, const char *fmt, ...) { (void)f; (void)fmt; return 0; }
static inline int printf(const char *fmt, ...) { (void)fmt; return 0; }
static inline int fflush(lotus_wasm_FILE *f) { (void)f; return 0; }
static inline int fputs(const char *s, lotus_wasm_FILE *f) { (void)s; (void)f; return 0; }
/* snprintf: names built with it (e.g. diag labels) become empty — fine
 * for v1 (no console). Always NUL-terminate; report 0 written. */
static inline int snprintf(char *buf, size_t n, const char *fmt, ...) {
    (void)fmt; if (buf && n) buf[0] = 0; return 0;
}

/* Numeric string parsing. Self-contained (no syscalls), so std::str::parse_int
 * / parse_float work under wasm just as they do natively — the portable
 * stdlib promise. Not bit-exact IEEE rounding for strtod, but correct for
 * the decimal magnitudes app/protocol data uses. */
static inline int lotus_wasm_isspace_(int c) {
    return c == ' ' || (c >= '\t' && c <= '\r');
}
static inline long long strtoll(const char *s, char **e, int base) {
    const char *p = s;
    while (lotus_wasm_isspace_((unsigned char)*p)) p++;
    int neg = 0;
    if (*p == '+' || *p == '-') { neg = (*p == '-'); p++; }
    if ((base == 0 || base == 16) && p[0] == '0' && (p[1] == 'x' || p[1] == 'X')) {
        p += 2; base = 16;
    } else if (base == 0 && p[0] == '0') {
        base = 8;
    } else if (base == 0) {
        base = 10;
    }
    long long acc = 0; int any = 0;
    for (;;) {
        int c = (unsigned char)*p, d;
        if (c >= '0' && c <= '9') d = c - '0';
        else if (c >= 'a' && c <= 'z') d = c - 'a' + 10;
        else if (c >= 'A' && c <= 'Z') d = c - 'A' + 10;
        else break;
        if (d >= base) break;
        acc = acc * base + d; any = 1; p++;
    }
    if (e) *e = (char *)(any ? p : s);
    return neg ? -acc : acc;
}
static inline unsigned long long strtoull(const char *s, char **e, int b) {
    return (unsigned long long)strtoll(s, e, b);
}
static inline long strtol(const char *s, char **e, int base) {
    return (long)strtoll(s, e, base);
}
static inline unsigned long strtoul(const char *s, char **e, int base) {
    return (unsigned long)strtoull(s, e, base);
}
static inline double strtod(const char *s, char **e) {
    const char *p = s;
    while (lotus_wasm_isspace_((unsigned char)*p)) p++;
    int neg = 0;
    if (*p == '+' || *p == '-') { neg = (*p == '-'); p++; }
    double v = 0.0; int any = 0;
    while (*p >= '0' && *p <= '9') { v = v * 10.0 + (*p - '0'); p++; any = 1; }
    if (*p == '.') {
        p++;
        double frac = 0.0, scale = 1.0;
        while (*p >= '0' && *p <= '9') { frac = frac * 10.0 + (*p - '0'); scale *= 10.0; p++; any = 1; }
        v += frac / scale;
    }
    if (any && (*p == 'e' || *p == 'E')) {
        const char *ep = p + 1; int eneg = 0;
        if (*ep == '+' || *ep == '-') { eneg = (*ep == '-'); ep++; }
        if (*ep >= '0' && *ep <= '9') {
            int exp = 0;
            while (*ep >= '0' && *ep <= '9') { exp = exp * 10 + (*ep - '0'); ep++; }
            double pw = 1.0;
            for (int i = 0; i < exp; i++) pw *= 10.0;
            if (eneg) v /= pw; else v *= pw;
            p = ep;
        }
    }
    if (e) *e = (char *)(any ? p : s);
    return neg ? -v : v;
}
static inline char *strerror(int e) { (void)e; return (char *)""; }

/* math + format constants (math.h / inttypes.h are gated out). */
#define INFINITY (__builtin_inff())
#define NAN      (__builtin_nanf(""))
#define PRIu64   "llu"

/* atexit: browser teardown happens at page unload via the host, not a C
 * atexit table. No-op for v1 (cleanup handlers are a later phase). */
static inline int atexit(void (*fn)(void)) { (void)fn; return 0; }

/* page size: wasm linear-memory pages are 64 KiB. */
#define _SC_PAGESIZE 30
static inline long sysconf(int name) { (void)name; return 65536; }

/* fd-based stdio (diagnostic dump paths) — inert in the browser. */
static inline int   dprintf(int fd, const char *fmt, ...) { (void)fd; (void)fmt; return 0; }
static inline int   fileno(lotus_wasm_FILE *f) { (void)f; return -1; }
static inline lotus_wasm_FILE *fdopen(int fd, const char *m) { (void)fd; (void)m; return (void *)0; }
static inline int   fclose(lotus_wasm_FILE *f) { (void)f; return 0; }
static inline int   dup(int fd) { (void)fd; return -1; }

/* qsort: minimal insertion sort (correct; only cold/diag paths sort).
 * Byte-wise swap avoids a temp buffer of dynamic element size. */
static inline void lotus_wasm_swap(unsigned char *x, unsigned char *y, size_t s) {
    for (size_t i = 0; i < s; i++) { unsigned char t = x[i]; x[i] = y[i]; y[i] = t; }
}
static inline void qsort(void *base, size_t n, size_t s,
                         int (*cmp)(const void *, const void *)) {
    unsigned char *a = (unsigned char *)base;
    for (size_t i = 1; i < n; i++)
        for (size_t j = i; j > 0 && cmp(a + (j - 1) * s, a + j * s) > 0; j--)
            lotus_wasm_swap(a + (j - 1) * s, a + j * s, s);
}
/* glibc reentrant qsort variant — same insertion sort, threaded arg. */
static inline void qsort_r(void *base, size_t n, size_t s,
                           int (*cmp)(const void *, const void *, void *),
                           void *arg) {
    unsigned char *a = (unsigned char *)base;
    for (size_t i = 1; i < n; i++)
        for (size_t j = i; j > 0 && cmp(a + (j - 1) * s, a + j * s, arg) > 0; j--)
            lotus_wasm_swap(a + (j - 1) * s, a + j * s, s);
}

/* ---- mmap: always-fail, so the arena's hugepage path falls back to
 * its malloc branch (the existing MAP_FAILED handling). The browser has
 * no mmap; chunks come from the bundled bump allocator. ------------- */
#define PROT_READ     0x1
#define PROT_WRITE    0x2
#define MAP_PRIVATE   0x2
#define MAP_ANONYMOUS 0x20
#define MAP_FAILED    ((void *)-1)
static inline void *mmap(void *a, size_t l, int p, int f, int fd, long off) {
    (void)a; (void)l; (void)p; (void)f; (void)fd; (void)off; return MAP_FAILED;
}
static inline int munmap(void *a, size_t l) { (void)a; (void)l; return 0; }

/* ---- rwlock: single-threaded → no-ops (see mutex note above). ----- */
typedef int pthread_rwlock_t;
typedef int pthread_rwlockattr_t;
static inline int pthread_rwlock_init(pthread_rwlock_t *l, const pthread_rwlockattr_t *a) { (void)a; if (l) *l = 0; return 0; }
static inline int pthread_rwlock_rdlock(pthread_rwlock_t *l) { (void)l; return 0; }
static inline int pthread_rwlock_wrlock(pthread_rwlock_t *l) { (void)l; return 0; }
static inline int pthread_rwlock_unlock(pthread_rwlock_t *l) { (void)l; return 0; }
static inline int pthread_rwlock_destroy(pthread_rwlock_t *l) { (void)l; return 0; }

/* ---- errno + POSIX constants + typedefs ---------------------------
 * errno-based error handling and POSIX integer constants are referenced
 * pervasively (including in core-adjacent code). errno is a real global;
 * the constants are plain integer #defines. The IO FUNCTIONS that use
 * sockets/fs/etc. are gated out with #ifndef __wasm__; these values just
 * let the remaining code compile. */
extern int errno;
#define EPERM 1
#define ENOENT 2
#define ESRCH 3
#define EINTR 4
#define EIO 5
#define EBADF 9
#define ECHILD 10
#define EAGAIN 11
#define EWOULDBLOCK EAGAIN
#define ENOMEM 12
#define EACCES 13
#define EEXIST 17
#define ENOTDIR 20
#define EISDIR 21
#define EINVAL 22
#define ENOSPC 28
#define EFBIG 27
#define ENAMETOOLONG 36
#define ENOTEMPTY 39
#define ENOTSUP 95
#define EADDRINUSE 98
#define ENETUNREACH 101
#define ECONNABORTED 103
#define ECONNRESET 104
#define ETIMEDOUT 110
#define ECONNREFUSED 111
#define EHOSTUNREACH 113
#define EPIPE 32
#define EMSGSIZE 90
#define E2BIG 7
#define ENOTSUP 95

typedef int   pid_t;
typedef long  off_t;
typedef long  time_t;
typedef long  suseconds_t;
typedef unsigned int socklen_t;
typedef unsigned int tcflag_t;

/* ---- time: stubbed (a host clock import replaces these later) ----- */
struct timespec { time_t tv_sec; long tv_nsec; };
struct timeval  { time_t tv_sec; suseconds_t tv_usec; };
#define CLOCK_REALTIME  0
#define CLOCK_MONOTONIC 1
typedef int clockid_t;
static inline int clock_gettime(clockid_t c, struct timespec *t) {
    (void)c; if (t) { t->tv_sec = 0; t->tv_nsec = 0; } return 0;
}
static inline int nanosleep(const struct timespec *r, struct timespec *rem) {
    (void)r; (void)rem; return 0;
}

/* POSIX declaration stubs for the gated-out IO/threading/coroutine
 * function families (compile-only; gc-stripped at link). Included last
 * so it sees the FILE define + pthread/typedef declarations above. */
#include "lotus_wasm_posix.h"

#endif /* LOTUS_WASM_SHIM_H */
