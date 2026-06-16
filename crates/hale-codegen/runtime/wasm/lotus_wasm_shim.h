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

/* env parsing helpers (dead on wasm: getenv() returns NULL). */
static inline unsigned long strtoul(const char *s, char **e, int base) {
    (void)s; (void)base; if (e) *e = (char *)s; return 0;
}
static inline long strtol(const char *s, char **e, int base) {
    (void)s; (void)base; if (e) *e = (char *)s; return 0;
}

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

#endif /* LOTUS_WASM_SHIM_H */
