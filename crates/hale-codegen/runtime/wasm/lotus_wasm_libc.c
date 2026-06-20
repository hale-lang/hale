/* lotus-wasm bundled libc (self-contained, no external sysroot).
 * WASM plan, Phase 1. Compiled ONLY for the wasm32 target, with
 * `-fno-builtin` so the byte-loop mem and str functions below aren't
 * re-recognized into recursive memcpy/memset calls. memcpy/memset/
 * memmove also lower to wasm `memory.copy`/`memory.fill` intrinsics
 * under `-mbulk-memory`; these definitions are the fallback and cover
 * the libc-name call sites the runtime makes.
 *
 * Allocator: a bump allocator over wasm linear memory growing from
 * `__heap_base` (provided by wasm-ld). `free` is a no-op — adequate for
 * finite programs and for the arena's chunk-pool reuse pattern (which
 * amortizes malloc); a reclaiming free-list allocator is a later
 * refinement for long-running browser loops. */

typedef unsigned long size_t;
typedef unsigned char u8;

/* wasm-ld places the heap base symbol at the end of static data. */
extern u8 __heap_base;

static size_t g_brk = 0;

/* Backing storage for the shim's single-threaded TLS keys + stdio
 * sentinels (see runtime/wasm/lotus_wasm_shim.h). */
void *lotus_wasm_tls_slots[64] = {0};
unsigned lotus_wasm_tls_next = 0;
int errno = 0;
struct lotus_wasm_FILE { int _; };
static struct lotus_wasm_FILE g_stderr_file, g_stdout_file, g_stdin_file;
struct lotus_wasm_FILE *const stderr = &g_stderr_file;
struct lotus_wasm_FILE *const stdout = &g_stdout_file;
struct lotus_wasm_FILE *const stdin = &g_stdin_file;

static size_t wasm_mem_bytes(void) {
    return (size_t)__builtin_wasm_memory_size(0) * 65536u;
}

void *malloc(size_t n) {
    if (g_brk == 0) {
        g_brk = (size_t)(&__heap_base);
    }
    /* 16-byte align (matches the arena's max scalar alignment). */
    g_brk = (g_brk + 15u) & ~(size_t)15u;
    size_t p = g_brk;
    g_brk += n;
    size_t have = wasm_mem_bytes();
    if (g_brk > have) {
        size_t need = g_brk - have;
        size_t pages = (need + 65535u) / 65536u;
        if (__builtin_wasm_memory_grow(0, (long)pages) == (size_t)-1) {
            return 0; /* OOM */
        }
    }
    return (void *)p;
}

void free(void *p) { (void)p; }

void *calloc(size_t count, size_t size) {
    size_t n = count * size;
    u8 *p = (u8 *)malloc(n);
    if (p) {
        for (size_t i = 0; i < n; i++) p[i] = 0;
    }
    return p;
}

/* No stored allocation sizes, so realloc conservatively allocates fresh
 * and copies `n` bytes. The runtime's only realloc caller is the arena
 * subregion free-list growth (N -> 2N), where copying the new (larger)
 * size reads at most N unused bytes past the old buffer — within the
 * heap, harmless. A size-tracking allocator removes the over-read. */
void *realloc(void *old, size_t n) {
    u8 *q = (u8 *)malloc(n);
    if (old && q) {
        u8 *o = (u8 *)old;
        for (size_t i = 0; i < n; i++) q[i] = o[i];
    }
    return q;
}

void *memcpy(void *dst, const void *src, size_t n) {
    u8 *d = (u8 *)dst;
    const u8 *s = (const u8 *)src;
    for (size_t i = 0; i < n; i++) d[i] = s[i];
    return dst;
}

void *memmove(void *dst, const void *src, size_t n) {
    u8 *d = (u8 *)dst;
    const u8 *s = (const u8 *)src;
    if (d < s) {
        for (size_t i = 0; i < n; i++) d[i] = s[i];
    } else {
        for (size_t i = n; i > 0; i--) d[i - 1] = s[i - 1];
    }
    return dst;
}

void *memset(void *dst, int c, size_t n) {
    u8 *d = (u8 *)dst;
    for (size_t i = 0; i < n; i++) d[i] = (u8)c;
    return dst;
}

