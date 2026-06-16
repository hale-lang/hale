/* lotus-wasm bundled libc (self-contained, no external sysroot).
 * WASM plan, Phase 1. Compiled ONLY for the wasm32 target, with
 * `-fno-builtin` so the byte-loop mem*/str* below aren't re-recognized
 * into recursive memcpy/memset calls. memcpy/memset/memmove are also
 * lowered to wasm `memory.copy`/`memory.fill` intrinsics under
 * `-mbulk-memory`; these definitions are the fallback + cover the
 * libc-name call sites the runtime makes.
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
struct lotus_wasm_FILE { int _; };
static struct lotus_wasm_FILE g_stderr_file, g_stdout_file;
struct lotus_wasm_FILE *const stderr = &g_stderr_file;
struct lotus_wasm_FILE *const stdout = &g_stdout_file;

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