int memcmp(const void *a, const void *b, size_t n) {
    const u8 *x = (const u8 *)a;
    const u8 *y = (const u8 *)b;
    for (size_t i = 0; i < n; i++) {
        if (x[i] != y[i]) return (int)x[i] - (int)y[i];
    }
    return 0;
}

size_t strlen(const char *s) {
    size_t n = 0;
    while (s[n]) n++;
    return n;
}

int strcmp(const char *a, const char *b) {
    while (*a && (*a == *b)) { a++; b++; }
    return (int)(u8)*a - (int)(u8)*b;
}

int strncmp(const char *a, const char *b, size_t n) {
    for (size_t i = 0; i < n; i++) {
        u8 ca = (u8)a[i], cb = (u8)b[i];
        if (ca != cb) return (int)ca - (int)cb;
        if (!ca) break;
    }
    return 0;
}

void *memchr(const void *s, int c, size_t n) {
    const u8 *p = (const u8 *)s;
    for (size_t i = 0; i < n; i++) if (p[i] == (u8)c) return (void *)(p + i);
    return 0;
}

char *strchr(const char *s, int c) {
    for (;; s++) { if (*s == (char)c) return (char *)s; if (!*s) return 0; }
}

char *strrchr(const char *s, int c) {
    const char *last = 0;
    for (;; s++) { if (*s == (char)c) last = s; if (!*s) break; }
    return (char *)last;
}

char *strstr(const char *h, const char *n) {
    if (!*n) return (char *)h;
    for (; *h; h++) {
        const char *a = h, *b = n;
        while (*a && *b && *a == *b) { a++; b++; }
        if (!*b) return (char *)h;
    }
    return 0;
}

char *strdup(const char *s) {
    size_t n = strlen(s) + 1;
    char *p = (char *)malloc(n);
    if (p) for (size_t i = 0; i < n; i++) p[i] = s[i];
    return p;
}

/* No host process to exit in v1: trap the module (a host-driven clean
 * shutdown replaces this in a later phase). */
__attribute__((noreturn)) void exit(int code) { (void)code; __builtin_trap(); }
__attribute__((noreturn)) void _exit(int code) { (void)code; __builtin_trap(); }

/* ---- 128-bit integer builtins (compiler-rt) ----------------------
 * clang lowers __int128 multiply / divide / int->double to these
 * libcalls, and Ubuntu's clang ships no wasm32 builtins archive — so the
 * runtime's Decimal path (i128 mantissa at scale 9: arithmetic +
 * to_string + to_float) linked __multi3 / __udivti3 / __umodti3 /
 * __floatuntidf as undefined imports that the JS loader stubbed to 0,
 * making every Decimal on wasm garbage. Provide real implementations.
 *
 * The bodies use ONLY 64-bit operations (plus *constant* 128-bit
 * shift/or to split & join the halves, which LLVM inlines to register
 * moves) — never a 128-bit multiply, divide, or variable shift — so they
 * cannot recurse into the very builtins they define. -fno-builtin (this
 * TU) also keeps the loops from being re-recognized into libcalls. */
typedef unsigned long long lotus_u64;
#define LOTUS_LO64(x) ((lotus_u64)(x))
#define LOTUS_HI64(x) ((lotus_u64)((unsigned __int128)(x) >> 64))
#define LOTUS_MK128(hi, lo) \
    (((unsigned __int128)(lotus_u64)(hi) << 64) | (unsigned __int128)(lotus_u64)(lo))

unsigned __int128 __multi3(unsigned __int128 a, unsigned __int128 b) {
    lotus_u64 alo = LOTUS_LO64(a), ahi = LOTUS_HI64(a);
    lotus_u64 blo = LOTUS_LO64(b), bhi = LOTUS_HI64(b);
    /* 64x64 -> 128 for alo*blo via 32-bit partial products. */
    lotus_u64 a0 = (unsigned)alo, a1 = alo >> 32;
    lotus_u64 b0 = (unsigned)blo, b1 = blo >> 32;
    lotus_u64 t = a0 * b0;
    lotus_u64 w0 = (unsigned)t;
    lotus_u64 t1 = a1 * b0 + (t >> 32);
    lotus_u64 w1 = (unsigned)t1;
    lotus_u64 w2 = t1 >> 32;
    lotus_u64 t2 = a0 * b1 + w1;
    w1 = (unsigned)t2;
    lotus_u64 lo_ll = (w1 << 32) | w0;
    lotus_u64 hi_ll = a1 * b1 + w2 + (t2 >> 32);
    /* + cross terms (only their low 64 bits land in the result's hi). */
    lotus_u64 res_hi = hi_ll + alo * bhi + ahi * blo;
    return LOTUS_MK128(res_hi, lo_ll);
}

/* Unsigned 128/128 divmod, shift-subtract over 64-bit halves. */
static unsigned __int128 lotus_udivmod128(unsigned __int128 n,
                                          unsigned __int128 d,
                                          unsigned __int128 *rem) {
    lotus_u64 n_hi = LOTUS_HI64(n), n_lo = LOTUS_LO64(n);
    lotus_u64 d_hi = LOTUS_HI64(d), d_lo = LOTUS_LO64(d);
    if ((d_hi | d_lo) == 0) __builtin_trap();   /* divide by zero */
    lotus_u64 q_hi = 0, q_lo = 0;
    lotus_u64 r_hi = 0, r_lo = 0;
    for (int i = 127; i >= 0; i--) {
        r_hi = (r_hi << 1) | (r_lo >> 63);      /* r <<= 1 */
        r_lo = r_lo << 1;
        lotus_u64 bit = (i < 64) ? ((n_lo >> i) & 1u)
                                 : ((n_hi >> (i - 64)) & 1u);
        r_lo |= bit;
        if (r_hi > d_hi || (r_hi == d_hi && r_lo >= d_lo)) {  /* r >= d */
            lotus_u64 borrow = (r_lo < d_lo) ? 1u : 0u;
            r_lo = r_lo - d_lo;
            r_hi = r_hi - d_hi - borrow;
            if (i < 64) q_lo |= ((lotus_u64)1 << i);
            else        q_hi |= ((lotus_u64)1 << (i - 64));
        }
    }
    if (rem) *rem = LOTUS_MK128(r_hi, r_lo);
    return LOTUS_MK128(q_hi, q_lo);
}

unsigned __int128 __udivti3(unsigned __int128 a, unsigned __int128 b) {
    return lotus_udivmod128(a, b, 0);
}
unsigned __int128 __umodti3(unsigned __int128 a, unsigned __int128 b) {
    unsigned __int128 r;
    lotus_udivmod128(a, b, &r);
    return r;
}
__int128 __divti3(__int128 a, __int128 b) {
    int neg = 0;
    unsigned __int128 ua = (a < 0) ? (neg ^= 1, (unsigned __int128)(-a))
                                   : (unsigned __int128)a;
    unsigned __int128 ub = (b < 0) ? (neg ^= 1, (unsigned __int128)(-b))
                                   : (unsigned __int128)b;
    unsigned __int128 q = lotus_udivmod128(ua, ub, 0);
    return neg ? -(__int128)q : (__int128)q;
}
__int128 __modti3(__int128 a, __int128 b) {
    int neg = (a < 0);
    unsigned __int128 ua = (a < 0) ? (unsigned __int128)(-a) : (unsigned __int128)a;
    unsigned __int128 ub = (b < 0) ? (unsigned __int128)(-b) : (unsigned __int128)b;
    unsigned __int128 r;
    lotus_udivmod128(ua, ub, &r);
    return neg ? -(__int128)r : (__int128)r;
}

/* i128 -> double. u64 -> double is native wasm (f64.convert_i64_u). */
double __floatuntidf(unsigned __int128 a) {
    double hi = (double)LOTUS_HI64(a);
    double lo = (double)LOTUS_LO64(a);
    return hi * 18446744073709551616.0 + lo;   /* hi * 2^64 + lo */
}
double __floattidf(__int128 a) {
    if (a < 0) return -__floatuntidf((unsigned __int128)(-a));
    return __floatuntidf((unsigned __int128)a);
}
