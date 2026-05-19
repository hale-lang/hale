/*
 * Lotus region allocator — v0 substrate.
 *
 * One arena = a linked list of bump chunks. Allocation bumps a
 * pointer in the head chunk; if the head can't fit the request,
 * a fresh chunk is malloc'd and pushed on the front. Destruction
 * walks the list and frees every chunk wholesale — no per-object
 * free, ever (matching spec/memory.md: "When the locus dissolves,
 * the region is freed wholesale.").
 *
 * v0 lives behind a stable C ABI so the LLVM-IR side of the
 * compiler doesn't need to know about the chunk-list shape.
 * m22 added per-coordinatee sub-regions (chunked-class
 * projection): a parent arena can carve "sub-region" arenas for
 * its accepted children, and tracks the slot indices via a
 * free-list so children can come and go without the parent's
 * bookkeeping growing unbounded. Sub-regions still hold their
 * own chunk lists — they're independent allocations — but they
 * register with the parent on creation and return their slot to
 * the parent's free-list on destroy.
 *
 * Backed by libc malloc for the chunks themselves. That's not a
 * cheat — the substrate's job is wholesale-region management;
 * the underlying *page* supplier can be libc, mmap, or a
 * pre-reserved pool, and the arena interface above doesn't
 * change. Replace this file's malloc/free with mmap when the
 * scheduler lands and we want page-aligned regions.
 */

#define _GNU_SOURCE
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <sched.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <unistd.h>
#include <errno.h>
#include <time.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <dirent.h>
#include <math.h>
/* C4: getrandom(2). Glibc 2.25+ exposes the declaration via
 * <sys/random.h>; we still gate the call site on a feature
 * macro so platforms that lack the syscall fall through to
 * the /dev/urandom path cleanly. */
#if defined(__linux__) || defined(__GLIBC__)
#include <sys/random.h>
#endif
/* C2 (pond/subprocess + pond/agent/sandbox): fork/exec/wait,
 * kill signals, poll for non-blocking pipe reads. */
#include <signal.h>
#include <sys/wait.h>
#include <poll.h>

/* Default chunk size: 64KB. Big enough that most loci fit in
 * one chunk, small enough that a leaf locus that allocates a
 * single ClosureViolation doesn't waste an entire MB. Tunable. */
#define LOTUS_ARENA_CHUNK_BYTES (64 * 1024)

typedef struct lotus_arena_chunk {
    struct lotus_arena_chunk *next;
    size_t                    used;
    size_t                    cap;
    /* `data` follows in the same allocation — accessed as
     * (char *)(chunk + 1). Inlined-trailing layout means each
     * chunk is one malloc, not two. */
} lotus_arena_chunk_t;

typedef struct lotus_arena {
    lotus_arena_chunk_t *head;
    size_t               default_chunk_size;
    /* m22: sub-region tracking. If `parent` is non-NULL, this
     * arena is a sub-region carved at one of its parent's slots;
     * destroy returns `slot` to the parent's free-list so the
     * next subregion_create can reuse it. Top-level arenas (the
     * program-wide @lotus.arena.global, plus any locus whose
     * parent is not chunked-class) have parent == NULL. */
    struct lotus_arena  *parent;
    int                  slot;
    /* m22: free-list of slot indices for sub-region children
     * (chunked-class). next_slot is the monotonic counter; freed
     * slots get pushed onto free_list and re-handed out before
     * the counter bumps again. free_list grows on demand. */
    int                 *free_list;
    size_t               free_count;
    size_t               free_cap;
    int                  next_slot;
    /* v1.x-3: when set, `lotus_arena_alloc` refuses to malloc a
     * fresh chunk on overflow — it returns NULL instead. Used by
     * recognition-class pools (fixed_cell + shared_slab) where
     * the capacity is a hard budget written down at the locus's
     * projection annotation. fixed_size also flags that the
     * arena struct + head chunk may live INLINE inside a recpool
     * cell (fixed_cell case), so `lotus_arena_destroy` becomes
     * a no-op and codegen routes teardown through the recpool's
     * release entry point instead. */
    int                  fixed_size;
} lotus_arena_t;

static lotus_arena_chunk_t *lotus_arena_new_chunk(size_t cap) {
    lotus_arena_chunk_t *c =
        (lotus_arena_chunk_t *)malloc(sizeof(lotus_arena_chunk_t) + cap);
    if (!c) return NULL;
    c->next = NULL;
    c->used = 0;
    c->cap  = cap;
    return c;
}

static inline size_t lotus_align_up(size_t n, size_t a) {
    return (n + a - 1) & ~(a - 1);
}

static lotus_arena_t *lotus_arena_alloc_struct(void) {
    lotus_arena_t *a = (lotus_arena_t *)malloc(sizeof(lotus_arena_t));
    if (!a) return NULL;
    a->default_chunk_size = LOTUS_ARENA_CHUNK_BYTES;
    a->head = lotus_arena_new_chunk(a->default_chunk_size);
    if (!a->head) {
        free(a);
        return NULL;
    }
    a->parent     = NULL;
    a->slot       = -1;
    a->free_list  = NULL;
    a->free_count = 0;
    a->free_cap   = 0;
    a->next_slot  = 0;
    a->fixed_size = 0;
    return a;
}

/* Public ABI ---------------------------------------------------- */

lotus_arena_t *lotus_arena_create(void) {
    return lotus_arena_alloc_struct();
}

/* Carve a sub-region of `parent`. The sub-region holds its own
 * chunk list (independent allocation lifetime is *bounded* by
 * the parent's, but the chunks themselves are separate from the
 * parent's chunks — m22 doesn't yet pool memory across siblings).
 *
 * The point of this entry point vs. plain `lotus_arena_create()`
 * is the bookkeeping: we get a slot number from the parent's
 * free-list / counter, and `lotus_arena_destroy` returns that
 * slot when this sub-region dies. The free-list keeps the
 * parent's slot space O(peak children alive), not O(total
 * children ever accepted). */
lotus_arena_t *lotus_arena_create_subregion(lotus_arena_t *parent) {
    if (!parent) return lotus_arena_create();
    lotus_arena_t *a = lotus_arena_alloc_struct();
    if (!a) return NULL;
    a->parent = parent;
    if (parent->free_count > 0) {
        a->slot = parent->free_list[--parent->free_count];
    } else {
        a->slot = parent->next_slot++;
    }
    return a;
}

void *lotus_arena_alloc(lotus_arena_t *a, size_t size, size_t align) {
    if (!a) return NULL;
    if (size == 0) size = 1;        /* every alloc gets a unique addr */
    if (align == 0) align = 8;      /* default 8-byte alignment */

    lotus_arena_chunk_t *c = a->head;
    size_t off = lotus_align_up(c->used, align);
    if (off + size > c->cap) {
        /* v1.x-3: recognition-class pools mark the arena
         * `fixed_size` — the cell's capacity is the budget
         * spelled at the locus's projection annotation, and
         * silently mallocing a fresh chunk would defeat that.
         * Return NULL; the caller (codegen-emitted body code in
         * v1.x-3 PR4+) routes this into the closure-violation
         * channel via lotus_root_panic. */
        if (a->fixed_size) return NULL;
        /* Need a fresh chunk. Size it to cover this single
         * request if the request itself is larger than the
         * default; otherwise use the default. The new chunk
         * becomes the head, so subsequent small allocs land
         * in it (and we don't bother trying to fit them into
         * older chunks — keeps the bump fast and the lookup
         * O(1)). */
        size_t need = size + align;
        size_t cap  = need > a->default_chunk_size
                          ? need
                          : a->default_chunk_size;
        lotus_arena_chunk_t *fresh = lotus_arena_new_chunk(cap);
        if (!fresh) return NULL;
        fresh->next = c;
        a->head = fresh;
        c = fresh;
        off = lotus_align_up(c->used, align);
    }

    char *base = (char *)(c + 1);
    void *p    = base + off;
    c->used    = off + size;
    return p;
}

void lotus_arena_destroy(lotus_arena_t *a) {
    if (!a) return;

    /* m22: if this is a sub-region, return its slot to the
     * parent's free-list so a future create_subregion can reuse
     * it. Grow the free_list capacity as needed (doubling).
     * The parent itself stays alive — only the SUB-region's
     * chunks + struct go away here. */
    if (a->parent) {
        lotus_arena_t *p = a->parent;
        if (p->free_count == p->free_cap) {
            size_t new_cap = p->free_cap == 0 ? 8 : p->free_cap * 2;
            int *new_list  = (int *)realloc(p->free_list,
                                            new_cap * sizeof(int));
            if (new_list) {
                p->free_list = new_list;
                p->free_cap  = new_cap;
            }
            /* If realloc failed, we silently drop the slot —
             * functionally correct (slot never gets reused) but
             * causes parent's slot space to grow. Acceptable
             * graceful-degradation for v0. */
        }
        if (p->free_count < p->free_cap) {
            p->free_list[p->free_count++] = a->slot;
        }
    }

    lotus_arena_chunk_t *c = a->head;
    while (c) {
        lotus_arena_chunk_t *next = c->next;
        free(c);
        c = next;
    }
    if (a->free_list) free(a->free_list);
    free(a);
}

/*
 * v1.x-3 — Recognition projection class pools.
 *
 * Recognition is the projection class for "I expect many siblings,
 * each shaped the same, with bounded per-child state." The locus
 * annotation
 *     : projection recognition(cap=N, <sub-mode>)
 * commits to a storage discipline at the declaration site, and the
 * sub-mode picks the allocator strategy at codegen time. v1 ships
 * two sub-modes; the other two parse + typecheck but reject at
 * codegen (mirrors the v1.x-4 / v1.x-4b surface-then-runtime split).
 *
 * fixed_cell(bytes=K): cap_count cells of K payload bytes each,
 *   pre-allocated as one contiguous block; bitmap-tracked. Each
 *   cell carries an INLINE lotus_arena_t + chunk header at its
 *   front, so the cell IS the child's arena — child body code
 *   treats the returned pointer as a regular lotus_arena_t* and
 *   the existing arena_alloc path bumps the in-cell bump pointer.
 *   Overflow returns NULL from arena_alloc (caller routes to the
 *   closure-violation channel). Release clears the bit; the slot
 *   is reusable. Whole block frees at parent dissolve.
 *
 * shared_slab(bytes=K): one fixed_size lotus_arena_t whose initial
 *   chunk is K bytes. Every acquire returns the SAME arena pointer
 *   — children share a bump space, so per-child release is a no-op
 *   and child structs + arena allocations interleave in the slab.
 *   Whole slab frees at parent dissolve. cap_count is recorded but
 *   not enforced at the C layer (codegen's birth-cap check is what
 *   limits concurrent children — the slab is a memory budget, not
 *   a child-count budget).
 *
 * In both cases the arena returned by acquire has `fixed_size=1`,
 * so `lotus_arena_alloc` refuses to grow on overflow. The codegen
 * dispatch (PR4) is responsible for emitting the matching
 * recpool_release at child dissolve and recpool_destroy at parent
 * dissolve instead of the regular lotus_arena_destroy — the cell
 * memory is owned by the recpool, not by the child's arena handle.
 *
 * Spec: spec/recognition.md (v1.x-3 PR6 ships the canonical doc).
 */

#include <assert.h>

typedef struct lotus_recpool_fixed {
    size_t    cap_count;     /* number of cells */
    size_t    cell_bytes;    /* user-facing payload bytes per cell */
    size_t    cell_stride;   /* total per-cell stride incl. inline header */
    size_t    bitmap_words;  /* number of uint64_t words in `bitmap` */
    uint64_t *bitmap;        /* 1 bit per cell; 1 = occupied */
    char     *cells;         /* cap_count * cell_stride bytes */
} lotus_recpool_fixed_t;

typedef struct lotus_recpool_slab {
    size_t         cap_count;   /* recorded, not enforced here (see codegen) */
    size_t         slab_bytes;
    lotus_arena_t *slab_arena;  /* fixed_size=1; never grows */
} lotus_recpool_slab_t;

/* Per-cell stride: inline lotus_arena_t + inline chunk header +
 * payload, rounded up to 16 bytes so the next cell is also 16-byte
 * aligned (the arena_alloc default align is 8; bumping to 16 covers
 * SSE/struct alignment without effort). */
static size_t lotus_recpool_compute_stride(size_t cell_bytes) {
    size_t raw = sizeof(lotus_arena_t)
               + sizeof(lotus_arena_chunk_t)
               + cell_bytes;
    return lotus_align_up(raw, 16);
}

/* Initialize the inline arena+chunk at the head of a cell so that
 * arena_alloc treats the rest of the cell as the bump space. The
 * cell layout is:
 *     [ lotus_arena_t | lotus_arena_chunk_t | cell_bytes payload ]
 * The arena's `head` points at the inline chunk; the chunk's data
 * lives at (chunk+1), which lands on the payload region. */
static void lotus_recpool_init_cell_arena(char *cell_base, size_t cell_bytes) {
    lotus_arena_t *a = (lotus_arena_t *)cell_base;
    lotus_arena_chunk_t *c =
        (lotus_arena_chunk_t *)(cell_base + sizeof(lotus_arena_t));
    c->next  = NULL;
    c->used  = 0;
    c->cap   = cell_bytes;

    a->head               = c;
    a->default_chunk_size = cell_bytes;  /* irrelevant when fixed_size=1 */
    a->parent             = NULL;
    a->slot               = -1;
    a->free_list          = NULL;
    a->free_count         = 0;
    a->free_cap           = 0;
    a->next_slot          = 0;
    a->fixed_size         = 1;
}

static size_t lotus_recpool_bitmap_words_for(size_t cap_count) {
    return (cap_count + 63) / 64;
}

/* Forward scan: find the lowest-index zero bit, or -1 if all set
 * up to cap_count. Uses ctzll on the inverted word for O(1) per
 * word; the bitmap is small enough (cap ~ 100s) that the loop is
 * fine without SIMD. */
static int lotus_recpool_bitmap_first_zero(uint64_t *bm,
                                           size_t words,
                                           size_t cap_count) {
    for (size_t w = 0; w < words; w++) {
        uint64_t inv = ~bm[w];
        if (inv == 0) continue;
        int b = __builtin_ctzll(inv);
        size_t slot = w * 64 + (size_t)b;
        if (slot >= cap_count) return -1;
        return (int)slot;
    }
    return -1;
}

/* fixed_cell ---------------------------------------------------- */

lotus_recpool_fixed_t *lotus_recpool_fixed_create(size_t cap_count,
                                                  size_t cell_bytes) {
    if (cap_count == 0 || cell_bytes == 0) return NULL;
    lotus_recpool_fixed_t *p =
        (lotus_recpool_fixed_t *)malloc(sizeof(lotus_recpool_fixed_t));
    if (!p) return NULL;
    p->cap_count    = cap_count;
    p->cell_bytes   = cell_bytes;
    p->cell_stride  = lotus_recpool_compute_stride(cell_bytes);
    p->bitmap_words = lotus_recpool_bitmap_words_for(cap_count);
    p->bitmap       = (uint64_t *)calloc(p->bitmap_words, sizeof(uint64_t));
    if (!p->bitmap) { free(p); return NULL; }
    p->cells = (char *)malloc(cap_count * p->cell_stride);
    if (!p->cells) { free(p->bitmap); free(p); return NULL; }
    return p;
}

lotus_arena_t *lotus_recpool_fixed_acquire(lotus_recpool_fixed_t *p) {
    if (!p) return NULL;
    int slot = lotus_recpool_bitmap_first_zero(p->bitmap,
                                               p->bitmap_words,
                                               p->cap_count);
    if (slot < 0) return NULL;
    p->bitmap[slot / 64] |= ((uint64_t)1 << (slot % 64));
    char *cell_base = p->cells + (size_t)slot * p->cell_stride;
    lotus_recpool_init_cell_arena(cell_base, p->cell_bytes);
    return (lotus_arena_t *)cell_base;
}

void lotus_recpool_fixed_release(lotus_recpool_fixed_t *p,
                                 lotus_arena_t *arena) {
    if (!p || !arena) return;
    char *base = (char *)arena;
    if (base < p->cells) return;
    size_t off = (size_t)(base - p->cells);
    if (off % p->cell_stride != 0) return;
    size_t slot = off / p->cell_stride;
    if (slot >= p->cap_count) return;
    p->bitmap[slot / 64] &= ~((uint64_t)1 << (slot % 64));
    /* Cell content stays valid-looking until the next acquire
     * re-initializes the inline arena. No memset — matches the
     * existing Pool free-list contract (caller of acquire is
     * responsible for treating the cell as freshly-allocated). */
}

void lotus_recpool_fixed_destroy(lotus_recpool_fixed_t *p) {
    if (!p) return;
    free(p->cells);
    free(p->bitmap);
    free(p);
}

/* shared_slab --------------------------------------------------- */

lotus_recpool_slab_t *lotus_recpool_slab_create(size_t cap_count,
                                                size_t slab_bytes) {
    if (slab_bytes == 0) return NULL;
    lotus_recpool_slab_t *p =
        (lotus_recpool_slab_t *)malloc(sizeof(lotus_recpool_slab_t));
    if (!p) return NULL;
    p->cap_count  = cap_count;
    p->slab_bytes = slab_bytes;
    /* Build the slab arena with one initial chunk sized to the
     * user-spelled budget, then mark it fixed_size=1 so arena_alloc
     * never mallocs a fresh chunk on overflow. */
    lotus_arena_t *a =
        (lotus_arena_t *)malloc(sizeof(lotus_arena_t));
    if (!a) { free(p); return NULL; }
    a->head = lotus_arena_new_chunk(slab_bytes);
    if (!a->head) { free(a); free(p); return NULL; }
    a->default_chunk_size = slab_bytes;
    a->parent             = NULL;
    a->slot               = -1;
    a->free_list          = NULL;
    a->free_count         = 0;
    a->free_cap           = 0;
    a->next_slot          = 0;
    a->fixed_size         = 1;
    p->slab_arena = a;
    return p;
}

lotus_arena_t *lotus_recpool_slab_acquire(lotus_recpool_slab_t *p) {
    if (!p) return NULL;
    /* Every child shares the same slab arena. Sibling allocations
     * interleave; per-child release is a no-op. The cap_count from
     * the locus annotation bounds the number of concurrent children
     * via codegen's accept-side check; the C layer doesn't track it. */
    return p->slab_arena;
}

void lotus_recpool_slab_release(lotus_recpool_slab_t *p,
                                lotus_arena_t *arena) {
    /* No-op by design — the slab is freed wholesale at parent
     * dissolve via lotus_recpool_slab_destroy. */
    (void)p;
    (void)arena;
}

void lotus_recpool_slab_destroy(lotus_recpool_slab_t *p) {
    if (!p) return;
    if (p->slab_arena) {
        /* arena_destroy walks the chunk list and frees each chunk
         * + the arena struct itself. The slab arena has one chunk
         * (it never grew, because fixed_size=1), so this frees the
         * slab cleanly. */
        lotus_arena_destroy(p->slab_arena);
    }
    free(p);
}

/*
 * F.22 capacity slot — Pool of T (fixed-size cell recycling).
 *
 * A pool holds a singly-linked list of chunks; each chunk is one
 * malloc holding N contiguous cells. Live cells are handed out
 * via acquire(); released cells get pushed onto an embedded
 * free-list (each free cell stores the next-free pointer at its
 * own base). When acquire() finds an empty free-list, it grows
 * by malloc'ing a fresh chunk and threading its cells onto the
 * list.
 *
 * Lifetime: wholesale teardown at slot destroy; individual
 * acquire/release rolls memory through the population without
 * touching the OS. The locus's parent arena is irrelevant — Pool
 * owns its own chunks and frees them in destroy.
 *
 * Cell stride = max(cell_size, sizeof(void*)) aligned to
 * cell_align. The sizeof(void*) floor ensures the embedded
 * free-list pointer fits inside any free cell, even if T's
 * own size is smaller than a pointer (e.g. Int8 in a future
 * narrow-int extension).
 *
 * Chunks grow geometrically (16, 32, 64, ...) capped at 4096
 * cells so peak-cells-alive populations don't all malloc on the
 * same boundary. The cap is tunable; the geometric ramp matches
 * the arena's "one big chunk amortizes many small allocs"
 * principle adapted to fixed-stride cells.
 *
 * v1.x-17: initial chunk size adapts to the host page size at
 * runtime — when one full page fits more than 16 cells of T,
 * the initial chunk holds page_size / cell_stride cells (so
 * the chunk is approximately one page including the chunk
 * header) instead of a hardcoded 16. Tiny T (single-byte cells
 * etc.) get a tighter initial chunk than the static 16 would
 * produce; large T (cell_stride > page/16) keep the static 16.
 * Falls back to LOTUS_POOL_INITIAL_CELLS when sysconf is
 * unavailable or returns nonsense.
 *
 * Spec: spec/design-rationale.md §F.22 — "Pool of T — *I hold
 * a bounded shape of recyclable state.*"
 */

#define LOTUS_POOL_INITIAL_CELLS 16
#define LOTUS_POOL_MAX_CHUNK_CELLS 4096

/* v1.x-17: page-size-aware initial chunk sizing. Cached after
 * first sysconf — page size doesn't change during program
 * lifetime, so a one-shot global is fine without locking
 * (the only race window writes the same value).
 */
static size_t lotus_host_page_size(void) {
    static size_t cached = 0;
    if (cached) return cached;
    long ps = sysconf(_SC_PAGESIZE);
    if (ps <= 0 || ps > (1L << 20)) {
        /* Implausible — fall back to the canonical 4 KiB. */
        cached = 4096;
    } else {
        cached = (size_t)ps;
    }
    return cached;
}

static size_t lotus_pool_initial_cells_for(size_t cell_stride) {
    if (cell_stride == 0) return LOTUS_POOL_INITIAL_CELLS;
    size_t page = lotus_host_page_size();
    if (page < cell_stride) return LOTUS_POOL_INITIAL_CELLS;
    size_t n = page / cell_stride;
    if (n < LOTUS_POOL_INITIAL_CELLS) n = LOTUS_POOL_INITIAL_CELLS;
    if (n > LOTUS_POOL_MAX_CHUNK_CELLS) n = LOTUS_POOL_MAX_CHUNK_CELLS;
    return n;
}

typedef struct lotus_pool_chunk {
    struct lotus_pool_chunk *next;
    size_t                   cells;
    /* cell data follows in the same allocation — first cell
     * starts at (char *)(chunk) + header_stride. */
} lotus_pool_chunk_t;

typedef struct lotus_pool {
    size_t              cell_stride;
    size_t              cell_align;
    size_t              header_stride;     /* aligned sizeof(chunk header) */
    size_t              next_chunk_cells;
    lotus_pool_chunk_t *chunks;
    void               *free_head;
} lotus_pool_t;

lotus_pool_t *lotus_pool_create(size_t cell_size, size_t cell_align) {
    if (cell_align == 0) cell_align = 8;
    size_t min_size = cell_size > sizeof(void *) ? cell_size : sizeof(void *);
    size_t stride   = lotus_align_up(min_size, cell_align);
    size_t hdr      = lotus_align_up(sizeof(lotus_pool_chunk_t), cell_align);
    lotus_pool_t *p = (lotus_pool_t *)malloc(sizeof(lotus_pool_t));
    if (!p) return NULL;
    p->cell_stride       = stride;
    p->cell_align        = cell_align;
    p->header_stride     = hdr;
    /* v1.x-17: initial chunk sized to host page size when that
     * fits more cells than the static-16 floor. */
    p->next_chunk_cells  = lotus_pool_initial_cells_for(stride);
    p->chunks            = NULL;
    p->free_head         = NULL;
    return p;
}

static int lotus_pool_grow(lotus_pool_t *p) {
    size_t n          = p->next_chunk_cells;
    size_t data_bytes = n * p->cell_stride;
    void  *raw        = malloc(p->header_stride + data_bytes);
    if (!raw) return 0;
    lotus_pool_chunk_t *c = (lotus_pool_chunk_t *)raw;
    c->next   = p->chunks;
    c->cells  = n;
    p->chunks = c;
    /* Thread the new cells onto the free-list in reverse so the
     * lowest-address cell ends up at the head — gives acquire-
     * order locality (first acquire after grow lands on the
     * lowest address, next acquire lands one stride above, etc.). */
    char *base = (char *)raw + p->header_stride;
    for (size_t i = n; i > 0; i--) {
        char *cell       = base + (i - 1) * p->cell_stride;
        *(void **)cell   = p->free_head;
        p->free_head     = cell;
    }
    if (p->next_chunk_cells < LOTUS_POOL_MAX_CHUNK_CELLS) {
        size_t doubled = p->next_chunk_cells * 2;
        p->next_chunk_cells = doubled > LOTUS_POOL_MAX_CHUNK_CELLS
                                  ? LOTUS_POOL_MAX_CHUNK_CELLS
                                  : doubled;
    }
    return 1;
}

void *lotus_pool_acquire(lotus_pool_t *p) {
    if (!p) return NULL;
    if (!p->free_head) {
        if (!lotus_pool_grow(p)) return NULL;
    }
    void *cell    = p->free_head;
    p->free_head  = *(void **)cell;
    /* Caller treats the cell as uninitialized — we don't memset.
     * Aperio's let-binding rule says every binding is the type's
     * initial declaration; the caller writes fields before any
     * read can observe the stale free-list pointer that still
     * sits in the cell's first sizeof(void*) bytes. */
    return cell;
}

void lotus_pool_release(lotus_pool_t *p, void *cell) {
    if (!p || !cell) return;
    *(void **)cell = p->free_head;
    p->free_head   = cell;
}

void lotus_pool_destroy(lotus_pool_t *p) {
    if (!p) return;
    lotus_pool_chunk_t *c = p->chunks;
    while (c) {
        lotus_pool_chunk_t *next = c->next;
        free(c);
        c = next;
    }
    free(p);
}

/*
 * F.22 capacity slot — Heap of T (individually-freed cells with
 * locus-bounded lifetime).
 *
 * Each alloc is one malloc; the heap struct holds a doubly-linked
 * list of every live cell so free() is O(1) (unlink the cell)
 * and destroy() can free every still-live cell wholesale.
 *
 * The list lives in a per-cell header sitting just before the
 * returned pointer in the same allocation. Cell payload starts
 * at base + header_stride, where header_stride is the aligned-up
 * size of the header. On free(), the header is recovered by
 * subtracting header_stride from the user pointer.
 *
 * Alignment: malloc returns alignof(max_align_t) (typically 16)
 * regardless of request. Aperio v1 types have alignment ≤ 8
 * (Int/Float = 8; user structs default to 8 or 16). For
 * cell_align > alignof(max_align_t) the substrate would need
 * posix_memalign; v1 doesn't generate such types so we don't
 * implement the fallback. If a cell_align larger than 16 ever
 * lands, the assertion path is to extend create() to record an
 * "oversized align" flag and route alloc through posix_memalign.
 *
 * Spec: spec/design-rationale.md §F.22 — "Heap of T — *I hold
 * growable state bounded by my own lifetime.*"
 */

typedef struct lotus_heap_cell {
    struct lotus_heap_cell *prev;
    struct lotus_heap_cell *next;
    /* cell payload follows at (char *)(cell) + header_stride. */
} lotus_heap_cell_t;

typedef struct lotus_heap {
    size_t              cell_size;
    size_t              cell_align;
    size_t              header_stride;
    lotus_heap_cell_t  *live_head;
} lotus_heap_t;

lotus_heap_t *lotus_heap_create(size_t cell_size, size_t cell_align) {
    if (cell_align == 0) cell_align = 8;
    size_t hdr = lotus_align_up(sizeof(lotus_heap_cell_t), cell_align);
    lotus_heap_t *h = (lotus_heap_t *)malloc(sizeof(lotus_heap_t));
    if (!h) return NULL;
    h->cell_size     = cell_size;
    h->cell_align    = cell_align;
    h->header_stride = hdr;
    h->live_head     = NULL;
    return h;
}

void *lotus_heap_alloc(lotus_heap_t *h) {
    if (!h) return NULL;
    void *raw = malloc(h->header_stride + h->cell_size);
    if (!raw) return NULL;
    lotus_heap_cell_t *cell = (lotus_heap_cell_t *)raw;
    cell->prev = NULL;
    cell->next = h->live_head;
    if (h->live_head) h->live_head->prev = cell;
    h->live_head = cell;
    return (char *)raw + h->header_stride;
}

void lotus_heap_free(lotus_heap_t *h, void *cell) {
    if (!h || !cell) return;
    lotus_heap_cell_t *hdr =
        (lotus_heap_cell_t *)((char *)cell - h->header_stride);
    if (hdr->prev) hdr->prev->next = hdr->next;
    else            h->live_head    = hdr->next;
    if (hdr->next) hdr->next->prev = hdr->prev;
    free(hdr);
}

void lotus_heap_destroy(lotus_heap_t *h) {
    if (!h) return;
    lotus_heap_cell_t *c = h->live_head;
    while (c) {
        lotus_heap_cell_t *next = c->next;
        free(c);
        c = next;
    }
    free(h);
}

/*
 * @form(vec) substrate (v1.x-FORM-1 PR4).
 *
 * A contiguous, growable buffer of elements of a single fixed
 * size. Inline in the locus's struct layout — codegen emits the
 * three-field struct `{ cap, len, buf }` for each `heap items of T`
 * slot under `@form(vec)`, and the functions below operate on
 * that struct generically by taking `elem_size` (= sizeof(T))
 * as an explicit parameter at each call site.
 *
 * The functions read/write the struct through a `void *` pointer
 * to the vec's start. All `lotus_vec_<T>_t` layouts share the
 * `{ size_t cap, size_t len, char *buf }` prefix — codegen
 * monomorphizes the typedef per T, but the runtime sees only the
 * common prefix. Element storage is contiguous in `buf`; the i-th
 * element lives at `buf + i * elem_size`.
 *
 * Growth policy: capacity starts at 0 (no allocation at locus
 * birth). The first `push` allocates a 4-element buffer. Each
 * overflow doubles cap and `realloc`s. Shrink is not implemented
 * in v1; `lotus_vec_destroy` releases the buffer at locus
 * dissolution.
 *
 * Fallible operations (`get`, `pop`) return `int` (1 = success,
 * 0 = error). Codegen in PR5/6 lifts that bool into the
 * `Ty::Fallible { success: T, payload: IndexError }` surface the
 * type system sees.
 */

typedef struct {
    size_t cap;
    size_t len;
    char *buf;
} lotus_vec_t;

/* Initial buffer size on first push, in elements. Chosen as a
 * small constant that avoids per-element malloc on tiny vecs
 * without wasting space for short-lived ones. */
#define LOTUS_VEC_INITIAL_CAP 4

void lotus_vec_init(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    v->cap = 0;
    v->len = 0;
    v->buf = NULL;
}

void lotus_vec_push(void *vec_ptr, size_t elem_size, const void *elem) {
    if (!vec_ptr || !elem) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len == v->cap) {
        size_t new_cap = v->cap == 0 ? LOTUS_VEC_INITIAL_CAP : v->cap * 2;
        char *new_buf = (char *)realloc(v->buf, new_cap * elem_size);
        if (!new_buf) {
            /* OOM. Per the design's failure surface, hardware
             * traps re-raise as closure violations; PR5/6 wires
             * that. For now, drop the push and signal via
             * unchanged v (best-effort; codegen integration will
             * add proper trap handling). */
            return;
        }
        v->buf = new_buf;
        v->cap = new_cap;
    }
    memcpy(v->buf + v->len * elem_size, elem, elem_size);
    v->len += 1;
}

int lotus_vec_get(void *vec_ptr, size_t elem_size, int64_t i, void *out) {
    if (!vec_ptr || !out) return 0;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (i < 0 || (size_t)i >= v->len) return 0;
    memcpy(out, v->buf + (size_t)i * elem_size, elem_size);
    return 1;
}

/* In-place mutation. Mirrors lotus_vec_get: bounds-checked at
 * [0, len). Returns 1 on success, 0 on out-of-bounds. Codegen
 * lifts that bool into `Ty::Fallible { success: (), payload:
 * IndexError }` at the call site. */
int lotus_vec_set(void *vec_ptr, size_t elem_size, int64_t i, const void *elem) {
    if (!vec_ptr || !elem) return 0;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (i < 0 || (size_t)i >= v->len) return 0;
    memcpy(v->buf + (size_t)i * elem_size, elem, elem_size);
    return 1;
}

int lotus_vec_pop(void *vec_ptr, size_t elem_size, void *out) {
    if (!vec_ptr || !out) return 0;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len == 0) return 0;
    v->len -= 1;
    memcpy(out, v->buf + v->len * elem_size, elem_size);
    return 1;
}

int64_t lotus_vec_len(void *vec_ptr) {
    if (!vec_ptr) return 0;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    return (int64_t)v->len;
}

int lotus_vec_is_empty(void *vec_ptr) {
    if (!vec_ptr) return 1;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    return v->len == 0 ? 1 : 0;
}

/* Typed comparators for the primitive `sort()` variants. qsort
 * is happy with these directly — no cookie / no trampoline. */
static int cmp_i64(const void *a, const void *b) {
    int64_t av = *(const int64_t *)a;
    int64_t bv = *(const int64_t *)b;
    return (av > bv) - (av < bv);
}
static int cmp_f64(const void *a, const void *b) {
    double av = *(const double *)a;
    double bv = *(const double *)b;
    if (av < bv) return -1;
    if (av > bv) return  1;
    return 0;
}
static int cmp_str(const void *a, const void *b) {
    const char *av = *(const char *const *)a;
    const char *bv = *(const char *const *)b;
    return strcmp(av, bv);
}

void lotus_vec_sort_int(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len < 2 || !v->buf) return;
    qsort(v->buf, v->len, sizeof(int64_t), cmp_i64);
}
void lotus_vec_sort_float(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len < 2 || !v->buf) return;
    qsort(v->buf, v->len, sizeof(double), cmp_f64);
}
void lotus_vec_sort_string(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len < 2 || !v->buf) return;
    qsort(v->buf, v->len, sizeof(const char *), cmp_str);
}

/* sort_by / sort_desc_by infrastructure. The trampoline pattern:
 * codegen emits a per-cell-type wrapper that loads (a, b) from
 * the buffer, calls the user's `fn(T, T) -> Bool` comparator,
 * and returns -1/0/+1 the way qsort expects. The cookie carries
 * (arena, user_cmp_fn, reverse_flag) — reverse_flag flips the
 * result so sort_desc_by reuses the same trampoline with a true
 * flag. */
typedef int (*lotus_vec_trampoline_t)(const void *a, const void *b, void *cookie);

void lotus_vec_sort_by(void *vec_ptr,
                       size_t elem_size,
                       lotus_vec_trampoline_t cmp,
                       void *cookie) {
    if (!vec_ptr || !cmp) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len < 2 || !v->buf) return;
    /* qsort_r is GNU-extension; the arg order matches glibc's
     * `(base, nmemb, size, compar, arg)` form. */
    qsort_r(v->buf, v->len, elem_size,
            (int (*)(const void *, const void *, void *))cmp,
            cookie);
}

void lotus_vec_destroy(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    free(v->buf);
    v->buf = NULL;
    v->cap = 0;
    v->len = 0;
}

/*
 * v1.x-FORM-4 — `@form(hashmap)` storage primitives.
 *
 * Intrusive open-addressing hash table with linear probing. The
 * value type S carries its own key as one of its fields
 * (`indexed_by <fieldname>`); codegen extracts the key by GEP'ing
 * the field offset before each call, so the C ABI takes key and
 * value as separate pointers and never has to know about the
 * struct's internal layout.
 *
 * Slot layout: each slot is `1 + key_size + value_size` bytes:
 *
 *   [occupied: 1 byte] [key: key_size bytes] [value: value_size bytes]
 *
 * `occupied = 0` means empty; we use backward-shift deletion
 * (no tombstones) so probes terminate as soon as an empty slot
 * is seen. Cap is always a power of two so the hash-to-index
 * fold is a single `& mask`. Initial cap = 8; doubles when load
 * factor exceeds 0.7.
 *
 * Key types at v1: 0 = Int (64-bit, Knuth multiplicative hash),
 * 1 = String (C-string pointer, FNV-1a over the bytes). The
 * key_type_tag is set at init and frozen for the hashmap's life.
 *
 * Fallible operations (`get`, `remove`) return `int` (1 =
 * success, 0 = not_found). Codegen in PR5/6 lifts that bool
 * into the `Ty::Fallible { success: S, payload: KeyError }`
 * surface the type system sees.
 */

#define LOTUS_HASHMAP_KEY_INT    0
#define LOTUS_HASHMAP_KEY_STRING 1

/* Initial slot count. Power of two so `& mask` folds the hash;
 * 8 covers small-population hashmaps (config tables, small
 * registries) without an early grow. */
#define LOTUS_HASHMAP_INITIAL_CAP 8

/* Load-factor threshold = LOAD_NUM / LOAD_DEN = 7/10. Grow
 * before insertion when `(len + 1) * LOAD_DEN > cap * LOAD_NUM`. */
#define LOTUS_HASHMAP_LOAD_NUM 7
#define LOTUS_HASHMAP_LOAD_DEN 10

typedef struct {
    size_t cap;
    size_t len;
    size_t key_size;
    size_t value_size;
    int key_type_tag;
    char *slots;
} lotus_hashmap_t;

static size_t lotus_hashmap_entry_size(const lotus_hashmap_t *m) {
    return 1 + m->key_size + m->value_size;
}

static size_t lotus_hashmap_hash(const lotus_hashmap_t *m, const void *key) {
    if (m->key_type_tag == LOTUS_HASHMAP_KEY_INT) {
        /* 64-bit Knuth multiplicative — distributes Int keys
         * including dense sequences (handles common workloads
         * like consecutive IDs without all colliding on slot 0). */
        uint64_t k = *(const uint64_t *)key;
        return (size_t)(k * 0x9E3779B97F4A7C15ULL);
    }
    /* String — the key is a C-string pointer; hash the bytes. */
    const char *s = *(const char *const *)key;
    if (!s) return 0;
    uint64_t h = 0xcbf29ce484222325ULL;
    for (const char *p = s; *p; ++p) {
        h ^= (uint8_t)*p;
        h *= 0x100000001b3ULL;
    }
    return (size_t)h;
}

static int lotus_hashmap_key_eq(const lotus_hashmap_t *m,
                                 const void *a,
                                 const void *b) {
    if (m->key_type_tag == LOTUS_HASHMAP_KEY_INT) {
        return *(const int64_t *)a == *(const int64_t *)b;
    }
    const char *sa = *(const char *const *)a;
    const char *sb = *(const char *const *)b;
    if (sa == sb) return 1;
    if (!sa || !sb) return 0;
    return strcmp(sa, sb) == 0;
}

/* Find the slot index for `key`. Returns either:
 *   - an existing entry with the matching key (slot occupied,
 *     key equal), or
 *   - the first empty slot encountered along the probe chain.
 * Caller inspects the occupied byte to disambiguate. */
static size_t lotus_hashmap_find_slot(const lotus_hashmap_t *m,
                                       const void *key) {
    size_t es = lotus_hashmap_entry_size(m);
    size_t mask = m->cap - 1;
    size_t i = lotus_hashmap_hash(m, key) & mask;
    for (;;) {
        char *slot = m->slots + i * es;
        if (!slot[0]) return i;
        if (lotus_hashmap_key_eq(m, slot + 1, key)) return i;
        i = (i + 1) & mask;
    }
}

void lotus_hashmap_init(void *map_ptr,
                         size_t key_size,
                         size_t value_size,
                         int key_type_tag) {
    if (!map_ptr) return;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    m->cap = LOTUS_HASHMAP_INITIAL_CAP;
    m->len = 0;
    m->key_size = key_size;
    m->value_size = value_size;
    m->key_type_tag = key_type_tag;
    size_t es = 1 + key_size + value_size;
    m->slots = (char *)calloc(m->cap, es);
}

/* Forward declaration — set + grow are mutually recursive on
 * the rehash path. */
void lotus_hashmap_set(void *map_ptr, const void *key, const void *value);

static void lotus_hashmap_grow(lotus_hashmap_t *m) {
    size_t old_cap = m->cap;
    char *old_slots = m->slots;
    size_t es = lotus_hashmap_entry_size(m);
    size_t new_cap = old_cap * 2;
    m->cap = new_cap;
    m->slots = (char *)calloc(new_cap, es);
    m->len = 0;
    /* Reinsert every live entry into the new table. The probe
     * sequence changes because mask = new_cap - 1 is wider, so
     * we route through the normal `set` path rather than copying
     * raw bytes. */
    for (size_t i = 0; i < old_cap; i++) {
        char *slot = old_slots + i * es;
        if (slot[0]) {
            lotus_hashmap_set(m, slot + 1, slot + 1 + m->key_size);
        }
    }
    free(old_slots);
}

void lotus_hashmap_set(void *map_ptr,
                        const void *key,
                        const void *value) {
    if (!map_ptr || !key || !value) return;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    /* Grow before insertion when adding one more entry would
     * cross the load-factor threshold. The check uses unsigned
     * arithmetic so it stays correct as len/cap grow. */
    if ((m->len + 1) * LOTUS_HASHMAP_LOAD_DEN >
        m->cap * LOTUS_HASHMAP_LOAD_NUM) {
        lotus_hashmap_grow(m);
    }
    size_t es = lotus_hashmap_entry_size(m);
    size_t i = lotus_hashmap_find_slot(m, key);
    char *slot = m->slots + i * es;
    int was_empty = !slot[0];
    slot[0] = 1;
    memcpy(slot + 1, key, m->key_size);
    memcpy(slot + 1 + m->key_size, value, m->value_size);
    if (was_empty) m->len++;
}

int lotus_hashmap_get(void *map_ptr, const void *key, void *out_value) {
    if (!map_ptr || !key || !out_value) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->len == 0) return 0;
    size_t es = lotus_hashmap_entry_size(m);
    size_t i = lotus_hashmap_find_slot(m, key);
    char *slot = m->slots + i * es;
    if (!slot[0]) return 0;
    memcpy(out_value, slot + 1 + m->key_size, m->value_size);
    return 1;
}

int lotus_hashmap_has(void *map_ptr, const void *key) {
    if (!map_ptr || !key) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->len == 0) return 0;
    size_t es = lotus_hashmap_entry_size(m);
    size_t i = lotus_hashmap_find_slot(m, key);
    return m->slots[i * es] ? 1 : 0;
}

/* Backward-shift deletion. After clearing the target slot,
 * walk forward and shift any entry whose natural position is
 * "before" the freed slot in the probe sequence — that's what
 * keeps `find_slot` correct without tombstones. */
int lotus_hashmap_remove(void *map_ptr, const void *key) {
    if (!map_ptr || !key) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->len == 0) return 0;
    size_t es = lotus_hashmap_entry_size(m);
    size_t mask = m->cap - 1;
    size_t i = lotus_hashmap_find_slot(m, key);
    if (!m->slots[i * es]) return 0;
    m->slots[i * es] = 0;
    m->len--;
    /* Walk forward through the cluster, shifting entries whose
     * probe chain runs through `i`. Stops at the first empty
     * slot — that's the cluster boundary. */
    size_t j = (i + 1) & mask;
    while (m->slots[j * es]) {
        size_t natural =
            lotus_hashmap_hash(m, m->slots + j * es + 1) & mask;
        size_t dist_to_j = (j - natural) & mask;
        size_t dist_to_i = (i - natural) & mask;
        if (dist_to_i < dist_to_j) {
            memmove(m->slots + i * es, m->slots + j * es, es);
            m->slots[j * es] = 0;
            i = j;
        }
        j = (j + 1) & mask;
    }
    return 1;
}

int64_t lotus_hashmap_len(void *map_ptr) {
    if (!map_ptr) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    return (int64_t)m->len;
}

int lotus_hashmap_is_empty(void *map_ptr) {
    if (!map_ptr) return 1;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    return m->len == 0 ? 1 : 0;
}

/* Hash-table-order iteration. Walk the slots array counting
 * occupied entries; on the i-th occupied slot copy out the key
 * (or value/entry) and return 1. Returns 0 if i is out of range
 * (i < 0 || i >= len).
 *
 * Order is hash-table order (insertion-affected but stable for
 * a given table state). For "populate then iterate" patterns
 * the snapshot order is reproducible; mixing iteration with
 * mutation will see shifting order after a rehash. Per-call
 * cost is O(cap), so a full sweep is O(cap²) — fine at small/
 * medium scale, watch out at 100k+ entries.
 */
/* 2026-05-16: word-tokenize a C-string into a @form(vec) of
 * String. A "word" is a maximal run of bytes for which
 * is_word_char (alpha + digit + underscore + apostrophe) is
 * true; whitespace and punctuation are delimiters. Each token is
 * lower-cased (canonical agent intent for wordfreq-style work)
 * and arena-allocated as a NUL-terminated C string; the pointer
 * is pushed into the target vec via lotus_vec_push.
 *
 * Caller is responsible for passing an empty (or otherwise
 * reusable) target vec; the primitive does NOT clear it.
 */
static int lotus_text_is_word_byte(unsigned char c) {
    return (c >= 'a' && c <= 'z')
        || (c >= 'A' && c <= 'Z')
        || (c >= '0' && c <= '9')
        || c == '_'
        || c == '\'';
}

void lotus_text_tokenize_words_into(
    void *target_vec,
    const char *src,
    void *arena_ptr,
    int lowercase
) {
    if (!target_vec || !src) return;
    lotus_arena_t *arena = (lotus_arena_t *)arena_ptr;
    size_t i = 0;
    while (src[i]) {
        /* Skip non-word bytes. */
        while (src[i] && !lotus_text_is_word_byte((unsigned char)src[i])) {
            i++;
        }
        if (!src[i]) break;
        size_t start = i;
        while (src[i] && lotus_text_is_word_byte((unsigned char)src[i])) {
            i++;
        }
        size_t tok_len = i - start;
        /* Arena-allocate tok_len + 1 bytes for NUL termination. */
        char *tok = (char *)lotus_arena_alloc(arena, tok_len + 1, 1);
        if (!tok) return;
        memcpy(tok, src + start, tok_len);
        if (lowercase) {
            for (size_t j = 0; j < tok_len; j++) {
                if (tok[j] >= 'A' && tok[j] <= 'Z') tok[j] += 32;
            }
        }
        tok[tok_len] = '\0';
        /* Push the pointer (sizeof(char*) = sizeof(void*) = 8 on
         * 64-bit). lotus_vec_push memcpys `es` bytes from the
         * source; we point at &tok which is a stack temporary
         * whose address is fine across the call. */
        char *tok_ptr_for_push = tok;
        lotus_vec_push(target_vec, sizeof(char *), &tok_ptr_for_push);
    }
}

int lotus_hashmap_key_at(void *map_ptr, int64_t i, void *out_key) {
    if (!map_ptr || !out_key || i < 0) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if ((size_t)i >= m->len) return 0;
    size_t es = lotus_hashmap_entry_size(m);
    size_t seen = 0;
    for (size_t s = 0; s < m->cap; s++) {
        char *slot = m->slots + s * es;
        if (!slot[0]) continue;
        if (seen == (size_t)i) {
            memcpy(out_key, slot + 1, m->key_size);
            return 1;
        }
        seen++;
    }
    return 0;
}

int lotus_hashmap_value_at(void *map_ptr, int64_t i, void *out_value) {
    if (!map_ptr || !out_value || i < 0) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if ((size_t)i >= m->len) return 0;
    size_t es = lotus_hashmap_entry_size(m);
    size_t seen = 0;
    for (size_t s = 0; s < m->cap; s++) {
        char *slot = m->slots + s * es;
        if (!slot[0]) continue;
        if (seen == (size_t)i) {
            memcpy(out_value, slot + 1 + m->key_size, m->value_size);
            return 1;
        }
        seen++;
    }
    return 0;
}

void lotus_hashmap_destroy(void *map_ptr) {
    if (!map_ptr) return;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    free(m->slots);
    m->slots = NULL;
    m->cap = 0;
    m->len = 0;
}

/*
 * @form(ring_buffer) — fixed-capacity FIFO with push-back / pop-front.
 *
 * Pre-allocated at locus birth (single malloc of `cap × elem_size`
 * bytes); never grows. `push` returns 0 when the buffer is full
 * (caller decides drop vs. backpressure). `pop` returns 0 when
 * empty. Head/tail indices wrap modulo cap; the "ring" lives in
 * a flat contiguous buffer, no per-element link overhead.
 *
 * Layout matches the inline LLVM struct codegen emits:
 *
 *     struct lotus_ring_buffer {
 *         size_t cap;        // fixed at init; never changes
 *         size_t head;       // index of oldest element (next pop)
 *         size_t len;        // current element count (0..cap)
 *         size_t elem_size;  // bytes per element
 *         char  *buf;        // cap * elem_size bytes
 *     }
 *
 * The 5-field shape mirrors @form(vec)'s 3-field
 * { cap, len, buf } and @form(hashmap)'s 6-field
 * { cap, len, key_size, value_size, key_type_tag, slots } — same
 * "inline header + heap-malloc'd backing buffer" pattern, same
 * codegen-emits-inline-struct discipline. Fixed cap means no
 * doubling realloc; the entire buffer lives until the locus
 * dissolves.
 */

typedef struct lotus_ring_buffer {
    size_t cap;
    size_t head;
    size_t len;
    size_t elem_size;
    char  *buf;
} lotus_ring_buffer_t;

void lotus_ring_buffer_init(void *rb_ptr, size_t cap, size_t elem_size) {
    if (!rb_ptr) return;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    rb->cap = cap;
    rb->head = 0;
    rb->len = 0;
    rb->elem_size = elem_size;
    /* malloc rather than calloc — push always writes before any
     * read sees the slot. */
    rb->buf = (char *)malloc(cap * elem_size);
}

/* Returns 1 on success, 0 when full. v1 contract: "push returns
 * false when full"; the spec preview names the synthesized method
 * `push(x: T) -> Bool` (infallible — full is a Bool result, not
 * a fallible error). */
int lotus_ring_buffer_push(void *rb_ptr, const void *src) {
    if (!rb_ptr || !src) return 0;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    if (rb->len == rb->cap) return 0;
    size_t tail = (rb->head + rb->len) % rb->cap;
    memcpy(rb->buf + tail * rb->elem_size, src, rb->elem_size);
    rb->len++;
    return 1;
}

/* Returns 1 on success (and writes `elem_size` bytes into `out`),
 * 0 when empty. The synthesized `pop()` codegen converts the
 * Bool into the fallible(EmptyError) shape. */
int lotus_ring_buffer_pop(void *rb_ptr, void *out) {
    if (!rb_ptr || !out) return 0;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    if (rb->len == 0) return 0;
    memcpy(out, rb->buf + rb->head * rb->elem_size, rb->elem_size);
    rb->head = (rb->head + 1) % rb->cap;
    rb->len--;
    return 1;
}

size_t lotus_ring_buffer_len(void *rb_ptr) {
    if (!rb_ptr) return 0;
    return ((lotus_ring_buffer_t *)rb_ptr)->len;
}

int lotus_ring_buffer_is_full(void *rb_ptr) {
    if (!rb_ptr) return 0;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    return rb->len == rb->cap ? 1 : 0;
}

void lotus_ring_buffer_destroy(void *rb_ptr) {
    if (!rb_ptr) return;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    free(rb->buf);
    rb->buf = NULL;
    rb->cap = 0;
    rb->head = 0;
    rb->len = 0;
}

/*
 * Cooperative scheduler — bus dispatch queue (m26 + m28b stage 1).
 *
 * Per The Design / lotus, every bus dispatch is a substrate
 * cell. The cooperative scheduler enqueues these cells at
 * publish time and pops them one at a time at drain time.
 * Each pop runs the handler to completion (handler-atomic;
 * cooperative yields BETWEEN cells, not within). Handlers may
 * publish more events, which enqueue more cells; drain
 * continues until the queue is empty.
 *
 * m28b stage 1 changed cell shape: cells now carry an INLINE
 * payload buffer instead of a pointer to subscriber-arena
 * memory. This is the prerequisite for cross-thread bus: the
 * publisher can be on a different thread than the subscriber,
 * so the payload can't live in either arena (each arena is
 * single-threaded territory). The boundary IS the queue —
 * inline payload makes the queue the single point of cross-
 * thread synchronization. Drain copies inline → subscriber's
 * arena before invoking the handler, so the per-spec/memory.md
 * "every locus boundary copies the payload" rule still holds:
 * the subscriber gets its own arena-resident copy that outlives
 * the publisher.
 *
 * Cost vs m26: every cell does TWO memcpy's (publisher → cell
 * inline + cell inline → subscriber arena) instead of one
 * (publisher → subscriber arena). For the small typed messages
 * lotus carries this is negligible; cross-thread correctness
 * is worth more than one memcpy.
 *
 * Mutex protects the cell array so pinned threads can enqueue
 * concurrently with the cooperative drain (m28b stage 2). v0
 * uses a single mutex around enqueue + each pop. Drain releases
 * the lock around handler invocation so handlers can re-enqueue
 * without self-deadlock (and so cooperative handlers don't
 * block pinned producers for their entire run-time).
 *
 * Inline payload size cap: LOTUS_PAYLOAD_MAX bytes per cell.
 * Larger payloads abort at enqueue (v0 limitation).
 */

#define LOTUS_PAYLOAD_MAX 512

typedef struct lotus_bus_cell {
    void  *handler;                       /* void (*)(void *self, void *payload) */
    void  *self_ptr;                      /* subscriber's locus ptr */
    size_t payload_size;                  /* bytes used in inline */
    char   payload_inline[LOTUS_PAYLOAD_MAX];
} lotus_bus_cell_t;

typedef struct lotus_bus_queue {
    lotus_bus_cell_t *cells;
    size_t            head;     /* next slot to pop */
    size_t            tail;     /* next slot to fill */
    size_t            cap;
    pthread_mutex_t   lock;
} lotus_bus_queue_t;

#define LOTUS_BUS_QUEUE_INITIAL_CAP 64

/* Pure-cooperative fast path. Set to non-zero before any pinned
 * thread starts; codegen emits a call to `lotus_bus_mark_pinned`
 * at every pinned-locus instantiation (sync, before pthread_create,
 * so the new thread can never observe the flag unset on its
 * publish path). When zero, every enqueue and pop happens on a
 * single thread (the cooperative scheduler's main), so the queue
 * mutex is dead overhead — ~20-40ns/event on uncontended lock+
 * unlock pair. The flag is monotonic 0→1; once any pinned locus
 * exists, contention is possible and we lock normally for the
 * rest of the program. */
static int g_bus_has_pinned = 0;

void lotus_bus_mark_pinned(void) {
    g_bus_has_pinned = 1;
}

lotus_bus_queue_t *lotus_bus_queue_create(void) {
    lotus_bus_queue_t *q =
        (lotus_bus_queue_t *)malloc(sizeof(lotus_bus_queue_t));
    if (!q) return NULL;
    q->cap   = LOTUS_BUS_QUEUE_INITIAL_CAP;
    q->cells = (lotus_bus_cell_t *)
        malloc(q->cap * sizeof(lotus_bus_cell_t));
    if (!q->cells) {
        free(q);
        return NULL;
    }
    q->head = 0;
    q->tail = 0;
    pthread_mutex_init(&q->lock, NULL);
    return q;
}

/* Enqueue (handler, self, payload_src + payload_size). The
 * publisher's payload is memcpy'd into the cell's inline
 * buffer; the cell does NOT carry a pointer back to publisher
 * memory. After enqueue returns, the publisher is free to
 * dissolve / reuse / overwrite the payload source — the queue
 * holds the canonical copy until drain re-copies it into the
 * subscriber's arena.
 *
 * Holds the queue's mutex for the duration so concurrent pinned
 * publishers don't corrupt each other's writes. */
void lotus_bus_queue_enqueue(lotus_bus_queue_t *q,
                             void *handler,
                             void *self_ptr,
                             const void *payload_src,
                             size_t payload_size) {
    if (!q) return;
    if (payload_size > LOTUS_PAYLOAD_MAX) {
        /* v0 limitation — payloads above 512 bytes need spill-
         * to-malloc support that isn't here yet. Drop silently;
         * better-than-corrupting. */
        return;
    }
    int locked = g_bus_has_pinned;
    if (locked) pthread_mutex_lock(&q->lock);
    if (q->tail == q->cap) {
        /* Compact first: slide live cells to the front. */
        size_t live = q->tail - q->head;
        if (q->head > 0) {
            memmove(q->cells, q->cells + q->head,
                    live * sizeof(lotus_bus_cell_t));
            q->head = 0;
            q->tail = live;
        }
        if (q->tail == q->cap) {
            /* Truly full — double the capacity. */
            size_t new_cap = q->cap * 2;
            lotus_bus_cell_t *new_cells = (lotus_bus_cell_t *)
                realloc(q->cells, new_cap * sizeof(lotus_bus_cell_t));
            if (!new_cells) {
                if (locked) pthread_mutex_unlock(&q->lock);
                return;     /* drop on OOM */
            }
            q->cells = new_cells;
            q->cap   = new_cap;
        }
    }
    lotus_bus_cell_t *slot = &q->cells[q->tail++];
    slot->handler      = handler;
    slot->self_ptr     = self_ptr;
    slot->payload_size = payload_size;
    if (payload_size > 0 && payload_src) {
        memcpy(slot->payload_inline, payload_src, payload_size);
    }
    if (locked) pthread_mutex_unlock(&q->lock);
}

/* Drain the queue: pop cells one at a time and invoke
 * handler(self, payload). Handlers may enqueue more cells
 * (cooperative-cooperative bus dispatch is the natural
 * interleaving — see The Design / lotus, substrate cells).
 * Loops until the queue is empty AT POP TIME, including any
 * cells enqueued during the drain itself.
 *
 * Payload-pointer lifetime: the pointer handed to the handler
 * is valid for the duration of that handler invocation only.
 * The handler reads field values out of it (typical pattern:
 * `self.total = self.total + payload.value`); field copies
 * land in self, the pointer itself does not escape. Aperio
 * doesn't allow taking explicit addresses in user code, so
 * this invariant is structurally enforced. Per spec/memory.md
 * § "Bus dispatch: copy-not-pointer semantic", the *value*
 * crosses the locus boundary (via the cell's inline buffer);
 * what changes here is that the value no longer bounces
 * through the subscriber's arena before the handler reads it
 * — a per-event `lotus_arena_alloc` + second `memcpy` that
 * dominated the cost for the small-payload event-flood case
 * (`bus_dispatch`/`stream_aggregator`/`pipeline_3stage`-style).
 *
 * Lock discipline (locked path): take the mutex to pop one cell
 * INTO a stack-local snapshot; release before invoking the
 * handler. The snapshot's `payload_inline` field IS the
 * canonical copy for this dispatch — handler reads through
 * `&snapshot.payload_inline`. Holding the lock across handler
 * invocation would (a) block pinned producers for the entire
 * handler runtime and (b) deadlock if the handler re-enqueues.
 *
 * Single-threaded path: a single stack buffer outside the loop
 * receives the cell's payload before each handler invocation.
 * Required because the handler may publish, which may realloc
 * `q->cells`, which would dangle a direct pointer into the
 * cell. Recursive drain calls (via the handler's trailing
 * bus_drain) get their own stack frame and their own buffer. */
typedef void (*lotus_handler_fn)(void *self, void *payload);

void lotus_bus_queue_drain(lotus_bus_queue_t *q) {
    if (!q) return;
    int locked = g_bus_has_pinned;
    if (locked) {
        /* Concurrent producers possible — must snapshot each cell
         * under the lock so the cells array can't be realloc'd out
         * from under the in-flight pop. The snapshot's inline
         * buffer is what the handler reads. */
        for (;;) {
            pthread_mutex_lock(&q->lock);
            if (q->head >= q->tail) {
                q->head = 0;
                q->tail = 0;
                pthread_mutex_unlock(&q->lock);
                return;
            }
            lotus_bus_cell_t cell_copy = q->cells[q->head++];
            pthread_mutex_unlock(&q->lock);

            void *payload_ptr = (cell_copy.payload_size > 0)
                ? (void *)cell_copy.payload_inline
                : NULL;
            ((lotus_handler_fn)cell_copy.handler)(
                cell_copy.self_ptr, payload_ptr);
        }
    } else {
        /* Single-threaded cooperative path: no concurrent producer
         * exists. One stack-allocated payload buffer, reused
         * across iterations and stable across recursive drain
         * calls (the recursive call has its own frame). */
        unsigned char stack_payload[LOTUS_PAYLOAD_MAX]
            __attribute__((aligned(16)));
        for (;;) {
            if (q->head >= q->tail) {
                q->head = 0;
                q->tail = 0;
                return;
            }
            lotus_bus_cell_t *cell = &q->cells[q->head++];
            void *handler_fn = cell->handler;
            void *handler_self = cell->self_ptr;
            size_t psize = cell->payload_size;
            void *payload_ptr = NULL;
            if (psize > 0) {
                /* Last cell-dereference before invoking the
                 * handler. After this memcpy, any handler-side
                 * realloc of q->cells is harmless — we're done
                 * reading from `cell`. */
                memcpy(stack_payload, cell->payload_inline, psize);
                payload_ptr = stack_payload;
            }
            ((lotus_handler_fn)handler_fn)(handler_self, payload_ptr);
        }
    }
}

void lotus_bus_queue_destroy(lotus_bus_queue_t *q) {
    if (!q) return;
    pthread_mutex_destroy(&q->lock);
    if (q->cells) free(q->cells);
    free(q);
}

/*
 * Per-pinned-locus mailbox (m28b stage 2).
 *
 * Each pinned locus that declares `bus subscribe` gets its own
 * mailbox: same cell shape as the global queue, plus a condvar
 * + shutdown flag. Cross-thread publishers (cooperative or
 * pinned) call lotus_mailbox_post to drop a cell into the
 * subscriber's mailbox; the pinned thread's main loop calls
 * lotus_mailbox_drain_one to pull one cell at a time, copy
 * its inline payload into the locus's arena, and invoke the
 * handler — handler-atomic per substrate cell, just like the
 * cooperative drain.
 *
 * post → broadcasts the not_empty condvar so a thread waiting
 * in drain_one wakes up.
 *
 * drain_one blocks on the condvar until either:
 *   - a cell arrives (returns 1 after invoking the handler), or
 *   - shutdown is signaled and the queue is empty (returns 0).
 *
 * shutdown sets the flag + broadcasts so all waiters return.
 * The pinned thread observes return=0, breaks its loop, runs
 * its drain/dissolve, and exits — main thread then joins.
 *
 * Per The Design / lotus, this is the canonical "any → pinned"
 * bus path: publisher and subscriber sit in different layers
 * of the lotus, the cost lives at the boundary (the mailbox
 * lock + the inline payload's two memcpy's), and each arena
 * stays single-threaded territory.
 */

typedef struct lotus_mailbox {
    lotus_bus_cell_t *cells;
    size_t            head;
    size_t            tail;
    size_t            cap;
    int               shutdown;
    pthread_mutex_t   lock;
    pthread_cond_t    not_empty;
} lotus_mailbox_t;

#define LOTUS_MAILBOX_INITIAL_CAP 64

lotus_mailbox_t *lotus_mailbox_create(void) {
    lotus_mailbox_t *mb =
        (lotus_mailbox_t *)malloc(sizeof(lotus_mailbox_t));
    if (!mb) return NULL;
    mb->cap   = LOTUS_MAILBOX_INITIAL_CAP;
    mb->cells = (lotus_bus_cell_t *)
        malloc(mb->cap * sizeof(lotus_bus_cell_t));
    if (!mb->cells) {
        free(mb);
        return NULL;
    }
    mb->head     = 0;
    mb->tail     = 0;
    mb->shutdown = 0;
    pthread_mutex_init(&mb->lock, NULL);
    pthread_cond_init(&mb->not_empty, NULL);
    return mb;
}

void lotus_mailbox_post(lotus_mailbox_t *mb,
                        void *handler,
                        void *self_ptr,
                        const void *payload_src,
                        size_t payload_size) {
    if (!mb) return;
    if (payload_size > LOTUS_PAYLOAD_MAX) {
        return;     /* v0 limit */
    }
    pthread_mutex_lock(&mb->lock);
    if (mb->tail == mb->cap) {
        size_t live = mb->tail - mb->head;
        if (mb->head > 0) {
            memmove(mb->cells, mb->cells + mb->head,
                    live * sizeof(lotus_bus_cell_t));
            mb->head = 0;
            mb->tail = live;
        }
        if (mb->tail == mb->cap) {
            size_t new_cap = mb->cap * 2;
            lotus_bus_cell_t *new_cells = (lotus_bus_cell_t *)
                realloc(mb->cells, new_cap * sizeof(lotus_bus_cell_t));
            if (!new_cells) {
                pthread_mutex_unlock(&mb->lock);
                return;
            }
            mb->cells = new_cells;
            mb->cap   = new_cap;
        }
    }
    lotus_bus_cell_t *slot = &mb->cells[mb->tail++];
    slot->handler      = handler;
    slot->self_ptr     = self_ptr;
    slot->payload_size = payload_size;
    if (payload_size > 0 && payload_src) {
        memcpy(slot->payload_inline, payload_src, payload_size);
    }
    pthread_cond_broadcast(&mb->not_empty);
    pthread_mutex_unlock(&mb->lock);
}

int lotus_mailbox_drain_one(lotus_mailbox_t *mb) {
    if (!mb) return 0;
    pthread_mutex_lock(&mb->lock);
    while (mb->head >= mb->tail && !mb->shutdown) {
        pthread_cond_wait(&mb->not_empty, &mb->lock);
    }
    if (mb->head >= mb->tail) {
        /* shutdown with empty queue */
        mb->head = 0;
        mb->tail = 0;
        pthread_mutex_unlock(&mb->lock);
        return 0;
    }
    lotus_bus_cell_t cell_copy = mb->cells[mb->head++];
    if (mb->head >= mb->tail) {
        mb->head = 0;
        mb->tail = 0;
    }
    pthread_mutex_unlock(&mb->lock);

    /* Hand `cell_copy.payload_inline` directly to the handler.
     * `cell_copy` is a stack-local snapshot of the dequeued cell;
     * its inline buffer is the canonical payload copy for this
     * dispatch. Skipping the prior `lotus_arena_alloc` + extra
     * memcpy into the locus's arena drops the per-event overhead
     * on the pinned-subscriber path. See the matching note in
     * lotus_bus_queue_drain — same lifetime invariant. */
    void *payload_ptr = (cell_copy.payload_size > 0)
        ? (void *)cell_copy.payload_inline
        : NULL;
    ((lotus_handler_fn)cell_copy.handler)(
        cell_copy.self_ptr, payload_ptr);
    return 1;
}

void lotus_mailbox_shutdown(lotus_mailbox_t *mb) {
    if (!mb) return;
    pthread_mutex_lock(&mb->lock);
    mb->shutdown = 1;
    pthread_cond_broadcast(&mb->not_empty);
    pthread_mutex_unlock(&mb->lock);
}

void lotus_mailbox_destroy(lotus_mailbox_t *mb) {
    if (!mb) return;
    pthread_cond_destroy(&mb->not_empty);
    pthread_mutex_destroy(&mb->lock);
    if (mb->cells) free(mb->cells);
    free(mb);
}

/*
 * Process-wide bus router (m45-followup proper-fix).
 *
 * Replaces the per-program LLVM-side {bus.entries, bus.count,
 * lotus.bus_dispatch} triple. Storage is a heap-allocated dynamic
 * vec that grows on demand, so adding a new subscription has no
 * compile-time-known capacity ceiling. Multiple instances of the
 * same subscribed locus type each get their own entry without
 * needing the m45 quickfix's INSTANCES_PER_TYPE multiplier.
 *
 * Entry shape mirrors the prior LLVM struct exactly: subject (NUL
 * marks deregistered, courtesy of `lotus_bus_quarantine_self`),
 * subscriber's locus self pointer, handler fn pointer, and an
 * optional mailbox (null = cooperative subscriber → cells go to
 * the program-wide queue; non-null = pinned subscriber → cells
 * post to that locus's mailbox).
 *
 * No mutex on the router itself: registration runs inside
 * single-threaded instantiation paths, dispatch's payload-copy
 * happens through the queue/mailbox locks, and quarantine runs on
 * the cooperative thread (pinned loci don't have closures so
 * never quarantine). If pinned-side registration ever lands, this
 * acquires a mutex.
 */

/* m60: per-payload deserializer signature. Codegen synthesizes
 * `__deserialize_T` per bus payload type and passes the fn ptr
 * to lotus_bus_register; the reader thread (m59) calls it on
 * recv'd wire bytes to reconstruct the struct before dispatching
 * to the local handler. v0.1 wire format is identity (memcpy of
 * sizeof(T) bytes), so the reconstructed struct equals the
 * publisher's original struct. Returns the size written into
 * `dst` on success, -1 on error. */
typedef ssize_t (*lotus_deserialize_fn)(const void *src,
                                        size_t n,
                                        void *dst,
                                        size_t cap);

typedef struct lotus_bus_entry {
    const char           *subject;
    void                 *self_ptr;
    void                 *handler;
    lotus_mailbox_t      *mailbox;
    lotus_deserialize_fn  deserialize;     /* m60: nullable */
} lotus_bus_entry_t;

static lotus_bus_entry_t *g_bus_entries = NULL;
static size_t             g_bus_count   = 0;
static size_t             g_bus_cap     = 0;

#define LOTUS_BUS_ROUTER_INITIAL_CAP 16

/* m94: subject wildcard matching.
 *
 * v0 supports one wildcard form: a trailing "**" that matches
 * zero or more remaining dot-separated segments. So "log.app.**"
 * matches "log.app" (the root), "log.app.db", "log.app.db.query"
 * — the publishing logger's own subject AND any descendant.
 * This is the cascade-friendly semantics: subscribing to
 * `log.app.**` captures the whole sub-tree including its root.
 *
 * "**" must appear at the end of the pattern, preceded either by
 * "." or by nothing (the bare "**" pattern matches every subject).
 * "**" in any other position rejects.
 *
 * Returns 1 on match, 0 otherwise. NULL inputs are treated as
 * non-matching. Patterns without "**" fall through to strcmp —
 * the cheap path stays cheap.
 */
int lotus_subject_match(const char *pattern, const char *subject) {
    if (!pattern || !subject) return 0;
    /* Pointer-equal fast path: both sides typically reference the
     * same merged `unnamed_addr` global. LLVM coalesces identical
     * string constants, so `subscribe "S"` + `<- "S"` use the
     * same address. Skips strlen + strstr + strcmp for the
     * common literal-subject case (`bus_dispatch` / `stream_*`
     * patterns) — ~5-10 ns/publish-per-subscriber on a no-
     * wildcard subject. */
    if (pattern == subject) return 1;
    size_t plen = strlen(pattern);
    if (plen < 2) {
        /* Too short to contain "**". */
        return strcmp(pattern, subject) == 0;
    }
    /* "**" is supported only as a trailing wildcard. Anywhere
     * else we treat as no match (rather than try-and-fail
     * matching) so a typo like "log.**.error" doesn't silently
     * match a stray subject. */
    if (pattern[plen - 1] == '*' && pattern[plen - 2] == '*') {
        if (plen == 2) {
            /* Bare "**" — matches every subject. */
            return 1;
        }
        /* Must be preceded by '.', else "log**" — invalid. */
        if (pattern[plen - 3] != '.') return 0;
        /* Pattern is "<root>." + "**". Two valid forms:
         *   - subject equals root (no trailing segments)
         *   - subject starts with "<root>." and has tail bytes
         */
        size_t root_len = plen - 3;        /* "<root>" length */
        size_t prefix_len = plen - 2;      /* "<root>." length */
        if (strlen(subject) == root_len &&
            strncmp(pattern, subject, root_len) == 0) {
            return 1;
        }
        if (strncmp(pattern, subject, prefix_len) != 0) return 0;
        return subject[prefix_len] != '\0';
    }
    /* Pattern contains "**" but not at the end — reject. */
    if (strstr(pattern, "**") != NULL) return 0;
    /* No wildcard. */
    return strcmp(pattern, subject) == 0;
}

/* m58: forward-declare the remote-transport fanout hooks defined
 * at the bottom of this file. Dispatch and router_destroy call
 * them after the local-table loops so cross-process subscribers
 * receive the same publishes that local subscribers do. The
 * remote table and load-config implementation live next to the
 * AF_UNIX transport section because they're tightly coupled to
 * lotus_transport_create / send / destroy. */
void lotus_bus_remote_fanout(const char *subject,
                             const void *payload,
                             size_t payload_size);
void lotus_bus_remote_destroy_all(void);

/* m59: subscriber-side reader thread support.
 *
 * Reader threads (one per LISTEN-role transport opened from the
 * deployment-config) loop on lotus_transport_recv and need to
 * dispatch incoming bytes into the same local-handler set that
 * in-process publishers reach via lotus_bus_dispatch. To do
 * that without plumbing the cooperative queue pointer through
 * the transport layer, codegen sets it on a global at boot via
 * lotus_bus_set_queue, and the reader thread calls
 * lotus_bus_local_dispatch which reads the global. Pinned
 * subscribers route via mailbox (thread-safe by construction);
 * cooperative subscribers enqueue onto the cooperative queue
 * (mutex-protected, see lotus_bus_queue_enqueue), so the
 * reader thread is safely a producer alongside main + any
 * pinned threads. */
void lotus_bus_local_dispatch(lotus_bus_queue_t *queue,
                              const char *subject,
                              const void *payload,
                              size_t payload_size);
void lotus_bus_set_queue(lotus_bus_queue_t *queue);

void lotus_bus_register(const char *subject,
                        void *self_ptr,
                        void *handler,
                        lotus_mailbox_t *mailbox,
                        lotus_deserialize_fn deserialize) {
    if (g_bus_count == g_bus_cap) {
        size_t new_cap = g_bus_cap == 0
            ? LOTUS_BUS_ROUTER_INITIAL_CAP
            : g_bus_cap * 2;
        lotus_bus_entry_t *grown = (lotus_bus_entry_t *)
            realloc(g_bus_entries, new_cap * sizeof(lotus_bus_entry_t));
        if (!grown) return;     /* drop on OOM — graceful degrade */
        g_bus_entries = grown;
        g_bus_cap     = new_cap;
    }
    lotus_bus_entry_t *e = &g_bus_entries[g_bus_count++];
    e->subject     = subject;
    e->self_ptr    = self_ptr;
    e->handler     = handler;
    e->mailbox     = mailbox;
    e->deserialize = deserialize;
}

/* Dispatch a published message to every subscriber of `subject`.
 * `queue` is the program-wide cooperative queue (passed in by
 * codegen rather than C-runtime-owned because the queue's
 * lifecycle is bound to main's prelude/exit, not to whatever
 * first triggers a register). Pinned subscribers route via their
 * mailbox; cooperative subscribers enqueue onto `queue`. */
/* m59 refactor: extracted from lotus_bus_dispatch so the m59
 * reader thread can dispatch recv'd bytes into the same local-
 * handler set without going through transport fanout (which
 * would re-emit them remotely and loop forever). */
void lotus_bus_local_dispatch(lotus_bus_queue_t *queue,
                              const char *subject,
                              const void *payload,
                              size_t payload_size) {
    if (!subject) return;
    for (size_t i = 0; i < g_bus_count; i++) {
        lotus_bus_entry_t *e = &g_bus_entries[i];
        if (!e->subject) continue;          /* deregistered */
        /* m94: pattern-match in case the subscriber registered a
         * wildcard subject (e.g. "log.**"). The fast path —
         * pattern with no '**' — costs one strcmp. */
        if (!lotus_subject_match(e->subject, subject)) continue;
        if (e->mailbox) {
            lotus_mailbox_post(e->mailbox, e->handler, e->self_ptr,
                               payload, payload_size);
        } else if (queue) {
            lotus_bus_queue_enqueue(queue, e->handler, e->self_ptr,
                                    payload, payload_size);
        }
    }
}

/* m105 (Wave B inbound): adapter-driven inbound dispatch.
 *
 * The symmetric inbound counterpart to lotus_bus_remote_fanout's
 * outbound path. An adapter locus's run-loop (or any user code
 * driving an adapter) calls this with the wire-format bytes it
 * just received from the protocol layer; the runtime looks up
 * the subject's deserialize fn (registered alongside the local
 * subscribers via codegen) to convert wire bytes back to in-
 * memory struct bytes, then hands those to lotus_bus_local_dispatch
 * for fanout into the local handler set.
 *
 * Mirrors the unix reader-thread path in
 * lotus_bus_reader_thread_main but exposed as a callable for
 * Aperio code via the `std::bus::__local_dispatch` primitive.
 * Out-of-band (non-bus) recv loops can use this too; the only
 * contract is "wire_bytes is one whole serialized payload."
 *
 * Silent no-op if no subscriber for the subject (matches the
 * unix path) or if the wire-bytes fail deserialization.
 *
 * Forward decl here; body lives further down once
 * g_bus_queue_for_remote is in scope (parallel to the
 * lotus_bus_remote_fanout split).
 */
void lotus_bus_dispatch_wire(const char *subject,
                             const void *wire_bytes,
                             size_t wire_size);

/* m70: lotus_bus_dispatch's signature grew a 5th arg — a per-
 * subject serialize fn pointer (NULL for cooperative-only
 * publishers; codegen always passes the right one for cross-
 * process-capable subjects). Local dispatch enqueues struct
 * bytes (in-memory layout); remote fanout serializes those
 * bytes through the supplied fn into the wire format the
 * reader thread will deserialize. Splitting local-vs-remote
 * here lets the wire format diverge from the in-memory struct
 * layout (variable-width Strings, length-prefixed) without
 * breaking the local in-process path. */
typedef ssize_t (*lotus_serialize_fn)(const void *src,
                                       void *dst,
                                       size_t cap);

/* Forward decl — the remote-entries table is defined further
 * down in this file. `lotus_bus_dispatch` checks this to skip
 * the serialize+fanout work when no remote subscribers exist. */
static inline int lotus_bus_has_remote_entries(void);

void lotus_bus_dispatch(lotus_bus_queue_t *queue,
                        const char *subject,
                        const void *struct_payload,
                        size_t struct_size,
                        lotus_serialize_fn serialize_fn) {
    /* Local fanout: enqueue struct bytes verbatim. Same shape
     * as the pre-m70 path — String fields inside the struct
     * are pointers into the publisher's arena; the local
     * subscriber's drain copies struct bytes into its own arena
     * and the handler reads through those pointers (which are
     * still valid in-process). */
    lotus_bus_local_dispatch(queue, subject, struct_payload, struct_size);

    /* Remote fanout: serialize struct → wire bytes (per-field
     * walk; codegen synthesizes the body), then dispatch to
     * each CONNECT-role transport bound to this subject via
     * the existing lotus_bus_remote_fanout iteration.
     *
     * Skip the serialize-call entirely when no remote entries
     * are configured at all (the common case — most programs
     * never set LOTUS_BUS_CONFIG). The serialize walks the
     * payload's fields into wire_buf, which costs ~10-30ns
     * per publish even for an 8-byte payload like Tick, and
     * the resulting bytes would be discarded by an empty
     * remote-fanout loop. Removing that work cuts ~20% off
     * `bus_dispatch` on cooperative-only programs.
     *
     * m58: local + remote share the same subject namespace per
     * notes/open-questions #9 (emergent cardinality). */
    if (!serialize_fn) return;
    if (!lotus_bus_has_remote_entries()) return;
    char wire_buf[LOTUS_PAYLOAD_MAX];
    ssize_t wire_size = serialize_fn(struct_payload, wire_buf,
                                     sizeof(wire_buf));
    if (wire_size <= 0) return;
    lotus_bus_remote_fanout(subject, wire_buf, (size_t)wire_size);
}

/* m41b semantic: null-out subject for any entry whose self
 * matches `self_ptr`. Subsequent `lotus_bus_dispatch` calls skip
 * those slots — quarantined subscribers stop receiving messages. */
void lotus_bus_quarantine_self(void *self_ptr) {
    for (size_t i = 0; i < g_bus_count; i++) {
        if (g_bus_entries[i].self_ptr == self_ptr) {
            g_bus_entries[i].subject = NULL;
        }
    }
}

void lotus_bus_router_destroy(void) {
    if (g_bus_entries) free(g_bus_entries);
    g_bus_entries = NULL;
    g_bus_count   = 0;
    g_bus_cap     = 0;
    /* m58: also tear down any remote-bound transports the
     * deployment-config loader opened at boot. */
    lotus_bus_remote_destroy_all();
}

/*
 * Pinned-thread CPU affinity helper (m28c).
 *
 * `: schedule pinned(core=N)` annotations route through here:
 * codegen emits a call to lotus_set_core_affinity right after
 * pthread_create succeeds, with the user-declared core index.
 * We wrap pthread_setaffinity_np behind a stable C helper so
 * codegen doesn't have to construct a cpu_set_t directly
 * (cpu_set_t is opaque + size-variable across glibc versions).
 *
 * If the affinity call fails (e.g., core index out of range,
 * permission denied in restricted environments) we silently
 * succeed — the thread runs without affinity, falling back to
 * normal OS scheduling. v0 prefers "best effort" over hard-
 * error here so a CI box with fewer cores than the source
 * declares doesn't refuse to start the binary.
 */
void lotus_set_core_affinity(unsigned long tid, int core) {
    cpu_set_t cpuset;
    CPU_ZERO(&cpuset);
    CPU_SET(core, &cpuset);
    (void)pthread_setaffinity_np(
        (pthread_t)tid, sizeof(cpu_set_t), &cpuset);
}

/*
 * Pinned-thread entry (m28a + m28b).
 *
 * The C-runtime adapter `lotus_thread_entry` is gone — m28a
 * synthesizes a per-locus `__pinned_main_<LocusName>` LLVM
 * function whose signature is exactly pthread's `void *(*)(void *)`.
 * That function takes self_ptr as its sole argument and runs
 * birth → run → (mailbox loop) → drain → dissolve in sequence
 * (each only if the locus declared it) before returning NULL.
 * The mailbox loop is included only when the pinned locus
 * declares `bus subscribe`; the codegen branches on that at
 * compile time (m28b).
 */

/*
 * String helpers (m36).
 *
 * Strings in the codegen are NUL-terminated byte arrays. A
 * literal lives as a module-global; a concat / slice result
 * lives in an arena, owned by the caller's locus. All string
 * ops preserve the "value is a pointer" shape — Codegen's
 * CodegenTy::String maps to a basic ptr_t at the LLVM level
 * regardless of provenance.
 *
 * Lifetimes follow the spec/memory.md region rule: results land
 * in whatever arena the caller passes (their current locus's
 * arena, or the program-wide arena in `main` and free fns).
 * No per-string free; the arena's wholesale destroy reclaims
 * everything together.
 */
char *lotus_str_concat(lotus_arena_t *a, const char *l, const char *r) {
    size_t lL = strlen(l);
    size_t lR = strlen(r);
    char *out = (char *)lotus_arena_alloc(a, lL + lR + 1, 1);
    if (!out) return NULL;
    memcpy(out, l, lL);
    memcpy(out + lL, r, lR);
    out[lL + lR] = '\0';
    return out;
}

int lotus_str_eq(const char *l, const char *r) {
    return strcmp(l, r) == 0 ? 1 : 0;
}

/* m49: deep-copy a string into the destination arena. Used at
 * free-fn return boundaries: the body's subregion is about to be
 * destroyed, so any String the body returns gets cloned into the
 * caller's arena first. The returned pointer outlives the
 * subregion destroy. Same shape as concat with a NULL right side
 * — kept as a separate symbol so the call-site IR is one helper
 * call, not a concat-with-empty-literal dance. */
char *lotus_str_clone(lotus_arena_t *a, const char *s) {
    size_t n = strlen(s);
    char *out = (char *)lotus_arena_alloc(a, n + 1, 1);
    if (!out) return NULL;
    memcpy(out, s, n);
    out[n] = '\0';
    return out;
}

int64_t lotus_str_len(const char *s) {
    return (int64_t)strlen(s);
}

/*
 * Substring `s[lo..hi]` with exclusive `hi`. Bounds clamp so
 * out-of-range indices produce a (possibly empty) string rather
 * than crashing — matches the interpreter and avoids a forced
 * runtime panic for off-by-one mistakes. Result is a fresh
 * arena-owned NUL-terminated copy.
 */
char *lotus_str_slice(lotus_arena_t *a, const char *s,
                      int64_t lo, int64_t hi) {
    int64_t n = (int64_t)strlen(s);
    if (lo < 0) lo = 0;
    if (lo > n) lo = n;
    if (hi < lo) hi = lo;
    if (hi > n) hi = n;
    int64_t len = hi - lo;
    char *out = (char *)lotus_arena_alloc(a, (size_t)len + 1, 1);
    if (!out) return NULL;
    if (len > 0) {
        memcpy(out, s + lo, (size_t)len);
    }
    out[len] = '\0';
    return out;
}

/*
 * to_string helpers (m37). Each renders one primitive into a
 * fresh NUL-terminated arena buffer using the same printf-style
 * format that `println` uses, so a value written via to_string
 * + concat reads identical to the same value passed to println.
 *
 * Buffer sizes:
 *   - i64  → max 20 digits + sign + NUL = 22 bytes; round up.
 *   - %g   → typical max ~24 chars for normal magnitudes; 32
 *     covers headroom for denormals and -DBL_MAX.
 *   - duration → i64 + "ns" suffix.
 */
char *lotus_str_from_int(lotus_arena_t *a, int64_t n) {
    char *out = (char *)lotus_arena_alloc(a, 32, 1);
    if (!out) return NULL;
    snprintf(out, 32, "%lld", (long long)n);
    return out;
}

char *lotus_str_from_float(lotus_arena_t *a, double f) {
    char *out = (char *)lotus_arena_alloc(a, 32, 1);
    if (!out) return NULL;
    snprintf(out, 32, "%g", f);
    return out;
}

char *lotus_str_from_duration(lotus_arena_t *a, int64_t ns) {
    char *out = (char *)lotus_arena_alloc(a, 32, 1);
    if (!out) return NULL;
    snprintf(out, 32, "%lldns", (long long)ns);
    return out;
}

/*
 * starts_with / contains (m38).
 *
 * Both return i32 0/1 (codegen truncates to i1). Empty
 * prefix / sub matches any string (matches Rust semantics).
 * No locale folding — byte-exact comparison so the result
 * doesn't drift across systems.
 */
int lotus_str_starts_with(const char *s, const char *prefix) {
    if (!s || !prefix) return 0;
    size_t lp = strlen(prefix);
    if (lp == 0) return 1;
    return strncmp(s, prefix, lp) == 0 ? 1 : 0;
}

int lotus_str_contains(const char *s, const char *sub) {
    if (!s || !sub) return 0;
    if (*sub == '\0') return 1;
    return strstr(s, sub) ? 1 : 0;
}

/*
 * m84: byte index of first occurrence of `sub` in `s`, or -1 if
 * not found. Mirrors lotus_str_contains's strstr-based search but
 * returns the position rather than just a presence flag — needed
 * by Phase 3's HTTP request parser (locating ` ` between method
 * and path, `\r\n` at the end of the request line, etc.). Empty
 * needle returns 0 by convention; null inputs return -1.
 */
/*
 * m89: Bytes value primitives.
 *
 * A Bytes value is a single arena-allocated pointer to a blob
 * laid out as `[i64 len][u8 data[len]]`. The leading length
 * makes the value self-describing — same single-pointer ABI
 * as String, but binary content with embedded NUL bytes
 * doesn't truncate (NUL is not a terminator here).
 *
 * Memory: allocated via lotus_arena_alloc on the caller's
 * arena, so the lifetime matches the locus or fn whose arena
 * it came from. v0 has no resize/append — Bytes is created
 * once with a known length (via read, recv, etc.) and lives
 * as long as the caller's arena does.
 */
void *lotus_bytes_create(lotus_arena_t *a, int64_t len) {
    if (len < 0) {
        return NULL;
    }
    /* sizeof(int64_t) for the prefix + len bytes for the body. */
    size_t blob = sizeof(int64_t) + (size_t)len;
    void *p = lotus_arena_alloc(a, blob, 8);
    if (!p) {
        return NULL;
    }
    *(int64_t *)p = len;
    /* Body bytes left uninitialized — caller fills them via
     * lotus_bytes_data(). Zeroing here would double the cost
     * for callers that overwrite the whole blob immediately
     * (the common case: read syscall reads into it, recv
     * fills it, etc.). */
    return p;
}

int64_t lotus_bytes_len(const void *b) {
    if (!b) return 0;
    return *(const int64_t *)b;
}

void *lotus_bytes_data(void *b) {
    if (!b) return NULL;
    return (char *)b + sizeof(int64_t);
}

/* B2 / G5 bytes-literal helper: allocate a Bytes blob in `a` and
 * copy `len` bytes from `src` into it. Used by codegen to lower
 * `b"..."` literals without a per-literal dance of create +
 * memcpy at the IR level. `src` may be NULL when `len == 0`. */
void *lotus_bytes_from_buf(lotus_arena_t *a, const void *src, int64_t len) {
    void *blob = lotus_bytes_create(a, len);
    if (!blob || len <= 0) {
        return blob;
    }
    memcpy(lotus_bytes_data(blob), src, (size_t)len);
    return blob;
}

int64_t lotus_str_index_of(const char *s, const char *sub) {
    if (!s || !sub) return -1;
    if (*sub == '\0') return 0;
    const char *hit = strstr(s, sub);
    if (!hit) return -1;
    return (int64_t)(hit - s);
}

/*
 * m48: render a Decimal value (i128 mantissa with implicit
 * scale 9 — i.e., mantissa × 10^-9) into a NUL-terminated
 * string. The i128 is passed as two i64 halves (hi:lo) since
 * the LLVM/C ABI for __int128 is awkward to wire; codegen
 * splits the value before the call.
 *
 * Output format trims trailing zeros + dangling decimal point,
 * matching the interpreter's DecimalVal::display so both
 * backends print identically. Caller passes a buffer of at
 * least LOTUS_DECIMAL_BUF_LEN bytes.
 */
#define LOTUS_DECIMAL_BUF_LEN 64

/* Helper used internally — exposed forward-decl form so the
 * arena-allocating sibling can call it. */
void lotus_decimal_to_string(int64_t hi, uint64_t lo, char *buf);

/*
 * Variant of lotus_decimal_to_string that allocates the buffer
 * inside the caller's arena and returns a pointer to it.
 * Mirrors lotus_str_from_float for the Float case.
 */
char *lotus_str_from_decimal(lotus_arena_t *a, int64_t hi, uint64_t lo) {
    char *out = (char *)lotus_arena_alloc(a, LOTUS_DECIMAL_BUF_LEN, 1);
    if (!out) return NULL;
    lotus_decimal_to_string(hi, lo, out);
    return out;
}

void lotus_decimal_to_string(int64_t hi, uint64_t lo, char *buf) {
    __int128 m = ((__int128)hi << 64) | (__int128)lo;
    int neg = m < 0;
    unsigned __int128 abs = neg ? (unsigned __int128)(-m) : (unsigned __int128)m;
    unsigned __int128 pow9 = 1000000000ULL;
    unsigned __int128 int_part = abs / pow9;
    unsigned __int128 frac_part = abs % pow9;
    char *p = buf;
    if (neg) {
        *p++ = '-';
    }
    /* int_part may exceed 64 bits when the mantissa's integer
     * part is over 10^19. The simple fast path covers the
     * common case; the fallback decomposes into 10^18 chunks. */
    if ((int_part >> 64) == 0) {
        p += snprintf(p, 32, "%llu", (unsigned long long)int_part);
    } else {
        unsigned __int128 base = 1000000000000000000ULL;
        unsigned __int128 hi_part = int_part / base;
        unsigned __int128 lo_part = int_part % base;
        p += snprintf(p, 48, "%llu%018llu",
            (unsigned long long)hi_part,
            (unsigned long long)lo_part);
    }
    if (frac_part != 0) {
        char fb[16];
        snprintf(fb, sizeof(fb), "%09llu", (unsigned long long)frac_part);
        size_t end = strlen(fb);
        while (end > 0 && fb[end - 1] == '0') {
            end--;
        }
        if (end > 0) {
            *p++ = '.';
            memcpy(p, fb, end);
            p += end;
        }
    }
    *p = '\0';
}

/*
 * m57: AF_UNIX transport for the cross-process bus.
 *
 * First substrate piece of the cross-process bus arc. Provides a
 * minimal "raw bytes between two processes over a unix socket"
 * surface: create a transport in listener or connector role, send
 * one message, recv one message, destroy. SOCK_SEQPACKET preserves
 * message boundaries so each lotus_transport_send shows up as
 * exactly one lotus_transport_recv — matches bus cell semantics
 * with no framing layer at this milestone.
 *
 * No protocol, no subject binding, no deployment-config: this is
 * the kernel-level transport substrate. m58 wires deployment-config
 * subject -> transport URL routing on top of these primitives;
 * m59 adds per-payload serializers; m60 weaves multi-binary builds
 * + fitter-applier-pair end-to-end. Source-level lotus stays
 * transport-agnostic per notes/open-questions #8.
 *
 * Lifecycle:
 *   - LISTEN role: bind + listen + accept. Blocks
 *     lotus_transport_create until exactly one connector connects.
 *   - CONNECT role: connect with retry-on-ENOENT/ECONNREFUSED for
 *     ~1s, then fail. Lets the connector start before the listener
 *     races to bind without needing an external sync point.
 *
 * Errors return NULL (create) or -1 (send/recv) and write a
 * perror-style message to stderr. v0.1 prefers fail-fast over
 * recovery — the protocol layer above this re-creates on failure.
 */

#define LOTUS_TRANSPORT_LISTEN  0
#define LOTUS_TRANSPORT_CONNECT 1

typedef struct lotus_transport {
    int   conn_fd;        /* duplex SEQPACKET fd carrying messages */
    int   listen_fd;      /* listener role only; -1 for connector */
    char *path;           /* listener role only; owned, unlinked on destroy */
    int   role;
} lotus_transport_t;

static int lotus__transport_set_addr(struct sockaddr_un *addr,
                                     const char *path) {
    size_t len = strlen(path);
    /* sun_path includes the NUL — reject anything that would not fit. */
    if (len + 1 > sizeof(addr->sun_path)) {
        errno = ENAMETOOLONG;
        return -1;
    }
    memset(addr, 0, sizeof(*addr));
    addr->sun_family = AF_UNIX;
    memcpy(addr->sun_path, path, len + 1);
    return 0;
}

lotus_transport_t *lotus_transport_create(const char *path, int role) {
    if (!path) {
        errno = EINVAL;
        return NULL;
    }
    struct sockaddr_un addr;
    if (lotus__transport_set_addr(&addr, path) != 0) {
        perror("lotus_transport_create: addr");
        return NULL;
    }

    int sock = socket(AF_UNIX, SOCK_SEQPACKET, 0);
    if (sock < 0) {
        perror("lotus_transport_create: socket");
        return NULL;
    }

    if (role == LOTUS_TRANSPORT_LISTEN) {
        /* Best-effort: clear any stale socket file so bind succeeds
         * after a previous run was killed without destroy(). */
        unlink(path);
        if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
            perror("lotus_transport_create: bind");
            close(sock);
            return NULL;
        }
        if (listen(sock, 1) < 0) {
            perror("lotus_transport_create: listen");
            close(sock);
            unlink(path);
            return NULL;
        }
        int conn = accept(sock, NULL, NULL);
        if (conn < 0) {
            perror("lotus_transport_create: accept");
            close(sock);
            unlink(path);
            return NULL;
        }
        lotus_transport_t *t = (lotus_transport_t *)calloc(1, sizeof(*t));
        if (!t) {
            close(conn);
            close(sock);
            unlink(path);
            return NULL;
        }
        t->conn_fd   = conn;
        t->listen_fd = sock;
        t->path      = strdup(path);
        t->role      = role;
        return t;
    }

    if (role == LOTUS_TRANSPORT_CONNECT) {
        /* Retry connect on ENOENT/ECONNREFUSED for up to ~1s so a
         * connector that races ahead of the listener's bind/listen
         * still succeeds once the listener becomes ready. */
        struct timespec backoff = { 0, 5L * 1000L * 1000L };  /* 5ms */
        int attempts = 200;                                   /* 200 × 5ms */
        while (attempts-- > 0) {
            if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) == 0) {
                lotus_transport_t *t =
                    (lotus_transport_t *)calloc(1, sizeof(*t));
                if (!t) {
                    close(sock);
                    return NULL;
                }
                t->conn_fd   = sock;
                t->listen_fd = -1;
                t->path      = NULL;
                t->role      = role;
                return t;
            }
            if (errno != ENOENT && errno != ECONNREFUSED) {
                perror("lotus_transport_create: connect");
                close(sock);
                return NULL;
            }
            nanosleep(&backoff, NULL);
        }
        fprintf(stderr,
                "lotus_transport_create: connect to %s timed out\n",
                path);
        close(sock);
        return NULL;
    }

    fprintf(stderr, "lotus_transport_create: invalid role %d\n", role);
    close(sock);
    errno = EINVAL;
    return NULL;
}

int lotus_transport_send(lotus_transport_t *t,
                         const void *buf,
                         size_t len) {
    if (!t || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    ssize_t n = send(t->conn_fd, buf, len, 0);
    if (n < 0) {
        perror("lotus_transport_send");
        return -1;
    }
    return 0;
}

ssize_t lotus_transport_recv(lotus_transport_t *t,
                             void *buf,
                             size_t cap) {
    if (!t || (!buf && cap > 0)) {
        errno = EINVAL;
        return -1;
    }
    ssize_t n = recv(t->conn_fd, buf, cap, 0);
    if (n < 0) {
        perror("lotus_transport_recv");
        return -1;
    }
    return n;
}

void lotus_transport_destroy(lotus_transport_t *t) {
    if (!t) return;
    if (t->conn_fd >= 0) close(t->conn_fd);
    if (t->listen_fd >= 0) close(t->listen_fd);
    if (t->path) {
        unlink(t->path);
        free(t->path);
    }
    free(t);
}

/*
 * m72: TCP transport (AF_INET) — sibling adapter to the AF_UNIX
 * SEQPACKET transport above.
 *
 * Design context (project_tcp_framing.md): the transport surface
 * contracts to deliver atomic messages — one send produces one
 * recv of the same byte sequence at the other end. SEQPACKET
 * satisfies this via kernel datagram semantics; TCP satisfies it
 * by length-prefix framing inside this adapter. The bus layer
 * above is transport-agnostic and treats every transport as
 * "give me the next whole message." Future transports (TLS, QUIC,
 * shared-memory rings) will each pick whatever internal mechanism
 * satisfies the same atomic-message contract.
 *
 * Wire format per message:
 *   [8-byte little-endian uint64 length] [N bytes payload]
 * The 8-byte LE length matches the m70 per-field serializer's
 * String framing convention.
 *
 * Sanity cap: LOTUS_TCP_MAX_MSG_BYTES rejects pathologically
 * large length headers before any allocation or recv loop runs,
 * preventing a malicious or buggy peer from claiming 2^63 bytes
 * and stalling the receiver.
 *
 * Lifecycle mirrors lotus_transport:
 *   - LISTEN role: socket + SO_REUSEADDR + bind + listen + accept.
 *     Blocks lotus_tcp_create until exactly one connector connects.
 *   - CONNECT role: connect with retry on ECONNREFUSED for ~1s.
 *
 * SO_REUSEADDR is set on the listener so a freshly-released port
 * (very recent test runs, dev iteration) doesn't trip TIME_WAIT.
 * TCP_NODELAY is set on the connection so single small messages
 * aren't held by Nagle's algorithm — the bus's typical workload
 * is request/response-shaped where latency matters more than
 * coalescing.
 *
 * Errors return NULL (create) or -1 (send/recv); recv also
 * returns -1 if the framed length exceeds `cap` (caller's buffer
 * too small) or LOTUS_TCP_MAX_MSG_BYTES (cap regardless).
 */

#define LOTUS_TCP_LISTEN  0
#define LOTUS_TCP_CONNECT 1

/* 8 MB ceiling. Generous for typed bus payloads while still
 * making a malicious 2^63 length header an immediate -1. */
#define LOTUS_TCP_MAX_MSG_BYTES (8u * 1024u * 1024u)

typedef struct lotus_tcp {
    int   conn_fd;     /* the connected stream socket */
    int   listen_fd;   /* listener role only; -1 for connector */
    int   role;
    uint16_t port;     /* the actual bound/connected port (esp. when listen requested 0) */
} lotus_tcp_t;

/* Read exactly `n` bytes into `buf` from `fd`, looping over short
 * reads. Returns 0 on success, -1 on error or EOF before n bytes.
 * Used by recv to reassemble the framed message — TCP is a byte
 * stream, so a single read may return any prefix of the requested
 * count. */
static int lotus__tcp_read_full(int fd, void *buf, size_t n) {
    char  *p = (char *)buf;
    size_t left = n;
    while (left > 0) {
        ssize_t r = read(fd, p, left);
        if (r > 0) {
            p    += (size_t)r;
            left -= (size_t)r;
            continue;
        }
        if (r == 0) {
            /* peer closed mid-message — surface as EIO so the
             * caller sees a non-zero errno. */
            errno = EIO;
            return -1;
        }
        if (errno == EINTR) continue;
        return -1;
    }
    return 0;
}

/* Write exactly `n` bytes from `buf` to `fd`, looping over short
 * writes. Mirrors lotus__tcp_read_full. */
static int lotus__tcp_write_full(int fd, const void *buf, size_t n) {
    const char *p = (const char *)buf;
    size_t      left = n;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        /* w == 0 on a regular fd is unusual; treat as error. */
        return -1;
    }
    return 0;
}

/* Encode a host-order uint64 as 8 little-endian bytes. */
static void lotus__u64_to_le(uint64_t v, unsigned char out[8]) {
    for (int i = 0; i < 8; i++) {
        out[i] = (unsigned char)(v >> (i * 8));
    }
}

/* Decode 8 little-endian bytes to a host-order uint64. */
static uint64_t lotus__u64_from_le(const unsigned char in[8]) {
    uint64_t v = 0;
    for (int i = 0; i < 8; i++) {
        v |= ((uint64_t)in[i]) << (i * 8);
    }
    return v;
}

lotus_tcp_t *lotus_tcp_create(const char *host, uint16_t port, int role) {
    /* host=NULL is allowed for both roles: listener interprets as
     * INADDR_ANY (bind-on-any-interface); connector defaults to
     * 127.0.0.1 since "no peer specified" means same-host. */
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    if (role == LOTUS_TCP_LISTEN) {
        if (!host) {
            addr.sin_addr.s_addr = htonl(INADDR_ANY);
        } else if (inet_pton(AF_INET, host, &addr.sin_addr) != 1) {
            fprintf(stderr,
                    "lotus_tcp_create: invalid listen host %s\n", host);
            errno = EINVAL;
            return NULL;
        }
    } else if (role == LOTUS_TCP_CONNECT) {
        const char *h = host ? host : "127.0.0.1";
        if (inet_pton(AF_INET, h, &addr.sin_addr) != 1) {
            fprintf(stderr,
                    "lotus_tcp_create: invalid connect host %s\n", h);
            errno = EINVAL;
            return NULL;
        }
    } else {
        fprintf(stderr, "lotus_tcp_create: invalid role %d\n", role);
        errno = EINVAL;
        return NULL;
    }

    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        perror("lotus_tcp_create: socket");
        return NULL;
    }

    if (role == LOTUS_TCP_LISTEN) {
        int one = 1;
        if (setsockopt(sock, SOL_SOCKET, SO_REUSEADDR,
                       &one, sizeof(one)) < 0) {
            perror("lotus_tcp_create: SO_REUSEADDR");
            close(sock);
            return NULL;
        }
        if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
            perror("lotus_tcp_create: bind");
            close(sock);
            return NULL;
        }
        /* If port=0 the OS picked one; getsockname tells us which. */
        socklen_t alen = sizeof(addr);
        if (getsockname(sock, (struct sockaddr *)&addr, &alen) < 0) {
            perror("lotus_tcp_create: getsockname");
            close(sock);
            return NULL;
        }
        if (listen(sock, 1) < 0) {
            perror("lotus_tcp_create: listen");
            close(sock);
            return NULL;
        }
        int conn = accept(sock, NULL, NULL);
        if (conn < 0) {
            perror("lotus_tcp_create: accept");
            close(sock);
            return NULL;
        }
        int nodelay = 1;
        (void)setsockopt(conn, IPPROTO_TCP, TCP_NODELAY,
                         &nodelay, sizeof(nodelay));
        lotus_tcp_t *t = (lotus_tcp_t *)calloc(1, sizeof(*t));
        if (!t) {
            close(conn);
            close(sock);
            return NULL;
        }
        t->conn_fd   = conn;
        t->listen_fd = sock;
        t->role      = role;
        t->port      = ntohs(addr.sin_port);
        return t;
    }

    /* CONNECT: retry on ECONNREFUSED for ~1s so a connector that
     * races ahead of the listener's bind/listen still succeeds
     * once the listener becomes ready. Mirrors the unix-socket
     * adapter. */
    struct timespec backoff = { 0, 5L * 1000L * 1000L };  /* 5ms */
    int attempts = 200;                                   /* 200 × 5ms */
    while (attempts-- > 0) {
        if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) == 0) {
            int nodelay = 1;
            (void)setsockopt(sock, IPPROTO_TCP, TCP_NODELAY,
                             &nodelay, sizeof(nodelay));
            lotus_tcp_t *t = (lotus_tcp_t *)calloc(1, sizeof(*t));
            if (!t) {
                close(sock);
                return NULL;
            }
            t->conn_fd   = sock;
            t->listen_fd = -1;
            t->role      = role;
            t->port      = port;
            return t;
        }
        if (errno != ECONNREFUSED && errno != EAGAIN) {
            perror("lotus_tcp_create: connect");
            close(sock);
            return NULL;
        }
        nanosleep(&backoff, NULL);
    }
    fprintf(stderr,
            "lotus_tcp_create: connect to port %u timed out\n",
            (unsigned)port);
    close(sock);
    return NULL;
}

uint16_t lotus_tcp_port(lotus_tcp_t *t) {
    return t ? t->port : 0;
}

int lotus_tcp_send(lotus_tcp_t *t, const void *buf, size_t len) {
    if (!t || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    if ((uint64_t)len > LOTUS_TCP_MAX_MSG_BYTES) {
        errno = EMSGSIZE;
        return -1;
    }
    unsigned char header[8];
    lotus__u64_to_le((uint64_t)len, header);
    if (lotus__tcp_write_full(t->conn_fd, header, sizeof(header)) < 0) {
        perror("lotus_tcp_send: header");
        return -1;
    }
    if (len > 0 && lotus__tcp_write_full(t->conn_fd, buf, len) < 0) {
        perror("lotus_tcp_send: payload");
        return -1;
    }
    return 0;
}

ssize_t lotus_tcp_recv(lotus_tcp_t *t, void *buf, size_t cap) {
    if (!t || (!buf && cap > 0)) {
        errno = EINVAL;
        return -1;
    }
    unsigned char header[8];
    if (lotus__tcp_read_full(t->conn_fd, header, sizeof(header)) < 0) {
        /* don't perror on the common EOF case — the caller knows
         * a -1 here means "stream ended or read error"; spammy
         * stderr would obscure the actual program output. */
        return -1;
    }
    uint64_t len = lotus__u64_from_le(header);
    if (len > LOTUS_TCP_MAX_MSG_BYTES) {
        fprintf(stderr,
                "lotus_tcp_recv: framed length %llu exceeds %u\n",
                (unsigned long long)len, LOTUS_TCP_MAX_MSG_BYTES);
        errno = EMSGSIZE;
        return -1;
    }
    if (len > (uint64_t)cap) {
        fprintf(stderr,
                "lotus_tcp_recv: framed length %llu exceeds caller cap %zu\n",
                (unsigned long long)len, cap);
        errno = EMSGSIZE;
        return -1;
    }
    if (len == 0) return 0;
    if (lotus__tcp_read_full(t->conn_fd, buf, (size_t)len) < 0) {
        perror("lotus_tcp_recv: payload");
        return -1;
    }
    return (ssize_t)len;
}

void lotus_tcp_destroy(lotus_tcp_t *t) {
    if (!t) return;
    if (t->conn_fd >= 0) close(t->conn_fd);
    if (t->listen_fd >= 0) close(t->listen_fd);
    free(t);
}

/*
 * m73b: split-shape primitives reachable from Aperio source.
 *
 * lotus_tcp_create collapses bind+listen+accept into one
 * blocking call — convenient for the m72 driver tests but wrong
 * for a Listener locus pattern where birth() should not block on
 * an incoming connection. The locus's lifecycle wants:
 *
 *   birth():     bind+listen     -> listen_fd          (non-blocking)
 *   run():       accept (loop)   -> conn_fd per peer   (blocks per accept)
 *   dissolve():  close(listen_fd)
 *
 * These three functions provide that split. Aperio source
 * reaches them via the magic `std::io::tcp::__*` path-call
 * primitives wired up in codegen (m73b path-call additions). The
 * `__` prefix is internal-only; the polished user surface is
 * the Listener / Stream loci that wrap these calls in idiomatic
 * lifecycle bodies.
 *
 * fds are returned as plain ints; -1 signals error (errno set).
 * Callers stash the listen_fd on `self` in birth() and read it
 * back in run/dissolve via the standard locus self-field
 * mechanics — no opaque handle struct needed because the
 * Listener locus IS the handle.
 */

int lotus_tcp_listen_socket(const char *host, uint16_t port) {
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    if (!host) {
        addr.sin_addr.s_addr = htonl(INADDR_ANY);
    } else if (inet_pton(AF_INET, host, &addr.sin_addr) != 1) {
        fprintf(stderr,
                "lotus_tcp_listen_socket: invalid host %s\n", host);
        errno = EINVAL;
        return -1;
    }
    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        perror("lotus_tcp_listen_socket: socket");
        return -1;
    }
    int one = 1;
    if (setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one)) < 0) {
        perror("lotus_tcp_listen_socket: SO_REUSEADDR");
        close(sock);
        return -1;
    }
    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        perror("lotus_tcp_listen_socket: bind");
        close(sock);
        return -1;
    }
    if (listen(sock, 16) < 0) {
        perror("lotus_tcp_listen_socket: listen");
        close(sock);
        return -1;
    }
    return sock;
}

int lotus_tcp_accept_one(int listen_fd) {
    int conn = accept(listen_fd, NULL, NULL);
    if (conn < 0) {
        if (errno != EINTR) {
            perror("lotus_tcp_accept_one: accept");
        }
        return -1;
    }
    int nodelay = 1;
    (void)setsockopt(conn, IPPROTO_TCP, TCP_NODELAY,
                     &nodelay, sizeof(nodelay));
    return conn;
}

int lotus_tcp_connect(const char *host, uint16_t port) {
    /* Mirrors lotus_tcp_create's CONNECT-role logic but returns a
     * raw fd so it can be wrapped by `std::io::tcp::Stream {
     * conn_fd }` from Aperio source. Same retry-on-ECONNREFUSED
     * shape (~1s window).
     *
     * C6 (pond follow-up): fast path is still numeric-address
     * via inet_pton; when that returns 0 (host isn't a dotted
     * quad), fall back to getaddrinfo(host, port_str, AF_INET +
     * SOCK_STREAM) and use the first returned address. The numeric
     * path is bit-for-bit identical to the pre-C6 behavior — only
     * non-numeric hosts take the DNS branch. */
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    const char *h = host ? host : "127.0.0.1";
    int pton = inet_pton(AF_INET, h, &addr.sin_addr);
    if (pton != 1) {
        /* C6: getaddrinfo fallback for hostname resolution. We map
         * gai errors onto the existing IoError errno taxonomy so
         * callers don't need a new error kind: EAI_NONAME (unknown
         * host) -> ENOENT ("not_found"); everything else (DNS
         * server failure, transient, no-address-for-family, etc.)
         * -> EHOSTUNREACH ("host_unreachable"). gai_strerror is
         * printed to stderr for diagnostic visibility but doesn't
         * cross the IoError boundary. */
        struct addrinfo hints;
        struct addrinfo *res = NULL;
        memset(&hints, 0, sizeof(hints));
        hints.ai_family   = AF_INET;
        hints.ai_socktype = SOCK_STREAM;
        char port_str[16];
        snprintf(port_str, sizeof(port_str), "%u", (unsigned)port);
        int gai = getaddrinfo(h, port_str, &hints, &res);
        if (gai != 0 || res == NULL) {
            fprintf(stderr,
                    "lotus_tcp_connect: resolve %s: %s\n",
                    h, gai_strerror(gai));
            if (res) freeaddrinfo(res);
            errno = (gai == EAI_NONAME) ? ENOENT : EHOSTUNREACH;
            return -1;
        }
        /* First result wins — round-robin / multi-A handling is
         * the caller's job (they can pre-resolve if they want it). */
        struct sockaddr_in *resolved = (struct sockaddr_in *)res->ai_addr;
        addr.sin_addr = resolved->sin_addr;
        freeaddrinfo(res);
    }
    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        perror("lotus_tcp_connect: socket");
        return -1;
    }
    struct timespec backoff = { 0, 5L * 1000L * 1000L };
    int attempts = 200;
    while (attempts-- > 0) {
        if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) == 0) {
            int nodelay = 1;
            (void)setsockopt(sock, IPPROTO_TCP, TCP_NODELAY,
                             &nodelay, sizeof(nodelay));
            return sock;
        }
        if (errno != ECONNREFUSED && errno != EAGAIN) {
            perror("lotus_tcp_connect: connect");
            close(sock);
            return -1;
        }
        nanosleep(&backoff, NULL);
    }
    fprintf(stderr,
            "lotus_tcp_connect: connect to %s:%u timed out\n",
            h, (unsigned)port);
    close(sock);
    return -1;
}

int lotus_tcp_close_fd(int fd) {
    if (fd < 0) return 0;
    return close(fd);
}

/* Forward decl — defined later (next to lotus_bus_payload_arena
 * proper). Lets the UDP block below build Bytes blobs in the
 * payload arena. */
void *lotus_bus_payload_arena_alloc(size_t size, size_t align);

/*
 * Raw UDP primitives. Datagram socket (SOCK_DGRAM) — preserves
 * per-message boundaries from the kernel, no framing needed at
 * this layer, no delivery guarantee (per UDP semantics). The
 * bus's "deliver one whole message" contract is therefore NOT
 * satisfied by UDP at the substrate level; cross-host bus over
 * UDP is the application's problem (retry, reorder, etc.).
 *
 * Surface (intentionally minimal for v1):
 *   - lotus_udp_bind(host, port) — create a socket bound to
 *     (host, port). host=NULL or "0.0.0.0" → INADDR_ANY.
 *   - lotus_udp_sendto(fd, host, port, buf, len) — send one
 *     datagram (best-effort) to the named peer.
 *   - lotus_udp_recv(fd, buf, cap) — receive one datagram.
 *     Returns bytes received, or -1 on error.
 *   - lotus_udp_close(fd) — close the socket.
 *
 * For held-open send-only sockets where you want a fixed peer,
 * pair lotus_udp_bind(NULL, 0) with repeated lotus_udp_sendto.
 * For receive, bind(host, port) then loop on lotus_udp_recv.
 *
 * Errors return -1 with errno set. lotus_udp_recv truncates
 * messages larger than `cap` (per recvfrom man page).
 */

int lotus_udp_bind(const char *host, uint16_t port) {
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    if (!host || *host == '\0' || strcmp(host, "0.0.0.0") == 0) {
        addr.sin_addr.s_addr = htonl(INADDR_ANY);
    } else if (inet_pton(AF_INET, host, &addr.sin_addr) != 1) {
        fprintf(stderr, "lotus_udp_bind: invalid host %s\n", host);
        errno = EINVAL;
        return -1;
    }
    int sock = socket(AF_INET, SOCK_DGRAM, 0);
    if (sock < 0) {
        perror("lotus_udp_bind: socket");
        return -1;
    }
    int one = 1;
    if (setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one)) < 0) {
        perror("lotus_udp_bind: SO_REUSEADDR");
        close(sock);
        return -1;
    }
    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        perror("lotus_udp_bind: bind");
        close(sock);
        return -1;
    }
    return sock;
}

int lotus_udp_sendto(int fd, const char *host, uint16_t port,
                     const void *buf, size_t len) {
    if (fd < 0 || !host) {
        errno = EINVAL;
        return -1;
    }
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    if (inet_pton(AF_INET, host, &addr.sin_addr) != 1) {
        fprintf(stderr, "lotus_udp_sendto: invalid host %s\n", host);
        errno = EINVAL;
        return -1;
    }
    ssize_t n = sendto(fd, buf, len, 0,
                       (struct sockaddr *)&addr, sizeof(addr));
    if (n < 0) {
        return -1;
    }
    return 0;
}

ssize_t lotus_udp_recv(int fd, void *buf, size_t cap) {
    if (fd < 0 || (!buf && cap > 0)) {
        errno = EINVAL;
        return -1;
    }
    ssize_t n = recvfrom(fd, buf, cap, 0, NULL, NULL);
    return n;
}

int lotus_udp_close(int fd) {
    if (fd < 0) return 0;
    return close(fd);
}

/* Aperio-side wrappers that adapt the raw primitives to the
 * String / Bytes ABI. The String variant of sendto takes a
 * NUL-terminated string and computes its length internally;
 * the Bytes variants take the standard [i64 len][u8 body[len]]
 * blob and walk the prefix. recv_bytes allocates the result
 * in the bus payload arena like its tcp sibling. */

int lotus_udp_sendto_str(int fd, const char *host, uint16_t port,
                         const char *msg) {
    if (!msg) {
        errno = EINVAL;
        return -1;
    }
    return lotus_udp_sendto(fd, host, port, msg, strlen(msg));
}

void *lotus_udp_recv_bytes_global(int fd, int max_bytes) {
    if (fd < 0 || max_bytes <= 0) {
        errno = EINVAL;
        return NULL;
    }
    /* Use a stack buffer for the kernel handoff; the result is
     * copied into a Bytes blob in the payload arena. UDP max
     * packet size is ~65507 bytes on IPv4, so a 64KB stack
     * buffer covers anything. */
    char stack_buf[65536];
    size_t cap = (size_t)max_bytes;
    if (cap > sizeof(stack_buf)) cap = sizeof(stack_buf);
    ssize_t n = recvfrom(fd, stack_buf, cap, 0, NULL, NULL);
    if (n < 0) return NULL;
    /* Hand-build a Bytes blob ([i64 len][body]) in the global
     * payload arena via the public alloc helper. Mirrors the
     * lotus_bytes_create shape (without needing a direct
     * lotus_arena_t handle to the static g_bus_payload_arena). */
    size_t blob_size = sizeof(int64_t) + (size_t)n;
    void *blob = lotus_bus_payload_arena_alloc(blob_size, 8);
    if (!blob) return NULL;
    *(int64_t *)blob = (int64_t)n;
    if (n > 0) {
        memcpy((char *)blob + sizeof(int64_t), stack_buf, (size_t)n);
    }
    return blob;
}

/*
 * m81: send / recv on a connected TCP fd, exposed to Aperio
 * as String-shaped operations. send_str writes the bytes of
 * the NUL-terminated input (length via strlen — embedded NULs
 * truncate, mirroring m75's std::io::fs::write_file behavior;
 * binary I/O waits on Bytes codegen). recv_str reads up to
 * max_bytes into a freshly-allocated buffer in the lazy global
 * payload arena, NUL-terminates at the actual byte count, and
 * returns a stable pointer the caller can hold for the program
 * lifetime (same ownership model as m75's read_file).
 */

/* Forward decl — defined later in this file. */
void *lotus_bus_payload_arena_alloc(size_t size, size_t align);

int lotus_tcp_send_str(int fd, const char *msg) {
    if (fd < 0) {
        errno = EBADF;
        return -1;
    }
    if (!msg) {
        errno = EINVAL;
        return -1;
    }
    size_t len = strlen(msg);
    const char *p = msg;
    size_t      left = len;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        perror("lotus_tcp_send_str: write");
        return -1;
    }
    return 0;
}

/*
 * m89: write a Bytes blob to a TCP fd. Uses the explicit
 * length stored in the blob's prefix (not strlen) so embedded
 * NUL bytes don't truncate. write(2) loop handles partial
 * writes; returns 0 on full send, -1 on error.
 */
int lotus_tcp_send_bytes(int fd, const void *bytes_ptr) {
    if (fd < 0) {
        errno = EBADF;
        return -1;
    }
    if (!bytes_ptr) {
        errno = EINVAL;
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
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        perror("lotus_tcp_send_bytes: write");
        return -1;
    }
    return 0;
}

const char *lotus_tcp_recv_str(int fd, int max_bytes) {
    /* Stable empty-string sentinel — same trick as g_empty_str
     * but local to this function-family because m81 may run
     * before lotus_env_init has cleared the env globals. */
    static const char empty[1] = { 0 };
    if (fd < 0 || max_bytes <= 0) {
        return empty;
    }
    size_t cap = (size_t)max_bytes;
    char *buf = (char *)lotus_bus_payload_arena_alloc(cap + 1, 1);
    if (!buf) {
        return empty;
    }
    ssize_t n = read(fd, buf, cap);
    if (n < 0) {
        if (errno != EINTR) {
            /* Treat read errors as "got nothing" at this level —
             * the buffer is in the lazy arena so it persists; the
             * stable empty-string sentinel signals "no data" to
             * the caller. */
        }
        return empty;
    }
    /* NUL-terminate at the actual bytes-read offset; a zero-byte
     * read (peer closed cleanly) yields an empty string at the
     * arena buffer. */
    buf[(size_t)n] = '\0';
    return buf;
}

/* Phase 2g: forward decls for the lotus_*_bytes helpers below.
 * Their bodies live next to the other global-payload-arena
 * wrappers (after lotus_bus_payload_arena_alloc at ~line 2814)
 * because that's where g_bus_payload_arena is first declared. */
void *lotus_tcp_recv_bytes(int fd, int max_bytes);
const char *lotus_str_from_bytes(const void *b);
void *lotus_bytes_from_str(const char *s);
int64_t lotus_bytes_at(const void *b, int64_t i);
void *lotus_bytes_slice(const void *b, int64_t lo, int64_t hi);

/* Wave B: same-shape forward decl for the bus-payload-arena
 * accessor used by lotus_bus_remote_fanout's adapter dispatch
 * path. Body lives with the rest of the payload-arena machinery
 * once g_bus_payload_arena itself is in scope. */
lotus_arena_t *lotus_bus_payload_arena_get(void);

/* Phase 2e + 2f + C9: forward decls for fs primitives whose
 * bodies need g_bus_payload_arena (declared further below) so
 * the returned String outlives the call frame. */
int64_t lotus_fs_list_dir_count(const char *path);
const char *lotus_fs_list_dir_at(const char *path, int64_t idx);
const char *lotus_fs_mktemp(const char *prefix, const char *suffix);

/*
 * m74: filesystem primitives (`std::io::fs::*` substrate).
 *
 * One-shot synchronous file operations. POSIX wrappers, no
 * caching, no buffering — the same shape POSIX presents,
 * surfaced through a small C ABI that codegen calls from the
 * `std::io::fs::__*` magic-path stdlib primitives.
 *
 * Shape choice: each function takes raw pointers + sizes and
 * returns either a count (read/size) or 0/-1 status (write/
 * exists). No opaque file-handle struct because Phase-1 file
 * operations are one-shot — there's no lifetime-of-a-stream
 * concept to manage. A future milestone that needs streaming
 * reads/writes adds a separate `lotus_fs_open` / `_read` /
 * `_close` family alongside this one.
 *
 * read_dir is deliberately deferred: the variable-length
 * output story (NUL-separated buffer? iteration model? per-
 * entry callback?) deserves its own design pass and is not
 * needed for the m76 capstone (which reads + writes a config
 * file and a log file, not a directory listing).
 */

/* Read up to `out_cap` bytes from `path` into `out_buf`.
 * Returns bytes read (>=0) on success, -1 on error (errno set).
 * If the file is larger than `out_cap` the surplus is silently
 * dropped — the caller decides whether that's acceptable by
 * comparing the return against the cap. Files larger than what
 * fits in size_t are not supported (extremely rare on the v0
 * target). */
ssize_t lotus_fs_read_file(const char *path,
                           void *out_buf,
                           size_t out_cap) {
    if (!path || (!out_buf && out_cap > 0)) {
        errno = EINVAL;
        return -1;
    }
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        /* keep the diagnostic terse — perror would be noisy
         * for the common "file not found" case; callers that
         * want to distinguish errors check errno. */
        return -1;
    }
    char *p = (char *)out_buf;
    size_t left = out_cap;
    ssize_t total = 0;
    while (left > 0) {
        ssize_t r = read(fd, p, left);
        if (r > 0) {
            p     += (size_t)r;
            left  -= (size_t)r;
            total += r;
            continue;
        }
        if (r == 0) break;             /* EOF */
        if (errno == EINTR) continue;  /* interrupted; retry */
        close(fd);
        return -1;
    }
    close(fd);
    return total;
}

/*
 * m89: read whole file as a Bytes blob. Allocates a fresh
 * Bytes value on the caller's arena sized to the file's
 * length, fills it from the fd, returns the pointer. NULL
 * on any error (file missing, permission denied, etc.) —
 * caller distinguishes via errno. Used by std::io::fs::
 * read_bytes for binary file I/O where String's NUL
 * truncation would silently corrupt the data.
 */
void *lotus_fs_read_bytes(lotus_arena_t *a, const char *path) {
    if (!a || !path) {
        errno = EINVAL;
        return NULL;
    }
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        return NULL;
    }
    /* Stat to size the blob exactly. fstat keeps us off a
     * second open; on regular files st_size is the byte
     * count we need. */
    struct stat st;
    if (fstat(fd, &st) < 0) {
        close(fd);
        return NULL;
    }
    int64_t size = (int64_t)st.st_size;
    void *blob = lotus_bytes_create(a, size);
    if (!blob) {
        close(fd);
        errno = ENOMEM;
        return NULL;
    }
    char *body = (char *)lotus_bytes_data(blob);
    size_t left = (size_t)size;
    while (left > 0) {
        ssize_t r = read(fd, body, left);
        if (r > 0) {
            body += (size_t)r;
            left -= (size_t)r;
            continue;
        }
        if (r == 0) break;
        if (errno == EINTR) continue;
        close(fd);
        return NULL;
    }
    close(fd);
    /* If the file shrank between fstat and read (race), the
     * trailing bytes are uninitialized blob memory. v0
     * accepts that — the next milestone might re-read st_size
     * after the loop and patch the prefix down. */
    return blob;
}

/*
 * m90: enumerate a directory's entries, returning a single
 * String with one entry per line (`\n`-separated, trailing
 * newline included). Skips `.` and `..` so callers don't
 * have to filter them. Errors (path missing, not a
 * directory, permission denied) return an empty string —
 * same soft-fail shape as the rest of std::io::fs.
 *
 * v0 design choice: newline-separated String, not Bytes /
 * not a List<String>, so the substrate composes with the
 * existing String primitives (index_of, slice). When Aperio
 * grows a generic List<T> this can grow a sibling
 * `list_dir_entries(path) -> [String]` API; for Phase 5's
 * doc-server need (enumerate `.md` files in docs/), the
 * String shape is sufficient — user code walks newlines via
 * std::str::index_of("\n").
 *
 * Filenames with embedded `\n` would corrupt this format.
 * POSIX permits them (only `\0` and `/` are illegal in path
 * segments) but they're rare; v0 documents the limitation
 * and chooses the simpler shape.
 */
const char *lotus_fs_list_dir(lotus_arena_t *a, const char *path) {
    static const char empty[1] = { 0 };
    if (!a || !path) {
        return empty;
    }
    DIR *dir = opendir(path);
    if (!dir) {
        return empty;
    }
    /* First pass: tally the byte count we need. struct
     * dirent's d_name is NUL-terminated; we add 1 byte per
     * entry for the joining `\n` (plus the trailing one). */
    size_t total = 0;
    struct dirent *e;
    while ((e = readdir(dir)) != NULL) {
        if (strcmp(e->d_name, ".") == 0
            || strcmp(e->d_name, "..") == 0) {
            continue;
        }
        total += strlen(e->d_name) + 1;
    }
    rewinddir(dir);

    /* Allocate (total + 1) for the trailing NUL terminator. */
    char *buf = (char *)lotus_arena_alloc(a, total + 1, 1);
    if (!buf) {
        closedir(dir);
        return empty;
    }
    /* Second pass: copy entry names + newlines. Because
     * filesystems can change between rewinddir and the
     * second readdir, the actual bytes copied may differ
     * from the first-pass tally; we cap by `total` to
     * avoid overrun and accept that a directory mutated
     * mid-call may lose late-arriving entries. v0 considers
     * directory-listing under concurrent mutation an out-of-
     * scope concern. */
    char *p = buf;
    size_t left = total;
    while ((e = readdir(dir)) != NULL && left > 0) {
        if (strcmp(e->d_name, ".") == 0
            || strcmp(e->d_name, "..") == 0) {
            continue;
        }
        size_t nlen = strlen(e->d_name);
        if (nlen + 1 > left) break;
        memcpy(p, e->d_name, nlen);
        p[nlen] = '\n';
        p += nlen + 1;
        left -= nlen + 1;
    }
    *p = '\0';
    closedir(dir);
    return buf;
}

/* Write exactly `len` bytes from `buf` to `path`. Truncates
 * any existing file. Returns 0 on success, -1 on error. */
int lotus_fs_write_file(const char *path,
                        const void *buf,
                        size_t len) {
    if (!path || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        return -1;
    }
    const char *p = (const char *)buf;
    size_t left = len;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        close(fd);
        return -1;
    }
    /* close return matters for write_file: a deferred filesystem
     * error (e.g. NFS write-back) surfaces here, not in write(). */
    if (close(fd) != 0) {
        return -1;
    }
    return 0;
}

/* Append `len` bytes of `buf` to `path`. Creates the file with
 * mode 0644 if it doesn't exist; otherwise opens existing for
 * append. Returns 0 on success, -1 on error (errno set).
 * Companion to lotus_fs_write_file (which truncates); ergonomics
 * milestone resolves the apps/log-router friction "no append
 * primitive forces buffer-everything-then-flush at dissolve". */
int lotus_fs_write_file_append(const char *path,
                               const void *buf,
                               size_t len) {
    if (!path || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    int fd = open(path, O_WRONLY | O_CREAT | O_APPEND, 0644);
    if (fd < 0) {
        return -1;
    }
    const char *p = (const char *)buf;
    size_t left = len;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        close(fd);
        return -1;
    }
    if (close(fd) != 0) {
        return -1;
    }
    return 0;
}

/* Create the directory at `path` with mode 0755. Returns 0 on
 * success, -1 on error (errno set; EEXIST when the directory
 * already exists). NOT recursive — callers that want
 * `mkdir -p`-style semantics should test parent existence
 * themselves. Resolves apps/ssg friction "no mkdir / create_dir
 * forces shell-out via README precondition". */
int lotus_fs_mkdir(const char *path) {
    if (!path) {
        errno = EINVAL;
        return -1;
    }
    if (mkdir(path, 0755) < 0) {
        return -1;
    }
    return 0;
}

/* C9 (pond/logfmt rotation): atomic rename `src` → `dst`. POSIX
 * rename(2); atomic on the same filesystem, EXDEV cross-fs.
 * Returns 0 on success, -1 on error (errno set). The codegen
 * wrapper anchors the IoError.path to `dst` because the
 * destination is the more diagnostic of the two on the common
 * failure modes (target dir missing, target already a non-empty
 * dir, cross-fs, etc.). */
int lotus_fs_rename(const char *src, const char *dst) {
    if (!src || !dst) {
        errno = EINVAL;
        return -1;
    }
    if (rename(src, dst) < 0) {
        return -1;
    }
    return 0;
}

/* C9 (pond/logfmt rotation): unlink `path`. POSIX unlink(2) —
 * removes a regular file or symlink. Directories require rmdir
 * (not yet exposed). Returns 0 on success, -1 on error (errno
 * set; ENOENT when the path didn't exist, EISDIR on a directory
 * target). */
int lotus_fs_unlink(const char *path) {
    if (!path) {
        errno = EINVAL;
        return -1;
    }
    if (unlink(path) < 0) {
        return -1;
    }
    return 0;
}

/* Returns the size of `path` in bytes, or -1 on error. Follows
 * symlinks (stat, not lstat). */
int64_t lotus_fs_file_size(const char *path) {
    if (!path) {
        errno = EINVAL;
        return -1;
    }
    struct stat st;
    if (stat(path, &st) < 0) {
        return -1;
    }
    return (int64_t)st.st_size;
}

/* Returns 1 if `path` exists, 0 otherwise. Errors that aren't
 * "doesn't exist" (e.g. EACCES on a parent dir) also return 0;
 * the caller can disambiguate via errno if needed. */
int lotus_fs_file_exists(const char *path) {
    if (!path) {
        errno = EINVAL;
        return 0;
    }
    struct stat st;
    return stat(path, &st) == 0 ? 1 : 0;
}

/*
 * Held-open file substrate (std::io::file::File).
 *
 * Complements the one-shot std::io::fs::* path-calls above:
 * those open+do+close per call; this family hands a raw fd back
 * to the Aperio-side File locus, which stashes it on self.fd
 * and runs lotus_file_close in its dissolve(). The locus's
 * scope-exit dissolve (per let-bound deferred-dissolve rules)
 * makes "open a file, do held-state I/O, get cleanup for free"
 * the same shape Stream / Listener already have for sockets.
 *
 * Mode string semantics (POSIX flag derivation):
 *   "r"  → O_RDONLY
 *   "w"  → O_WRONLY | O_CREAT | O_TRUNC   (mode 0644)
 *   "a"  → O_WRONLY | O_CREAT | O_APPEND  (mode 0644)
 *   "r+" → O_RDWR
 *   "w+" → O_RDWR   | O_CREAT | O_TRUNC   (mode 0644)
 *
 * Returned String data (read_line) lives in the global bus
 * payload arena, matching the lifetime semantics of
 * std::io::fs::read_file — the buffer survives the call frame
 * and the eventual File.dissolve(), so a String escaping the
 * File's scope stays valid.
 */

/* Open `path` with POSIX mode derived from `mode_str`. Returns
 * the fd (>= 0) on success, -1 on error (errno set). The mode
 * string is one of {"r","w","a","r+","w+"} — anything else is
 * EINVAL. */
int lotus_file_open(const char *path, const char *mode_str) {
    if (!path || !mode_str) {
        errno = EINVAL;
        return -1;
    }
    int flags;
    int create_perm = 0;
    if (strcmp(mode_str, "r") == 0) {
        flags = O_RDONLY;
    } else if (strcmp(mode_str, "w") == 0) {
        flags = O_WRONLY | O_CREAT | O_TRUNC;
        create_perm = 0644;
    } else if (strcmp(mode_str, "a") == 0) {
        flags = O_WRONLY | O_CREAT | O_APPEND;
        create_perm = 0644;
    } else if (strcmp(mode_str, "r+") == 0) {
        flags = O_RDWR;
    } else if (strcmp(mode_str, "w+") == 0) {
        flags = O_RDWR | O_CREAT | O_TRUNC;
        create_perm = 0644;
    } else {
        errno = EINVAL;
        return -1;
    }
    int fd = (create_perm != 0)
        ? open(path, flags, create_perm)
        : open(path, flags);
    return fd;
}

/* Close a held-open fd. Returns 0 on success, -1 on error.
 * Idempotent for fd == -1 (the "never opened" sentinel) — that
 * lets File.dissolve() blind-close on the params default
 * without synthesizing a phantom IoError. */
int lotus_file_close(int fd) {
    if (fd < 0) return 0;
    return close(fd);
}

/* Read one '\n'-terminated line from `fd`, returning a heap-
 * allocated NUL-terminated string in the global bus payload
 * arena. The terminating newline is included if present
 * (matches getline(3) convention); EOF without a trailing
 * newline returns whatever bytes were read. EOF with no
 * available bytes returns an empty string (the stable empty
 * sentinel) — callers paired with lotus_file_at_eof can
 * distinguish that from a genuine empty line. Read errors
 * also collapse to the empty-string sentinel; surfacing them
 * is a v1.x follow-up (the at_eof / read_line loop pattern
 * doesn't have a clean fallible attach point because of the
 * empty-line vs EOF ambiguity without a sum-type result).
 *
 * Capped at 8 MB per line; longer lines are truncated at the
 * cap and the caller sees the partial line (no error). The
 * truncation is deliberate over a stream-extension API for v1
 * simplicity — typical config / log lines are well under this. */
#define LOTUS_FILE_LINE_CAP (8u * 1024u * 1024u)

const char *lotus_file_read_line_global(int fd) {
    static const char empty[1] = { 0 };
    if (fd < 0) {
        errno = EINVAL;
        return empty;
    }
    /* Single-byte reads — getline(3) would be slicker but
     * requires a FILE* and we hold a raw fd. The kernel
     * read-ahead cache makes this not-terrible for typical
     * input sizes; if it shows up in a profile, swap to a
     * readbuf per-fd later. */
    char  stack_buf[4096];
    char *heap_buf = NULL;
    size_t cap = sizeof(stack_buf);
    size_t len = 0;
    char *buf = stack_buf;
    for (;;) {
        if (len >= LOTUS_FILE_LINE_CAP) {
            break;     /* truncate at cap */
        }
        if (len + 1 >= cap) {
            size_t new_cap = cap * 2;
            if (new_cap > LOTUS_FILE_LINE_CAP + 1) {
                new_cap = LOTUS_FILE_LINE_CAP + 1;
            }
            char *grown = (char *)realloc(
                (buf == stack_buf) ? NULL : heap_buf, new_cap);
            if (!grown) {
                if (heap_buf) free(heap_buf);
                return empty;
            }
            if (buf == stack_buf) {
                memcpy(grown, stack_buf, len);
            }
            heap_buf = grown;
            buf      = grown;
            cap      = new_cap;
        }
        char ch;
        ssize_t r = read(fd, &ch, 1);
        if (r > 0) {
            buf[len++] = ch;
            if (ch == '\n') break;
            continue;
        }
        if (r == 0) {
            /* EOF: if we have buffered bytes, return them as
             * the last (unterminated) line. If we have nothing,
             * return the stable empty-string sentinel. */
            if (len == 0) {
                if (heap_buf) free(heap_buf);
                return empty;
            }
            break;
        }
        if (errno == EINTR) continue;
        /* Read error: collapse to empty-string for the v1
         * non-fallible surface. Caller's at_eof() loop terminates
         * naturally because subsequent reads also hit EOF/error. */
        if (heap_buf) free(heap_buf);
        return empty;
    }
    buf[len] = '\0';
    /* Anchor result in the global bus payload arena so it
     * outlives the call frame AND the File locus's dissolve. */
    char *out = (char *)lotus_bus_payload_arena_alloc(len + 1, 1);
    if (!out) {
        if (heap_buf) free(heap_buf);
        return empty;
    }
    memcpy(out, buf, len + 1);
    if (heap_buf) free(heap_buf);
    return out;
}

/* Returns 1 if `fd` has no more bytes to read, 0 otherwise, -1
 * on error. Implemented via lseek(SEEK_CUR) vs lseek(SEEK_END)
 * comparison — works for regular files; for pipes / sockets the
 * function returns -1 with errno=ESPIPE (caller should not use
 * at_eof on non-seekable fds). */
int lotus_file_at_eof(int fd) {
    if (fd < 0) {
        errno = EINVAL;
        return -1;
    }
    off_t cur = lseek(fd, 0, SEEK_CUR);
    if (cur < 0) return -1;
    off_t end = lseek(fd, 0, SEEK_END);
    if (end < 0) return -1;
    /* Restore position. */
    if (lseek(fd, cur, SEEK_SET) < 0) return -1;
    return (cur >= end) ? 1 : 0;
}

/* Seek `fd` to absolute byte offset `offset`. Returns 0 on
 * success, -1 on error. */
int lotus_file_seek(int fd, int64_t offset) {
    if (fd < 0 || offset < 0) {
        errno = EINVAL;
        return -1;
    }
    return (lseek(fd, (off_t)offset, SEEK_SET) >= 0) ? 0 : -1;
}

/* Write all `len` bytes from `buf` to `fd`, looping over short
 * writes. Returns 0 on success, -1 on error. */
int lotus_file_write_all(int fd, const void *buf, size_t len) {
    if (fd < 0 || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    const char *p = (const char *)buf;
    size_t left = len;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        return -1;
    }
    return 0;
}

/* Surface the current platform errno to the LLVM-side fallible-
 * dispatch wrappers. Each `std::io::fs::*` / `std::io::tcp::*`
 * primitive sets errno on failure; the codegen-side wrapper
 * reads it back via this helper and synthesizes an `IoError`
 * payload. Same global-state contract as POSIX itself — assumes
 * the wrapper calls this immediately after the failing call
 * with no intervening errno-setting syscalls. */
int32_t lotus_get_errno(void) {
    return (int32_t)errno;
}

/* Map a platform errno code to a stable kind-tag string the
 * IoError payload carries. Returns a pointer into a static
 * table; caller must not free. The kind taxonomy is the
 * agent-facing vocabulary — keep it small and intuitive.
 * Unmapped codes return "io" as the catch-all. */
const char *lotus_io_error_kind(int32_t errno_val) {
    switch (errno_val) {
        case 0:           return "";
        case ENOENT:      return "not_found";
        case EACCES:      return "permission_denied";
        case EPERM:       return "permission_denied";
        case EISDIR:      return "is_dir";
        case ENOTDIR:     return "not_dir";
        case EEXIST:      return "already_exists";
        case ENOTEMPTY:   return "not_empty";
        case ENOSPC:      return "no_space";
        case ENAMETOOLONG: return "name_too_long";
        case EINVAL:      return "invalid";
        case EAGAIN:      return "would_block";
#if defined(EWOULDBLOCK) && (EWOULDBLOCK != EAGAIN)
        case EWOULDBLOCK: return "would_block";
#endif
        case ETIMEDOUT:   return "timeout";
        case ECONNREFUSED: return "connection_refused";
        case ECONNRESET:  return "connection_reset";
        case ECONNABORTED: return "connection_aborted";
        case EHOSTUNREACH: return "host_unreachable";
        case ENETUNREACH: return "network_unreachable";
        case EADDRINUSE:  return "address_in_use";
        case EPIPE:       return "broken_pipe";
        case EINTR:       return "interrupted";
        /* C2 (pond/subprocess): subprocess-specific errnos. ESRCH is
         * "no such process" (typically from kill() against a pid
         * that has already been reaped). ECHILD is "no child
         * processes" (waitpid against a non-child). E2BIG surfaces
         * when argv is over the kernel limit at exec time. */
        case ESRCH:       return "not_found";
        case ECHILD:      return "not_found";
        case E2BIG:       return "invalid";
        default:          return "io";
    }
}

/* Locates the extension within `path` — including the leading
 * dot (".go", ".md") — or returns NULL when there is no
 * extension. The lookup operates on the basename: a dot inside
 * an earlier directory segment ("a.b/c") does NOT count as the
 * file's extension, and a leading-dot file (".bashrc",
 * "src/.config") has no extension by this rule. Mirrors the
 * conventional split used by Python's os.path.splitext and
 * Rust's Path::extension.
 *
 * Internal helper: the returned pointer (when non-NULL) aliases
 * `path`. External callers go through lotus_fs_extension_global,
 * which copies the slice into the program-lifetime payload arena
 * so the result is safe to stash past the call frame. */
static const char *lotus_fs_extension_locate(const char *path) {
    if (!path) return NULL;
    const char *base = path;
    for (const char *p = path; *p; p++) {
        if (*p == '/') base = p + 1;
    }
    const char *dot = NULL;
    for (const char *p = base; *p; p++) {
        if (*p == '.' && p != base) dot = p;
    }
    return dot;
}

/*
 * m77: process environment + argv access.
 *
 * Captures argc/argv in main's prelude (codegen emits a call
 * to lotus_env_init at the top of main, before any user code
 * runs) and exposes:
 *
 *   - args_count: argc
 *   - arg(i):     argv[i] for valid i, else stable empty string
 *   - var(name):  getenv(name) or stable empty string
 *   - var_exists: getenv(name) != NULL
 *
 * Aperio Strings need NUL-terminated, pointer-stable buffers.
 * argv entries and getenv returns satisfy both (POSIX: argv
 * strings are NUL-terminated and live for main's lifetime;
 * getenv returns valid until a setenv/putenv we don't have a
 * surface for in v0). The empty-string sentinel is a single
 * NUL byte at static address — also pointer-stable for the
 * program's life.
 */
static int          g_argc       = 0;
static char *const *g_argv       = NULL;
static const char   g_empty_str[1] = { 0 };

void lotus_env_init(int argc, char *const *argv) {
    g_argc = argc;
    g_argv = argv;
}

/*
 * 2026-05-17 — stdout buffering discipline.
 *
 * libc fully-buffers stdout when it isn't a TTY (pipes, files,
 * subprocess captures). That's wrong for Aperio's contract:
 * `println("READY"); accept_blocking_call();` should make
 * "READY\n" visible immediately, not on accept's return — pipe
 * consumers (test oracles, supervisors waiting for a READY
 * handshake, log tailers) hang forever otherwise.
 *
 * Switch stdout to line-buffered globally so `\n`-terminated
 * `println` flushes on the newline regardless of how stdout is
 * connected. Matches Python's `python -u` discipline + Go's
 * default. Called once from main's prelude.
 *
 * stderr is already line-buffered per POSIX; we don't touch it.
 *
 * C2 (pond/subprocess) addendum: also ignore SIGPIPE globally so a
 * write to a closed pipe (the canonical hazard when a subprocess
 * exits before the parent finishes draining its stdin pipe) returns
 * EPIPE instead of killing the parent. Affects every Aperio
 * program — but the contract Aperio offers is "writes to broken
 * pipes return EPIPE via the IoError channel" not "the OS kills
 * your program when a stream closes", so the global flip is the
 * right default. Cost: programs that need SIGPIPE-driven termination
 * (rare) lose it.
 */
void lotus_io_init(void) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    /* SIG_IGN return is "previous action" — discarding it is fine;
     * the only way this fails is signum out of range, which can't
     * happen for SIGPIPE. */
    signal(SIGPIPE, SIG_IGN);
}

int lotus_env_args_count(void) {
    return g_argc;
}

const char *lotus_env_arg(int i) {
    if (i < 0 || i >= g_argc || !g_argv || !g_argv[i]) {
        return g_empty_str;
    }
    return g_argv[i];
}

const char *lotus_env_var(const char *name) {
    if (!name) return g_empty_str;
    const char *v = getenv(name);
    return v ? v : g_empty_str;
}

int lotus_env_var_exists(const char *name) {
    if (!name) return 0;
    return getenv(name) != NULL ? 1 : 0;
}

/*
 * Standard input — `std::io::stdin::read_line` substrate.
 *
 * Reads one line from stdin via POSIX getline(3) and copies the
 * content (with trailing newline stripped) into the lazy global
 * payload arena so the returned String is pointer-stable for
 * the program's lifetime. The libc getline buffer is freed
 * after the copy.
 *
 * Returns "" (the static empty-string sentinel) on EOF or
 * read error. Empty input lines (`\n` with no other content)
 * also return "" — the EOF-vs-empty-line collision is
 * documented in spec/stdlib.md; programs that need to
 * distinguish drive the read through a sibling status getter
 * (see lotus_stdin_read_line_status below).
 */
static int g_stdin_last_status = 0;
/*  0 = success (line was read; possibly empty)
 * -1 = EOF (no bytes read before EOF)
 * -2 = IO error (errno set; getline returned -1 with non-EOF)
 * -3 = OOM in payload arena (alloc returned NULL after a read)
 */

const char *lotus_stdin_read_line(void) {
    char *line = NULL;
    size_t cap = 0;
    errno = 0;
    ssize_t n = getline(&line, &cap, stdin);
    if (n < 0) {
        free(line);
        if (feof(stdin)) {
            g_stdin_last_status = -1;
        } else {
            g_stdin_last_status = -2;
        }
        return g_empty_str;
    }
    /* Strip the trailing '\n' (and optional '\r' before it) so
     * callers don't have to. getline preserves the newline; we
     * normalize here once. */
    if (n > 0 && line[n - 1] == '\n') {
        n--;
        if (n > 0 && line[n - 1] == '\r') {
            n--;
        }
    }
    char *out = (char *)lotus_bus_payload_arena_alloc((size_t)n + 1, 1);
    if (!out) {
        free(line);
        g_stdin_last_status = -3;
        return g_empty_str;
    }
    if (n > 0) {
        memcpy(out, line, (size_t)n);
    }
    out[n] = '\0';
    free(line);
    g_stdin_last_status = 0;
    return out;
}

/* Returns the status of the most recent lotus_stdin_read_line
 * call: 0 success, -1 EOF, -2 IO error, -3 OOM. Lets callers
 * distinguish "empty input line" (status 0, len 0) from "EOF"
 * (status -1, len 0). */
int lotus_stdin_read_line_status(void) {
    return g_stdin_last_status;
}

/*
 * m78: minimal string parsing primitives.
 *
 * Atoi-style: returns 0 when the input doesn't look like an
 * integer. Callers that need to distinguish "0" from "bad
 * input" probe with the boolean sibling. Implemented via
 * strtoll so leading whitespace and a leading sign are
 * accepted, but trailing garbage rejects (the strict shape).
 *
 * v0 scope: signed 64-bit integers in base 10. Hex / octal /
 * underscores wait on a richer parsing library. The
 * sufficient case for "parse a port from argv" is base 10
 * with optional leading minus.
 */

int64_t lotus_str_parse_int(const char *s) {
    if (!s || !*s) return 0;
    char *end = NULL;
    errno = 0;
    long long v = strtoll(s, &end, 10);
    if (errno != 0 || !end || *end != '\0') {
        return 0;
    }
    return (int64_t)v;
}

int lotus_str_can_parse_int(const char *s) {
    if (!s || !*s) return 0;
    char *end = NULL;
    errno = 0;
    (void)strtoll(s, &end, 10);
    if (errno != 0 || !end || *end != '\0') {
        return 0;
    }
    return 1;
}

/*
 * v1.x-16: parse_float / can_parse_float.
 * Strict trailing-NUL parse — empty / non-numeric / partial-tail
 * inputs return 0.0 and 0 respectively. Matches the parse_int
 * contract: a "soft" check function lets callers gate on
 * parseability and the parser returns 0 on failure for surface
 * code that wants a defaulting shape.
 */
double lotus_str_parse_float(const char *s) {
    if (!s || !*s) return 0.0;
    char *end = NULL;
    errno = 0;
    double v = strtod(s, &end);
    if (errno != 0 || !end || *end != '\0') {
        return 0.0;
    }
    return v;
}

int lotus_str_can_parse_float(const char *s) {
    if (!s || !*s) return 0;
    char *end = NULL;
    errno = 0;
    (void)strtod(s, &end);
    if (errno != 0 || !end || *end != '\0') {
        return 0;
    }
    return 1;
}

/*
 * m58: deployment-config subject binding.
 *
 * Layered on top of the m57 AF_UNIX transport: a startup config
 * file maps each `bus subscribe` / publish subject to a transport
 * URL (currently only `unix://<path>`). Source stays transport-
 * agnostic per notes/open-questions #8 — the binding lives
 * entirely in deployment-config.
 *
 * Codegen emits one call to lotus_bus_load_config in main's
 * prelude:
 *
 *     lotus_bus_load_config(getenv("LOTUS_BUS_CONFIG"));
 *
 * If the env var is unset (or the file is unreadable),
 * lotus_bus_load_config no-ops and the binary behaves as a
 * single-process program — matches the m45-followup baseline so
 * existing examples are unaffected.
 *
 * Wire format: one entry per line, `subject=url:role`. Comments
 * begin with '#' and run to end-of-line. Whitespace is trimmed
 * around all three tokens. role is `listen` or `connect`. The
 * role is per-binary, per-subject — two binaries on the same
 * subject must declare opposite roles in their respective configs.
 *
 * v0.1 supports CONNECT-side dispatch only: a publisher with a
 * CONNECT-role binding fans out via lotus_transport_send during
 * lotus_bus_dispatch. LISTEN-side accept-and-spawn-reader-thread
 * is m59+ — at this milestone the listener role is exercised by
 * the m57 transport_driver harness so the full publisher pipeline
 * can be verified end-to-end without yet wiring receive-side
 * dispatch. v0.1 also supports exactly one peer per subject; the
 * fanout cardinality story (multi-peer per subject, multi-subject
 * per peer) is m60.
 */

/* Wave B (bus-transport redesign): an entry is one of two kinds.
 * UNIX = substrate-provided AF_UNIX transport; ADAPTER = user-
 * supplied protocol-layer locus (NATS, MQTT, raw-TCP-with-framing,
 * ...) whose `send` method receives outbound payloads. */
#define LOTUS_BUS_REMOTE_KIND_UNIX    0
#define LOTUS_BUS_REMOTE_KIND_ADAPTER 1

typedef struct lotus_bus_remote_entry {
    char              *subject;       /* owned (strdup'd at register) */
    int                kind;          /* one of LOTUS_BUS_REMOTE_KIND_* */
    /* --- UNIX fields (valid when kind == UNIX) --- */
    lotus_transport_t *transport;     /* set in main for CONNECT,
                                         in reader-thread for LISTEN */
    int                role;
    /* m59: per-subject reader thread for LISTEN role. Set when the
     * pthread is spawned at register time; the thread loops on
     * lotus_transport_recv and dispatches to local subscribers via
     * lotus_bus_local_dispatch. CONNECT-role entries leave both
     * fields zero (no thread, transport opened on the main path). */
    pthread_t          reader_thread;
    int                has_reader_thread;
    /* --- ADAPTER fields (valid when kind == ADAPTER) --- */
    /* `adapter_self` is the adapter locus's self pointer; held
     * in the program-lifetime payload arena by codegen so it
     * outlives main. `adapter_send_fn` is the address of the
     * locus's `send(subject: String, bytes: Bytes)` method,
     * resolved at codegen time. The runtime invokes it directly
     * without going through the F.20 vtable. */
    void              *adapter_self;
    void             (*adapter_send_fn)(void *self,
                                        const char *subject,
                                        void *bytes);
} lotus_bus_remote_entry_t;

static lotus_bus_remote_entry_t *g_bus_remote_entries = NULL;
static size_t g_bus_remote_count = 0;
static size_t g_bus_remote_cap   = 0;

static inline int lotus_bus_has_remote_entries(void) {
    return g_bus_remote_count > 0;
}

#define LOTUS_BUS_REMOTE_INITIAL_CAP 4

/* m59: queue pointer published by the codegen prelude (via
 * lotus_bus_set_queue) so reader threads can dispatch into the
 * cooperative-subscriber path without plumbing the queue through
 * the transport layer. NULL until the codegen prelude runs;
 * reader threads handle the NULL case by skipping cooperative
 * dispatch (pinned subscribers via mailbox always work). */
static lotus_bus_queue_t *g_bus_queue_for_remote = NULL;

void lotus_bus_set_queue(lotus_bus_queue_t *queue) {
    g_bus_queue_for_remote = queue;
}

/* m59: reader-thread args. Owns the path string so the thread
 * can outlive the lotus_bus_register_remote call. The entry
 * back-reference lets the thread publish its transport ptr to
 * the entry so lotus_bus_remote_destroy_all can find it. */
typedef struct lotus_bus_reader_args {
    char                     *path;       /* owned by the thread */
    lotus_bus_remote_entry_t *entry;
} lotus_bus_reader_args_t;

static void *lotus_bus_reader_thread_main(void *arg) {
    lotus_bus_reader_args_t *args = (lotus_bus_reader_args_t *)arg;
    /* Open the LISTEN transport HERE, on the reader thread, so
     * accept() blocks the reader thread instead of main's boot
     * path. m58 opened transports inline in register_remote which
     * meant a subscriber binary would hang at startup until the
     * publisher connected; m59 defers the accept off the boot
     * path so main proceeds and any local-subscribe registration
     * can complete before we wait for a peer. */
    lotus_transport_t *t = lotus_transport_create(
        args->path, LOTUS_TRANSPORT_LISTEN);
    if (!t) {
        free(args->path);
        free(args);
        return NULL;
    }
    /* Publish the transport pointer back to the entry so
     * lotus_bus_remote_destroy_all can shutdown(2) the connection
     * if a clean teardown is needed. (Race: between accept
     * returning and this store, destroy_all sees NULL and skips
     * the shutdown — that's fine because in well-formed test
     * scenarios destroy_all runs after the peer has closed,
     * which already drives recv to EOF.) */
    args->entry->transport = t;

    char wire_buf[LOTUS_PAYLOAD_MAX];
    char struct_buf[LOTUS_PAYLOAD_MAX];
    while (1) {
        ssize_t n = lotus_transport_recv(t, wire_buf, sizeof(wire_buf));
        if (n <= 0) break;     /* peer closed (0) or error (-1) */

        /* m60: deserialize wire bytes into struct-layout bytes
         * before handing them to local dispatch. Look up the
         * deserialize_fn from the FIRST local entry matching
         * this subject — by language constraint all entries on
         * the same subject share the payload type, so any one
         * works. Skip dispatch if the type-checker mismatches
         * or there are no local subscribers (the recv'd bytes
         * have nowhere to go locally; that's not an error in
         * relay-shaped programs). */
        lotus_deserialize_fn deserialize = NULL;
        for (size_t i = 0; i < g_bus_count; i++) {
            lotus_bus_entry_t *e = &g_bus_entries[i];
            if (!e->subject) continue;
            /* m94: wildcard locals (e.g. "log.**") need to match
             * concrete remote-bound subjects too, so use the same
             * pattern-matching as the dispatch path. By language
             * constraint, all subscribers on the same subject
             * share the payload type, so the deserialize_fn from
             * any matching entry is the right one. */
            if (!lotus_subject_match(e->subject, args->entry->subject)) continue;
            deserialize = e->deserialize;
            break;
        }
        if (!deserialize) continue;
        ssize_t struct_size = deserialize(
            wire_buf, (size_t)n, struct_buf, sizeof(struct_buf));
        if (struct_size <= 0) continue;
        lotus_bus_local_dispatch(g_bus_queue_for_remote,
                                 args->entry->subject,
                                 struct_buf, (size_t)struct_size);
    }

    lotus_transport_destroy(t);
    args->entry->transport = NULL;     /* prevent double-destroy */
    free(args->path);
    free(args);
    return NULL;
}

void lotus_bus_register_remote(const char *subject,
                               const char *url,
                               int role) {
    if (!subject || !url) {
        fprintf(stderr,
                "lotus_bus_register_remote: null subject or url\n");
        return;
    }
    /* v1.x recognizes the `unix://` scheme only. User-supplied
     * adapters (NATS, TCP-with-framing, etc.) await Wave B of
     * the bus-transport redesign — see [[bus-transport-redesign]]
     * in the dev memory. */
    static const char unix_scheme[] = "unix://";
    size_t scheme_len = sizeof(unix_scheme) - 1;
    if (strncmp(url, unix_scheme, scheme_len) != 0) {
        fprintf(stderr,
                "lotus_bus_register_remote: unsupported URL scheme "
                "(only unix:// in v1.x): %s\n",
                url);
        return;
    }
    const char *path = url + scheme_len;
    if (*path == '\0') {
        fprintf(stderr,
                "lotus_bus_register_remote: empty path in %s\n", url);
        return;
    }

    /* Grow the table BEFORE doing any side effects so the
     * realloc-relocation can't invalidate a pointer we already
     * handed out (e.g., to a reader thread via reader_args). */
    if (g_bus_remote_count == g_bus_remote_cap) {
        size_t new_cap = g_bus_remote_cap == 0
            ? LOTUS_BUS_REMOTE_INITIAL_CAP
            : g_bus_remote_cap * 2;
        lotus_bus_remote_entry_t *grown = (lotus_bus_remote_entry_t *)
            realloc(g_bus_remote_entries,
                    new_cap * sizeof(lotus_bus_remote_entry_t));
        if (!grown) return;
        g_bus_remote_entries = grown;
        g_bus_remote_cap     = new_cap;
    }

    char *subject_copy = strdup(subject);
    if (!subject_copy) return;

    lotus_bus_remote_entry_t *e =
        &g_bus_remote_entries[g_bus_remote_count++];
    e->subject           = subject_copy;
    e->kind              = LOTUS_BUS_REMOTE_KIND_UNIX;
    e->transport         = NULL;
    e->role              = role;
    e->has_reader_thread = 0;
    e->adapter_self      = NULL;
    e->adapter_send_fn   = NULL;

    if (role == LOTUS_TRANSPORT_LISTEN) {
        /* m59: spawn a reader thread that owns this subject's
         * recv loop. The thread opens the LISTEN transport on
         * its own stack so accept() doesn't block the main
         * thread. */
        lotus_bus_reader_args_t *args =
            (lotus_bus_reader_args_t *)malloc(sizeof(*args));
        if (!args) return;
        args->path  = strdup(path);
        args->entry = e;
        if (!args->path) {
            free(args);
            return;
        }
        if (pthread_create(&e->reader_thread, NULL,
                           lotus_bus_reader_thread_main, args) != 0) {
            perror("lotus_bus_register_remote: pthread_create");
            free(args->path);
            free(args);
            return;
        }
        e->has_reader_thread = 1;
    } else {
        /* CONNECT: open inline so the connect-with-retry happens
         * on the boot path. The first publish on this subject
         * fans out through the resulting transport. */
        e->transport = lotus_transport_create(path, role);
        /* On failure lotus_transport_create already perror'd; the
         * entry stays in the table with transport=NULL so fanout
         * skips it and destroy_all is a no-op for this slot. */
    }
}

/* Wave B: register an adapter binding. The adapter locus has
 * already been instantiated by codegen with program-lifetime
 * allocation, and its `send(subject, bytes)` method's fn pointer
 * has been resolved. Outbound fanout to this subject invokes
 * `send_fn(self_data, subject_c_str, bytes_struct)` with a Bytes
 * value built from the local payload via lotus_bytes_from_buf
 * against the lazy global payload arena.
 *
 * Adapter entries don't open a transport or spawn a reader thread
 * — the adapter locus itself owns its protocol lifecycle through
 * its own birth/dissolve methods. destroy_all is a no-op for
 * adapter slots beyond freeing the subject string.
 */
void lotus_bus_register_remote_adapter(
    const char *subject,
    void *self_data,
    void (*send_fn)(void *self,
                    const char *subject,
                    void *bytes))
{
    if (!subject || !self_data || !send_fn) {
        fprintf(stderr,
                "lotus_bus_register_remote_adapter: null subject, "
                "self_data, or send_fn\n");
        return;
    }
    if (g_bus_remote_count == g_bus_remote_cap) {
        size_t new_cap = g_bus_remote_cap == 0
            ? LOTUS_BUS_REMOTE_INITIAL_CAP
            : g_bus_remote_cap * 2;
        lotus_bus_remote_entry_t *grown = (lotus_bus_remote_entry_t *)
            realloc(g_bus_remote_entries,
                    new_cap * sizeof(lotus_bus_remote_entry_t));
        if (!grown) return;
        g_bus_remote_entries = grown;
        g_bus_remote_cap     = new_cap;
    }
    char *subject_copy = strdup(subject);
    if (!subject_copy) return;
    lotus_bus_remote_entry_t *e =
        &g_bus_remote_entries[g_bus_remote_count++];
    e->subject           = subject_copy;
    e->kind              = LOTUS_BUS_REMOTE_KIND_ADAPTER;
    e->transport         = NULL;
    e->role              = 0;
    e->has_reader_thread = 0;
    e->adapter_self      = self_data;
    e->adapter_send_fn   = send_fn;
}

/* Trim leading + trailing whitespace in-place. Returns a pointer
 * into the same buffer; the caller still owns the allocation. */
static char *lotus__bus_strip(char *s) {
    while (*s == ' ' || *s == '\t') s++;
    char *end = s + strlen(s);
    while (end > s) {
        char c = end[-1];
        if (c != ' ' && c != '\t' && c != '\n' && c != '\r') break;
        end--;
    }
    *end = '\0';
    return s;
}

void lotus_bus_load_config(const char *path) {
    if (!path) return;
    FILE *fp = fopen(path, "r");
    if (!fp) {
        fprintf(stderr,
                "lotus_bus_load_config: cannot open %s: %s\n",
                path, strerror(errno));
        return;
    }
    char line[1024];
    int lineno = 0;
    while (fgets(line, sizeof(line), fp)) {
        lineno++;
        /* Strip end-of-line comments. */
        char *hash = strchr(line, '#');
        if (hash) *hash = '\0';
        char *trimmed = lotus__bus_strip(line);
        if (*trimmed == '\0') continue;

        char *eq = strchr(trimmed, '=');
        if (!eq) {
            fprintf(stderr,
                    "lotus_bus_load_config: %s:%d: missing '=' in '%s'\n",
                    path, lineno, trimmed);
            continue;
        }
        *eq = '\0';
        char *subject = lotus__bus_strip(trimmed);
        char *rest    = lotus__bus_strip(eq + 1);

        /* Split URL and role on the LAST ':'. URLs like
         * unix:///tmp/foo.sock contain a ':' inside the scheme,
         * so strrchr (last colon) reliably locates the role
         * suffix. */
        char *colon = strrchr(rest, ':');
        if (!colon || colon == rest) {
            fprintf(stderr,
                    "lotus_bus_load_config: %s:%d: missing ':role' "
                    "suffix on '%s'\n",
                    path, lineno, rest);
            continue;
        }
        *colon = '\0';
        char *url      = lotus__bus_strip(rest);
        char *role_str = lotus__bus_strip(colon + 1);

        int role_val;
        if (strcmp(role_str, "listen") == 0) {
            role_val = LOTUS_TRANSPORT_LISTEN;
        } else if (strcmp(role_str, "connect") == 0) {
            role_val = LOTUS_TRANSPORT_CONNECT;
        } else {
            fprintf(stderr,
                    "lotus_bus_load_config: %s:%d: unknown role "
                    "'%s' (expected 'listen' or 'connect')\n",
                    path, lineno, role_str);
            continue;
        }
        lotus_bus_register_remote(subject, url, role_val);
    }
    fclose(fp);
}

/* Forward-declared at the top of the bus router section so
 * lotus_bus_dispatch can fan out to remote subscribers without
 * caring about table layout. */
void lotus_bus_remote_fanout(const char *subject,
                             const void *payload,
                             size_t payload_size) {
    if (!subject) return;
    for (size_t i = 0; i < g_bus_remote_count; i++) {
        lotus_bus_remote_entry_t *e = &g_bus_remote_entries[i];
        if (!e->subject) continue;
        if (strcmp(e->subject, subject) != 0) continue;
        if (e->kind == LOTUS_BUS_REMOTE_KIND_ADAPTER) {
            /* Wave B: package the wire bytes as an Aperio-level
             * Bytes value (program-lifetime, lives in the payload
             * arena), then dispatch through the adapter locus's
             * `send` method. The adapter's body owns framing /
             * delivery — the bus only guarantees one whole message
             * per call. */
            if (!e->adapter_self || !e->adapter_send_fn) continue;
            lotus_arena_t *parena = lotus_bus_payload_arena_get();
            if (!parena) continue;
            void *bytes_val = lotus_bytes_from_buf(
                parena, payload, (int64_t)payload_size);
            if (!bytes_val) continue;
            e->adapter_send_fn(e->adapter_self, e->subject, bytes_val);
            continue;
        }
        if (!e->transport) continue;
        /* CONNECT role only fans out at this milestone. LISTEN
         * role transports exist on the receive side and are
         * driven by the (future) reader thread, not by publish-
         * site dispatch. */
        if (e->role != LOTUS_TRANSPORT_CONNECT) continue;
        (void)lotus_transport_send(e->transport, payload, payload_size);
        /* Errors are logged inside lotus_transport_send; we don't
         * abort dispatch on transport failure — local subscribers
         * already received their copy. */
    }
}

/* m105 body — forward-declared near lotus_bus_local_dispatch.
 * See the doc-comment up there for the design rationale. Lives
 * here because the function body references g_bus_queue_for_remote
 * (declared just above) and the per-subject deserialize_fn from
 * g_bus_entries. */
void lotus_bus_dispatch_wire(const char *subject,
                             const void *wire_bytes,
                             size_t wire_size) {
    if (!subject || !wire_bytes || wire_size == 0) return;
    lotus_deserialize_fn deserialize = NULL;
    for (size_t i = 0; i < g_bus_count; i++) {
        lotus_bus_entry_t *e = &g_bus_entries[i];
        if (!e->subject) continue;
        if (!lotus_subject_match(e->subject, subject)) continue;
        deserialize = e->deserialize;
        break;
    }
    if (!deserialize) return;
    char struct_buf[LOTUS_PAYLOAD_MAX];
    ssize_t struct_size = deserialize(
        wire_bytes, wire_size, struct_buf, sizeof(struct_buf));
    if (struct_size <= 0) return;
    lotus_bus_local_dispatch(g_bus_queue_for_remote,
                             subject,
                             struct_buf,
                             (size_t)struct_size);
}

/* m70: lazy global "payload arena" for String byte storage in
 * cross-process bus deserialization. The reader thread fills a
 * stack-local struct_buf, dispatches via lotus_bus_local_dispatch
 * (which copies struct_buf bytes into a queue cell), and after
 * drain copies the cell bytes into the subscriber's arena. Any
 * String pointer in struct_buf must outlive that whole chain —
 * the subscriber's arena isn't accessible at deserialize time
 * (we don't yet know which subscriber will fire; a subject can
 * have multiple), so we allocate from a long-lived shared arena
 * instead. Lifetime is the program; destroyed in
 * lotus_bus_remote_destroy_all. Memory grows unbounded —
 * acceptable for v1 (subscribers run for bounded duration). The
 * pthread mutex serializes allocator access since reader threads
 * call this concurrently. */
static lotus_arena_t   *g_bus_payload_arena       = NULL;
static pthread_mutex_t  g_bus_payload_arena_mutex = PTHREAD_MUTEX_INITIALIZER;

void *lotus_bus_payload_arena_alloc(size_t size, size_t align) {
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *p = lotus_arena_alloc(g_bus_payload_arena, size, align);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return p;
}

/* Wave B: handle-only accessor. Lazy-initializes the bus payload
 * arena on first call (same machinery as lotus_bus_payload_arena_alloc)
 * and returns the pointer. The bus adapter fanout path uses this
 * to hand a stable arena to lotus_bytes_from_buf so the Bytes
 * value lives past the dispatch call frame. */
lotus_arena_t *lotus_bus_payload_arena_get(void) {
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
    }
    lotus_arena_t *p = g_bus_payload_arena;
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return p;
}

/*
 * m89: read_bytes wrapper that anchors the resulting Bytes
 * blob in the lazy global payload arena (same lifetime
 * mechanism as read_file's String). Doing it this way keeps
 * the Bytes value valid for the entire program — a fn that
 * returns Bytes can rely on the pointer staying live past
 * the call site without m49-style deep-copy plumbing.
 */
void *lotus_fs_read_bytes_global(const char *path) {
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    /* lotus_fs_read_bytes allocates internally via
     * lotus_arena_alloc; we hold the mutex around it because
     * the global arena is shared across reader threads. */
    void *result = lotus_fs_read_bytes(g_bus_payload_arena, path);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return result;
}

/*
 * C4 (pond/crypto follow-up): cryptographically-strong random
 * bytes. Returns a fresh Bytes blob of length `n` anchored in
 * the bus payload arena, mirroring `lotus_fs_read_bytes_global`'s
 * lifetime story.
 *
 * Implementation order (Linux):
 *   1. `getrandom(buf, n, 0)` — modern syscall, retries on EINTR,
 *      handles short returns by looping.
 *   2. If `getrandom` is unavailable (ENOSYS) or `<sys/random.h>`
 *      isn't visible at build time, fall through to reading
 *      `/dev/urandom` until `n` bytes are filled.
 *
 * Argument shape:
 *   - `n <= 0`         → returns a length-0 Bytes blob, no error.
 *   - `n > GETRANDOM_MAX (8192)` → sets errno=EINVAL and returns
 *     NULL so the codegen-side wrapper synthesizes an IoError
 *     with kind="invalid". The cap is a per-call ergonomic
 *     limit, not the kernel's GRND_MAX (33,554,431) — agents
 *     pulling more than 8 KiB at once are almost certainly
 *     doing something wrong (key material is 16-64 bytes;
 *     session tokens are 16-32). They can loop if they really
 *     want more.
 *   - Any read error from the underlying source surfaces as
 *     NULL + errno from the failing call.
 */
#define LOTUS_GETRANDOM_PER_CALL_MAX 8192

void *lotus_os_getrandom(int64_t n) {
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            errno = ENOMEM;
            return NULL;
        }
    }
    /* n <= 0 → caller wants empty (no error). */
    if (n <= 0) {
        void *empty = lotus_bytes_create(g_bus_payload_arena, 0);
        pthread_mutex_unlock(&g_bus_payload_arena_mutex);
        return empty;
    }
    if (n > LOTUS_GETRANDOM_PER_CALL_MAX) {
        pthread_mutex_unlock(&g_bus_payload_arena_mutex);
        errno = EINVAL;
        return NULL;
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, n);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) {
        errno = ENOMEM;
        return NULL;
    }
    unsigned char *body = (unsigned char *)lotus_bytes_data(blob);
    size_t left = (size_t)n;
    unsigned char *p = body;

    /* Step 1: try getrandom(2). On ENOSYS, fall through. */
#if defined(__linux__) || defined(__GLIBC__)
    while (left > 0) {
        ssize_t r = getrandom(p, left, 0);
        if (r > 0) {
            p    += (size_t)r;
            left -= (size_t)r;
            continue;
        }
        if (r < 0 && errno == EINTR) continue;
        if (r < 0 && errno == ENOSYS) {
            /* Kernel too old (pre-3.17). Reset progress so the
             * urandom fallback rewrites the entire buffer. */
            p    = body;
            left = (size_t)n;
            break;
        }
        /* Any other error: surface it. */
        return NULL;
    }
    if (left == 0) {
        return blob;
    }
#endif

    /* Step 2: /dev/urandom fallback. Used on platforms without
     * the syscall and on Linux kernels too old to expose it. */
    int fd = open("/dev/urandom", O_RDONLY);
    if (fd < 0) {
        return NULL;
    }
    while (left > 0) {
        ssize_t r = read(fd, p, left);
        if (r > 0) {
            p    += (size_t)r;
            left -= (size_t)r;
            continue;
        }
        if (r < 0 && errno == EINTR) continue;
        /* r == 0 from /dev/urandom is "impossible" but treat as
         * EIO so the caller sees a useful kind. */
        if (r == 0) errno = EIO;
        close(fd);
        return NULL;
    }
    close(fd);
    return blob;
}

/*
 * ====================================================================
 * C2 — subprocess primitives.
 *
 * Two surfaces:
 *   1. `lotus_process_run` — synchronous fork+exec+wait. The 80%
 *      shell-out case: capture stdout/stderr/exit-code and hand
 *      the whole package back as a ProcessOutput struct.
 *   2. `lotus_process_spawn` + `_wait` + `_kill_escalate` +
 *      `_pipe_read_nonblocking` + `_pipe_write` — async lifecycle
 *      for long-running children (`pip install`, a TUI subprocess,
 *      a build job).
 *
 * Argv shape (both surfaces): newline-separated String. argv[0] is
 * the executable, argv[1..] are args. Trailing newline allowed but
 * not required. Aperio's array surface is statically-sized, so the
 * newline-blob is the v1 ergonomic compromise (mirrors cli.ap's
 * argv_keys: String pattern). Internal split-on-newline produces
 * the exec(2) argv array; the buffer is malloc'd, freed in the
 * caller frame after exec returns.
 *
 * Design notes:
 *   - SIGPIPE is ignored globally (see lotus_io_init); writes to a
 *     closed pipe surface EPIPE through errno instead of killing
 *     the parent.
 *   - Child gets its own process group (`setpgid(0, 0)` in child)
 *     so a parent crash doesn't strand orphans on shared session
 *     resources, and a `kill_escalate` against the pid can target
 *     the whole pgid later if we want to. setpgid over prctl(
 *     PR_SET_PDEATHSIG) because the former is POSIX (macOS / BSD
 *     work the same way) and the latter is Linux-only — auto-reap-
 *     on-parent-death is also too aggressive: a controlled Aperio
 *     dissolve already covers the orderly-shutdown path, and the
 *     hard-parent-crash case is handled at a higher level (systemd
 *     reaper, docker cgroup, etc.).
 *   - All fds are closed on every path. Subprocess code leaks fds
 *     more easily than anything else; every error-handling branch
 *     here explicitly drops the pipes.
 *   - kill_escalate is TERM → wait 100ms (polling via waitpid
 *     WNOHANG) → KILL → blocking waitpid to reap.
 * ====================================================================
 */

/* Per-call cap on argv blob size — defensive against pathological
 * inputs. Linux's ARG_MAX is typically 2MB; we cap lower so a
 * runaway concat in user code surfaces here, not after the kernel
 * rejects with E2BIG on exec. */
#define LOTUS_PROCESS_ARGV_MAX 65536

/* Internal: split `blob` (newline-separated) into a malloc'd argv
 * array with a trailing NULL slot. Returns 0 on success; on failure
 * (oversized, empty, OOM) sets errno and returns -1. On success,
 * the caller owns *out_argv (a single malloc — the entries point
 * into a sibling malloc'd buffer that's also returned via *out_buf
 * so the caller can free both). */
static int lotus_process_split_argv(
    const char *blob,
    char ***out_argv,
    char **out_buf,
    int *out_count
) {
    if (!blob) {
        errno = EINVAL;
        return -1;
    }
    size_t blob_len = strlen(blob);
    if (blob_len == 0) {
        errno = EINVAL;
        return -1;
    }
    if (blob_len > LOTUS_PROCESS_ARGV_MAX) {
        errno = E2BIG;
        return -1;
    }
    char *buf = (char *)malloc(blob_len + 1);
    if (!buf) {
        errno = ENOMEM;
        return -1;
    }
    memcpy(buf, blob, blob_len + 1);
    int count = 0;
    for (size_t i = 0; i < blob_len; i++) {
        if (buf[i] == '\n') count++;
    }
    /* If the last character isn't a newline, the tail is its own
     * arg. */
    if (blob_len > 0 && buf[blob_len - 1] != '\n') {
        count++;
    }
    if (count == 0) {
        free(buf);
        errno = EINVAL;
        return -1;
    }
    char **argv = (char **)malloc(sizeof(char *) * (size_t)(count + 1));
    if (!argv) {
        free(buf);
        errno = ENOMEM;
        return -1;
    }
    int idx = 0;
    char *start = buf;
    for (size_t i = 0; i < blob_len; i++) {
        if (buf[i] == '\n') {
            buf[i] = '\0';
            argv[idx++] = start;
            start = buf + i + 1;
        }
    }
    if (start < buf + blob_len) {
        argv[idx++] = start;
    }
    argv[idx] = NULL;
    *out_argv = argv;
    *out_buf = buf;
    *out_count = count;
    return 0;
}

/* C2 surface 1: synchronous run. fork → setpgid → exec in the
 * child; parent reads stdout + stderr to EOF (interleaved via
 * poll() so the child can write either stream in any order
 * without deadlocking), waits for exit.
 *
 * Out-params (all must be non-NULL):
 *   - *out_code   = exit code (-1 if killed by signal)
 *   - *out_signal = signal number (0 if normal exit)
 *   - *out_stdout = captured stdout (NUL-terminated, arena-anchored)
 *   - *out_stderr = captured stderr (NUL-terminated, arena-anchored)
 *
 * Returns 0 on success (fork+exec ok and waited), errno on
 * failure. ENOENT if the executable isn't found, EACCES if it's
 * not executable, ENOMEM on allocation failure, etc.
 */
int lotus_process_run(
    const char *argv_blob,
    int32_t *out_code,
    int32_t *out_signal,
    const char **out_stdout_str,
    const char **out_stderr_str
) {
    if (!out_code || !out_signal || !out_stdout_str || !out_stderr_str) {
        return EINVAL;
    }
    *out_code = -1;
    *out_signal = 0;
    *out_stdout_str = "";
    *out_stderr_str = "";

    char **argv = NULL;
    char *argv_buf = NULL;
    int argc = 0;
    if (lotus_process_split_argv(argv_blob, &argv, &argv_buf, &argc) < 0) {
        return errno ? errno : EINVAL;
    }

    int out_pipe[2] = { -1, -1 };
    int err_pipe[2] = { -1, -1 };
    if (pipe(out_pipe) < 0) {
        int saved = errno;
        free(argv); free(argv_buf);
        return saved;
    }
    if (pipe(err_pipe) < 0) {
        int saved = errno;
        close(out_pipe[0]); close(out_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }

    pid_t pid = fork();
    if (pid < 0) {
        int saved = errno;
        close(out_pipe[0]); close(out_pipe[1]);
        close(err_pipe[0]); close(err_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }
    if (pid == 0) {
        /* CHILD. setpgid first so a kill against this pid can
         * target the whole group later. Errors from setpgid are
         * non-fatal at exec time — proceed regardless. */
        setpgid(0, 0);
        if (dup2(out_pipe[1], STDOUT_FILENO) < 0) _exit(127);
        if (dup2(err_pipe[1], STDERR_FILENO) < 0) _exit(127);
        close(out_pipe[0]);
        close(out_pipe[1]);
        close(err_pipe[0]);
        close(err_pipe[1]);
        /* stdin from /dev/null so the child doesn't hang on read.
         * Best-effort — if /dev/null can't be opened, leave stdin
         * inherited from the parent. */
        int devnull = open("/dev/null", O_RDONLY);
        if (devnull >= 0) {
            dup2(devnull, STDIN_FILENO);
            close(devnull);
        }
        execvp(argv[0], argv);
        _exit(127);
    }

    /* PARENT. Close the write ends; we only read. */
    close(out_pipe[1]); out_pipe[1] = -1;
    close(err_pipe[1]); err_pipe[1] = -1;

    /* Drain both pipes via poll() so the child can write to
     * stdout and stderr in any order without filling either pipe
     * buffer and blocking. Naive "drain stdout to EOF then
     * stderr" deadlocks when the child writes >PIPE_BUF (~64KB)
     * to stderr without the parent reading. Cap each stream at
     * 16 MiB to bound parent memory against runaway children. */
    struct pollfd pfds[2];
    pfds[0].fd = out_pipe[0]; pfds[0].events = POLLIN;
    pfds[1].fd = err_pipe[0]; pfds[1].events = POLLIN;
    const size_t cap_bytes = 16 * 1024 * 1024;
    size_t out_cap = 4096, out_len = 0;
    size_t err_cap = 4096, err_len = 0;
    char *out_buf = (char *)malloc(out_cap);
    char *err_buf = (char *)malloc(err_cap);
    int drain_err = 0;
    if (!out_buf || !err_buf) {
        drain_err = ENOMEM;
    }
    int out_open = 1, err_open = 1;
    while (!drain_err && (out_open || err_open)) {
        pfds[0].events = out_open ? POLLIN : 0;
        pfds[1].events = err_open ? POLLIN : 0;
        int pn = poll(pfds, 2, -1);
        if (pn < 0) {
            if (errno == EINTR) continue;
            drain_err = errno;
            break;
        }
        if (out_open && (pfds[0].revents & (POLLIN | POLLHUP | POLLERR))) {
            if (out_len + 1 >= out_cap && out_cap < cap_bytes) {
                size_t nc = out_cap * 2;
                if (nc > cap_bytes) nc = cap_bytes + 1;
                char *nb = (char *)realloc(out_buf, nc);
                if (!nb) { drain_err = ENOMEM; break; }
                out_buf = nb; out_cap = nc;
            }
            ssize_t r = read(out_pipe[0], out_buf + out_len,
                             out_cap - out_len - 1);
            if (r > 0) {
                out_len += (size_t)r;
                if (out_len >= cap_bytes) out_open = 0;
            } else if (r == 0) {
                out_open = 0;
            } else if (errno != EINTR) {
                drain_err = errno;
                break;
            }
        }
        if (err_open && (pfds[1].revents & (POLLIN | POLLHUP | POLLERR))) {
            if (err_len + 1 >= err_cap && err_cap < cap_bytes) {
                size_t nc = err_cap * 2;
                if (nc > cap_bytes) nc = cap_bytes + 1;
                char *nb = (char *)realloc(err_buf, nc);
                if (!nb) { drain_err = ENOMEM; break; }
                err_buf = nb; err_cap = nc;
            }
            ssize_t r = read(err_pipe[0], err_buf + err_len,
                             err_cap - err_len - 1);
            if (r > 0) {
                err_len += (size_t)r;
                if (err_len >= cap_bytes) err_open = 0;
            } else if (r == 0) {
                err_open = 0;
            } else if (errno != EINTR) {
                drain_err = errno;
                break;
            }
        }
    }
    close(out_pipe[0]); out_pipe[0] = -1;
    close(err_pipe[0]); err_pipe[0] = -1;

    /* Reap. waitpid retries on EINTR. */
    int status = 0;
    for (;;) {
        pid_t w = waitpid(pid, &status, 0);
        if (w == pid) break;
        if (w < 0 && errno == EINTR) continue;
        if (!drain_err) drain_err = errno;
        break;
    }

    if (drain_err) {
        free(out_buf); free(err_buf);
        free(argv); free(argv_buf);
        return drain_err;
    }

    out_buf[out_len] = '\0';
    err_buf[err_len] = '\0';
    /* Anchor in the bus payload arena so the pointers outlive
     * this call frame. */
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            free(out_buf); free(err_buf);
            free(argv); free(argv_buf);
            return ENOMEM;
        }
    }
    char *out_anchored = (char *)lotus_arena_alloc(
        g_bus_payload_arena, out_len + 1, 1);
    char *err_anchored = (char *)lotus_arena_alloc(
        g_bus_payload_arena, err_len + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out_anchored || !err_anchored) {
        free(out_buf); free(err_buf);
        free(argv); free(argv_buf);
        return ENOMEM;
    }
    memcpy(out_anchored, out_buf, out_len + 1);
    memcpy(err_anchored, err_buf, err_len + 1);
    free(out_buf); free(err_buf);
    free(argv); free(argv_buf);

    *out_stdout_str = out_anchored;
    *out_stderr_str = err_anchored;

    /* Decode exit status. */
    if (WIFEXITED(status)) {
        *out_code = WEXITSTATUS(status);
        *out_signal = 0;
    } else if (WIFSIGNALED(status)) {
        *out_code = -1;
        *out_signal = WTERMSIG(status);
    } else {
        *out_code = -1;
        *out_signal = 0;
    }
    /* Distinguish exec failure (127) so the agent sees "not_found"
     * when execvp failed in the child. We do this here because the
     * child's _exit(127) is the only signal we have — we don't
     * route the exec errno across the fork boundary in v1. If
     * stderr is empty, this was almost certainly a child-side
     * exec failure. If the child wrote to stderr, it ran and
     * chose 127 — we trust that and don't override. */
    if (WIFEXITED(status) && WEXITSTATUS(status) == 127 && err_len == 0) {
        errno = ENOENT;
        return ENOENT;
    }
    return 0;
}

/* C2 surface 2a: async spawn. fork → setpgid → exec in the child;
 * parent gets back the pid + three pipe fds (stdin write, stdout
 * read, stderr read).
 *
 * Returns 0 on success, errno on failure. All out-params are
 * populated only on success.
 */
int lotus_process_spawn(
    const char *argv_blob,
    int32_t *out_pid,
    int32_t *out_stdin_fd,
    int32_t *out_stdout_fd,
    int32_t *out_stderr_fd
) {
    if (!out_pid || !out_stdin_fd || !out_stdout_fd || !out_stderr_fd) {
        return EINVAL;
    }
    char **argv = NULL;
    char *argv_buf = NULL;
    int argc = 0;
    if (lotus_process_split_argv(argv_blob, &argv, &argv_buf, &argc) < 0) {
        return errno ? errno : EINVAL;
    }

    int in_pipe[2]  = { -1, -1 };
    int out_pipe[2] = { -1, -1 };
    int err_pipe[2] = { -1, -1 };
    if (pipe(in_pipe) < 0) {
        int saved = errno;
        free(argv); free(argv_buf);
        return saved;
    }
    if (pipe(out_pipe) < 0) {
        int saved = errno;
        close(in_pipe[0]); close(in_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }
    if (pipe(err_pipe) < 0) {
        int saved = errno;
        close(in_pipe[0]); close(in_pipe[1]);
        close(out_pipe[0]); close(out_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }

    pid_t pid = fork();
    if (pid < 0) {
        int saved = errno;
        close(in_pipe[0]); close(in_pipe[1]);
        close(out_pipe[0]); close(out_pipe[1]);
        close(err_pipe[0]); close(err_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }
    if (pid == 0) {
        setpgid(0, 0);
        if (dup2(in_pipe[0], STDIN_FILENO) < 0) _exit(127);
        if (dup2(out_pipe[1], STDOUT_FILENO) < 0) _exit(127);
        if (dup2(err_pipe[1], STDERR_FILENO) < 0) _exit(127);
        close(in_pipe[0]);  close(in_pipe[1]);
        close(out_pipe[0]); close(out_pipe[1]);
        close(err_pipe[0]); close(err_pipe[1]);
        execvp(argv[0], argv);
        _exit(127);
    }

    /* PARENT. Close the child-side ends, keep our own. */
    close(in_pipe[0]);  in_pipe[0] = -1;
    close(out_pipe[1]); out_pipe[1] = -1;
    close(err_pipe[1]); err_pipe[1] = -1;

    /* Mark the parent-side pipe fds non-blocking so reads return
     * EAGAIN promptly instead of blocking. stdin write stays
     * blocking — the caller controls when to write, and a write
     * EAGAIN would confuse the common case. SIGPIPE is ignored
     * globally so a write after the child exits returns EPIPE
     * via the IoError channel. */
    int flags;
    flags = fcntl(out_pipe[0], F_GETFL, 0);
    if (flags >= 0) fcntl(out_pipe[0], F_SETFL, flags | O_NONBLOCK);
    flags = fcntl(err_pipe[0], F_GETFL, 0);
    if (flags >= 0) fcntl(err_pipe[0], F_SETFL, flags | O_NONBLOCK);

    free(argv); free(argv_buf);

    *out_pid = (int32_t)pid;
    *out_stdin_fd = (int32_t)in_pipe[1];
    *out_stdout_fd = (int32_t)out_pipe[0];
    *out_stderr_fd = (int32_t)err_pipe[0];
    return 0;
}

/* C2 surface 2b: blocking wait. Reaps the child, decodes the exit
 * status. Returns 0 on success, errno on failure. */
int lotus_process_wait(
    int32_t pid,
    int32_t *out_code,
    int32_t *out_signal
) {
    if (!out_code || !out_signal) return EINVAL;
    *out_code = -1;
    *out_signal = 0;
    int status = 0;
    for (;;) {
        pid_t w = waitpid((pid_t)pid, &status, 0);
        if (w == (pid_t)pid) break;
        if (w < 0 && errno == EINTR) continue;
        return errno ? errno : ECHILD;
    }
    if (WIFEXITED(status)) {
        *out_code = WEXITSTATUS(status);
        *out_signal = 0;
    } else if (WIFSIGNALED(status)) {
        *out_code = -1;
        *out_signal = WTERMSIG(status);
    }
    return 0;
}

/* C2 surface 2c: TERM → wait 100ms → KILL → reap. Returns 0 on
 * success (the process has been reaped or was already gone),
 * errno on failure. Idempotent against already-reaped children
 * (ESRCH from kill is treated as success since the goal is "this
 * pid is no longer running").
 *
 * The 100ms window is the SIGTERM grace period. Long enough that
 * a well-behaved child (one that handles SIGTERM by flushing +
 * exiting) finishes; short enough that a wedged child gets the
 * KILL hammer promptly. Polls via waitpid(WNOHANG) at 5ms
 * intervals so we exit early as soon as the child reaps.
 */
int lotus_process_kill_escalate(int32_t pid) {
    if (pid <= 0) return EINVAL;
    if (kill((pid_t)pid, SIGTERM) < 0) {
        if (errno != ESRCH) {
            return errno;
        }
    }
    const int poll_interval_us = 5000;
    const int total_us = 100 * 1000;
    int elapsed = 0;
    int status = 0;
    while (elapsed < total_us) {
        pid_t w = waitpid((pid_t)pid, &status, WNOHANG);
        if (w == (pid_t)pid) return 0;
        if (w < 0) {
            if (errno == EINTR) continue;
            if (errno == ECHILD) return 0;
            return errno;
        }
        struct timespec ts;
        ts.tv_sec = 0;
        ts.tv_nsec = (long)poll_interval_us * 1000L;
        nanosleep(&ts, NULL);
        elapsed += poll_interval_us;
    }
    if (kill((pid_t)pid, SIGKILL) < 0) {
        if (errno != ESRCH) return errno;
    }
    for (;;) {
        pid_t w = waitpid((pid_t)pid, &status, 0);
        if (w == (pid_t)pid) return 0;
        if (w < 0) {
            if (errno == EINTR) continue;
            if (errno == ECHILD) return 0;
            return errno;
        }
    }
}

/* C2 surface 2d: non-blocking read from a pipe fd opened by
 * lotus_process_spawn. Returns an arena-anchored NUL-terminated
 * string containing up to 64 KiB of available bytes.
 *
 * Return shapes:
 *   - non-empty string: bytes were available; copied into the
 *     bus_payload_arena.
 *   - empty string (""): EAGAIN / EWOULDBLOCK (no data available)
 *     OR EOF (child closed its write end). Use lotus_process_wait
 *     to distinguish.
 *   - NULL: hard error (EBADF, EIO, etc.) — errno set so the
 *     codegen-side wrapper synthesizes an IoError.
 */
const char *lotus_process_pipe_read_nonblocking(int32_t fd) {
    static const char empty[1] = { 0 };
    if (fd < 0) {
        errno = EBADF;
        return NULL;
    }
    char buf[65536];
    ssize_t r = read((int)fd, buf, sizeof(buf) - 1);
    if (r < 0) {
        if (errno == EAGAIN
#if defined(EWOULDBLOCK) && (EWOULDBLOCK != EAGAIN)
            || errno == EWOULDBLOCK
#endif
            || errno == EINTR) {
            return empty;
        }
        return NULL;
    }
    if (r == 0) {
        /* EOF — surface as empty. */
        return empty;
    }
    buf[r] = '\0';
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            errno = ENOMEM;
            return NULL;
        }
    }
    char *out = (char *)lotus_arena_alloc(
        g_bus_payload_arena, (size_t)r + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) {
        errno = ENOMEM;
        return NULL;
    }
    memcpy(out, buf, (size_t)r + 1);
    return out;
}

/* C2 surface 2e: write a string to a pipe fd opened by
 * lotus_process_spawn. Returns bytes written on success, -1 on
 * error (errno set). Writes the full strlen of `s` — embedded NULs
 * truncate. SIGPIPE is ignored globally so a write after the child
 * closed its read end surfaces as EPIPE through errno, not a
 * signal. */
int64_t lotus_process_pipe_write(int32_t fd, const char *s) {
    if (fd < 0) {
        errno = EBADF;
        return -1;
    }
    if (!s) {
        errno = EINVAL;
        return -1;
    }
    size_t total = strlen(s);
    size_t left = total;
    const char *p = s;
    while (left > 0) {
        ssize_t w = write((int)fd, p, left);
        if (w > 0) {
            p += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        return -1;
    }
    return (int64_t)total;
}

/*
 * m90: list_dir wrapper anchoring the resulting String in
 * the global payload arena. Same lifetime motivation as
 * read_bytes_global / read_file: callers can stash the
 * pointer past the call site without m49-style deep-copy
 * plumbing.
 */
const char *lotus_fs_list_dir_global(const char *path) {
    static const char empty[1] = { 0 };
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return empty;
        }
    }
    const char *result = lotus_fs_list_dir(g_bus_payload_arena, path);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return result;
}

/*
 * Extension lookup wrapper. Resolves the basename's last dot
 * (see lotus_fs_extension_locate) and copies the dot-prefixed
 * slice into the program-lifetime payload arena so the returned
 * String outlives the call frame — same convention as
 * read_file / list_dir / read_bytes. Returns the stable empty
 * string when there is no extension.
 */
const char *lotus_fs_extension_global(const char *path) {
    static const char empty[1] = { 0 };
    const char *ext = lotus_fs_extension_locate(path);
    if (!ext) return empty;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return empty;
        }
    }
    char *out = lotus_str_clone(g_bus_payload_arena, ext);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return out ? out : empty;
}

/*
 * Phase 2g: allocate a zero-length Bytes blob in the global
 * payload arena. Used as the "empty / error" return shape for
 * recv_bytes and the bytes_* helpers so callers always get a
 * well-formed blob (length=0 visible via lotus_bytes_len) rather
 * than NULL.
 */
static void *lotus_bytes_empty_global(void) {
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *empty = lotus_bytes_create(g_bus_payload_arena, 0);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return empty;
}

/*
 * Phase 2g: binary-safe TCP recv. Mirrors lotus_tcp_recv_str's
 * allocation + read(2) shape but builds a Bytes blob (length
 * prefix + body) instead of a NUL-terminated string, so embedded
 * NUL bytes survive intact. The blob is anchored in the lazy
 * global payload arena, matching the lifetime convention of
 * lotus_fs_read_bytes_global — callers can stash the pointer
 * past the call site without m49 deep-copy plumbing.
 *
 * Returns a Bytes blob with length 0 on fd/cap errors or EOF;
 * the caller distinguishes "empty" from "error" via the explicit
 * length, since the truncate-on-NUL hazard that motivated this
 * primitive is exactly the case where length-on-the-wire matters.
 */
void *lotus_tcp_recv_bytes(int fd, int max_bytes) {
    if (fd < 0 || max_bytes <= 0) {
        return lotus_bytes_empty_global();
    }
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    /* Allocate the body at the cap, read into it, then patch the
     * length prefix down to the actual bytes read. lotus_bytes_create
     * sets prefix=cap initially; partial reads (the common case)
     * need the prefix corrected so callers see the true length. */
    void *blob = lotus_bytes_create(g_bus_payload_arena, (int64_t)max_bytes);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) {
        return lotus_bytes_empty_global();
    }
    char *body = (char *)lotus_bytes_data(blob);
    ssize_t n;
    for (;;) {
        n = read(fd, body, (size_t)max_bytes);
        if (n >= 0) break;
        if (errno == EINTR) continue;
        /* read error: hand back an empty Bytes so downstream code
         * sees length=0 and can detect "nothing read". The reserved
         * arena memory leaks until program exit (matches recv_str's
         * convention). */
        return lotus_bytes_empty_global();
    }
    /* Patch the length prefix down to the actual bytes read. */
    *(int64_t *)blob = (int64_t)n;
    return blob;
}

/*
 * Phase 2g: Bytes → String conversion. Allocates a (len+1)-byte
 * buffer in the global payload arena, memcpys the Bytes body
 * into it, and NUL-terminates. Embedded NUL bytes survive in
 * the buffer but the resulting String's strlen-based view will
 * truncate at the first one — callers who need NUL-safe handling
 * should stay in Bytes. The conversion is for the common case
 * of "received bytes I'm pretty sure are UTF-8 / ASCII and want
 * to treat as a String".
 */
const char *lotus_str_from_bytes(const void *b) {
    static const char empty[1] = { 0 };
    if (!b) return empty;
    int64_t len = lotus_bytes_len(b);
    if (len <= 0) return empty;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return empty;
        }
    }
    char *buf = (char *)lotus_arena_alloc(
        g_bus_payload_arena, (size_t)len + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!buf) return empty;
    memcpy(buf, (const char *)b + sizeof(int64_t), (size_t)len);
    buf[(size_t)len] = '\0';
    return buf;
}

/*
 * Phase 2g: String → Bytes conversion. strlen the source string,
 * allocate a Bytes blob of that length in the global payload
 * arena, memcpy the body. Symmetric inverse of lotus_str_from_bytes.
 * Useful for handing String data to send_bytes when the payload
 * is text but the protocol surface demands the binary-safe call.
 */
void *lotus_bytes_from_str(const char *s) {
    if (!s) {
        return lotus_bytes_empty_global();
    }
    int64_t len = (int64_t)strlen(s);
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, len);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) {
        return lotus_bytes_empty_global();
    }
    memcpy(lotus_bytes_data(blob), s, (size_t)len);
    return blob;
}

/*
 * Phase 2g: byte-as-Int accessor — returns the i-th byte's
 * unsigned value (0..255) sign-extended into an Int (i64). Used
 * by binary protocol parsers (WebSocket frame headers, framing
 * length fields, etc.) that need to peek at a single byte. Out
 * of range (i < 0 or i >= len) returns -1 — bytes never go
 * negative on read, so -1 is a clean sentinel.
 */
int64_t lotus_bytes_at(const void *b, int64_t i) {
    if (!b) return -1;
    int64_t len = lotus_bytes_len(b);
    if (i < 0 || i >= len) return -1;
    const unsigned char *body =
        (const unsigned char *)b + sizeof(int64_t);
    return (int64_t)body[i];
}

/*
 * Phase 2g: Bytes slice — returns a fresh Bytes blob containing
 * the half-open range [lo, hi). Out-of-range bounds clamp to the
 * source length; hi <= lo yields an empty blob. The result is a
 * copy (not a view) so it composes with deep-copy-shaped lifetime
 * conventions; anchored in the global payload arena.
 */
void *lotus_bytes_slice(const void *b, int64_t lo, int64_t hi) {
    if (!b) return lotus_bytes_empty_global();
    int64_t len = lotus_bytes_len(b);
    if (lo < 0) lo = 0;
    if (hi > len) hi = len;
    if (hi <= lo) return lotus_bytes_empty_global();
    int64_t out_len = hi - lo;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, out_len);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) return lotus_bytes_empty_global();
    memcpy(
        lotus_bytes_data(blob),
        (const char *)b + sizeof(int64_t) + lo,
        (size_t)out_len);
    return blob;
}

/*
 * ws-echo `bytes-construction-from-ints`: build a one-byte Bytes
 * blob from an Int (low 8 bits). Companion to the recv side
 * for outbound binary protocols (WebSocket frame headers,
 * length-encoded prefixes, etc.). Anchored in the program-
 * lifetime payload arena so the returned pointer matches the
 * lifetime conventions of recv_bytes / bytes_slice. The Int
 * argument is taken mod 256 — callers that pre-mask explicitly
 * are no-ops; callers passing larger ints lose the high bits
 * silently, matching how `b << 8` truncates.
 */
void *lotus_bytes_from_int(int64_t v) {
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) return lotus_bytes_empty_global();
    unsigned char *body = (unsigned char *)lotus_bytes_data(blob);
    body[0] = (unsigned char)(v & 0xFF);
    return blob;
}

/*
 * ws-echo `bytes-construction-from-ints`: concatenate two Bytes
 * blobs into a fresh one. Composes with from_int to assemble
 * arbitrary outbound payloads (recursive: from_int + concat builds
 * any byte sequence). Either argument may be NULL/empty; the
 * result mirrors the non-empty side (or is empty if both are).
 */
void *lotus_bytes_concat(const void *a, const void *b) {
    int64_t la = a ? lotus_bytes_len(a) : 0;
    int64_t lb = b ? lotus_bytes_len(b) : 0;
    int64_t total = la + lb;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, total);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) return lotus_bytes_empty_global();
    char *body = (char *)lotus_bytes_data(blob);
    if (la > 0) {
        memcpy(body, (const char *)a + sizeof(int64_t), (size_t)la);
    }
    if (lb > 0) {
        memcpy(body + la, (const char *)b + sizeof(int64_t), (size_t)lb);
    }
    return blob;
}

/*
 * ws-echo `sha1-base64-missing`: SHA-1 of a Bytes blob,
 * returning the 20-byte digest as Bytes. Stand-alone
 * implementation per RFC 3174 to avoid pulling in OpenSSL
 * just for the WebSocket handshake. Single-shot API: no
 * streaming Update/Final pair; callers that need streaming
 * can build it on top.
 */
static uint32_t sha1_rotl(uint32_t v, int n) {
    return (v << n) | (v >> (32 - n));
}

void *lotus_crypto_sha1(const void *b) {
    int64_t len = b ? lotus_bytes_len(b) : 0;
    const unsigned char *msg =
        b ? (const unsigned char *)b + sizeof(int64_t) : NULL;

    uint32_t h0 = 0x67452301u;
    uint32_t h1 = 0xEFCDAB89u;
    uint32_t h2 = 0x98BADCFEu;
    uint32_t h3 = 0x10325476u;
    uint32_t h4 = 0xC3D2E1F0u;

    /* Build padded message: original + 0x80 + zeros + 8-byte big-endian
     * length (in bits). Total length is multiple of 64. */
    uint64_t bit_len = (uint64_t)len * 8u;
    int64_t padded_len = len + 1;   /* original + 0x80 */
    /* pad zeros until padded_len % 64 == 56 */
    int64_t mod = padded_len % 64;
    int64_t pad_zeros = (mod <= 56) ? (56 - mod) : (56 + 64 - mod);
    padded_len += pad_zeros + 8;     /* +8 for length field */

    unsigned char *buf = (unsigned char *)malloc((size_t)padded_len);
    if (!buf) return lotus_bytes_empty_global();
    if (len > 0) memcpy(buf, msg, (size_t)len);
    buf[len] = 0x80;
    for (int64_t i = len + 1; i < padded_len - 8; i++) buf[i] = 0;
    for (int i = 0; i < 8; i++) {
        buf[padded_len - 1 - i] = (unsigned char)(bit_len >> (i * 8));
    }

    for (int64_t off = 0; off < padded_len; off += 64) {
        uint32_t w[80];
        for (int i = 0; i < 16; i++) {
            w[i] = ((uint32_t)buf[off + i * 4 + 0] << 24)
                 | ((uint32_t)buf[off + i * 4 + 1] << 16)
                 | ((uint32_t)buf[off + i * 4 + 2] << 8)
                 | ((uint32_t)buf[off + i * 4 + 3]);
        }
        for (int i = 16; i < 80; i++) {
            w[i] = sha1_rotl(w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16], 1);
        }
        uint32_t a = h0, ba = h1, c = h2, d = h3, e = h4;
        for (int i = 0; i < 80; i++) {
            uint32_t f, k;
            if (i < 20)      { f = (ba & c) | (~ba & d);         k = 0x5A827999u; }
            else if (i < 40) { f = ba ^ c ^ d;                    k = 0x6ED9EBA1u; }
            else if (i < 60) { f = (ba & c) | (ba & d) | (c & d); k = 0x8F1BBCDCu; }
            else             { f = ba ^ c ^ d;                    k = 0xCA62C1D6u; }
            uint32_t temp = sha1_rotl(a, 5) + f + e + k + w[i];
            e = d;
            d = c;
            c = sha1_rotl(ba, 30);
            ba = a;
            a = temp;
        }
        h0 += a; h1 += ba; h2 += c; h3 += d; h4 += e;
    }
    free(buf);

    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, 20);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) return lotus_bytes_empty_global();
    unsigned char *dgst = (unsigned char *)lotus_bytes_data(blob);
    uint32_t hs[5] = { h0, h1, h2, h3, h4 };
    for (int i = 0; i < 5; i++) {
        dgst[i * 4 + 0] = (unsigned char)(hs[i] >> 24);
        dgst[i * 4 + 1] = (unsigned char)(hs[i] >> 16);
        dgst[i * 4 + 2] = (unsigned char)(hs[i] >> 8);
        dgst[i * 4 + 3] = (unsigned char)(hs[i]);
    }
    return blob;
}

/*
 * C3 (pond follow-up): SHA-256 per FIPS 180-4 of a Bytes blob,
 * returning the 32-byte digest as Bytes. Stand-alone — no
 * libcrypto / OpenSSL dependency. Single-shot API; callers that
 * need streaming can build it on top. Mirrors the lotus_crypto_sha1
 * surface (anchored in the bus payload arena).
 */
static const uint32_t lotus_sha256_k[64] = {
    0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u,
    0x3956c25bu, 0x59f111f1u, 0x923f82a4u, 0xab1c5ed5u,
    0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u,
    0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u,
    0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu,
    0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
    0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u,
    0xc6e00bf3u, 0xd5a79147u, 0x06ca6351u, 0x14292967u,
    0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u,
    0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u,
    0xa2bfe8a1u, 0xa81a664bu, 0xc24b8b70u, 0xc76c51a3u,
    0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
    0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u,
    0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu, 0x682e6ff3u,
    0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u,
    0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u,
};

static uint32_t lotus_sha256_rotr(uint32_t v, int n) {
    return (v >> n) | (v << (32 - n));
}

/* Core SHA-256 compute: hash `msg[0..len]` into `out[0..32]`.
 * `out` must point to a writable 32-byte buffer. Pure function;
 * no arena allocation here so HMAC can reuse it. */
static void lotus_sha256_compute(const unsigned char *msg,
                                  int64_t len,
                                  unsigned char out[32]) {
    uint32_t h0 = 0x6a09e667u;
    uint32_t h1 = 0xbb67ae85u;
    uint32_t h2 = 0x3c6ef372u;
    uint32_t h3 = 0xa54ff53au;
    uint32_t h4 = 0x510e527fu;
    uint32_t h5 = 0x9b05688cu;
    uint32_t h6 = 0x1f83d9abu;
    uint32_t h7 = 0x5be0cd19u;

    /* Padded message: original + 0x80 + zeros + 8-byte BE bit-length.
     * Total length a multiple of 64. */
    uint64_t bit_len = (uint64_t)len * 8u;
    int64_t padded_len = len + 1;
    int64_t mod = padded_len % 64;
    int64_t pad_zeros = (mod <= 56) ? (56 - mod) : (56 + 64 - mod);
    padded_len += pad_zeros + 8;

    unsigned char *buf = (unsigned char *)malloc((size_t)padded_len);
    if (!buf) {
        /* Out-of-memory: fall back to all-zero digest. Same shape
         * as the sha1 path's empty-bytes guard — recoverable in
         * principle by callers checking len. */
        memset(out, 0, 32);
        return;
    }
    if (len > 0) memcpy(buf, msg, (size_t)len);
    buf[len] = 0x80;
    for (int64_t i = len + 1; i < padded_len - 8; i++) buf[i] = 0;
    for (int i = 0; i < 8; i++) {
        buf[padded_len - 1 - i] = (unsigned char)(bit_len >> (i * 8));
    }

    for (int64_t off = 0; off < padded_len; off += 64) {
        uint32_t w[64];
        for (int i = 0; i < 16; i++) {
            w[i] = ((uint32_t)buf[off + i * 4 + 0] << 24)
                 | ((uint32_t)buf[off + i * 4 + 1] << 16)
                 | ((uint32_t)buf[off + i * 4 + 2] << 8)
                 | ((uint32_t)buf[off + i * 4 + 3]);
        }
        for (int i = 16; i < 64; i++) {
            uint32_t s0 = lotus_sha256_rotr(w[i - 15], 7)
                       ^ lotus_sha256_rotr(w[i - 15], 18)
                       ^ (w[i - 15] >> 3);
            uint32_t s1 = lotus_sha256_rotr(w[i - 2], 17)
                       ^ lotus_sha256_rotr(w[i - 2], 19)
                       ^ (w[i - 2] >> 10);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }

        uint32_t a = h0, b = h1, c = h2, d = h3;
        uint32_t e = h4, f = h5, g = h6, hh = h7;
        for (int i = 0; i < 64; i++) {
            uint32_t S1 = lotus_sha256_rotr(e, 6)
                       ^ lotus_sha256_rotr(e, 11)
                       ^ lotus_sha256_rotr(e, 25);
            uint32_t ch = (e & f) ^ ((~e) & g);
            uint32_t temp1 = hh + S1 + ch + lotus_sha256_k[i] + w[i];
            uint32_t S0 = lotus_sha256_rotr(a, 2)
                       ^ lotus_sha256_rotr(a, 13)
                       ^ lotus_sha256_rotr(a, 22);
            uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
            uint32_t temp2 = S0 + maj;
            hh = g;
            g = f;
            f = e;
            e = d + temp1;
            d = c;
            c = b;
            b = a;
            a = temp1 + temp2;
        }
        h0 += a; h1 += b; h2 += c; h3 += d;
        h4 += e; h5 += f; h6 += g; h7 += hh;
    }
    free(buf);

    uint32_t hs[8] = { h0, h1, h2, h3, h4, h5, h6, h7 };
    for (int i = 0; i < 8; i++) {
        out[i * 4 + 0] = (unsigned char)(hs[i] >> 24);
        out[i * 4 + 1] = (unsigned char)(hs[i] >> 16);
        out[i * 4 + 2] = (unsigned char)(hs[i] >> 8);
        out[i * 4 + 3] = (unsigned char)(hs[i]);
    }
}

void *lotus_crypto_sha256(const void *b) {
    int64_t len = b ? lotus_bytes_len(b) : 0;
    const unsigned char *msg =
        b ? (const unsigned char *)b + sizeof(int64_t) : NULL;

    unsigned char digest[32];
    lotus_sha256_compute(msg, len, digest);

    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, 32);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) return lotus_bytes_empty_global();
    memcpy(lotus_bytes_data(blob), digest, 32);
    return blob;
}

/*
 * C3 (pond follow-up): HMAC-SHA256 per RFC 2104.
 *   HMAC(K, m) = H((K' xor opad) || H((K' xor ipad) || m))
 * where K' = K padded to the block size (64 bytes for SHA-256):
 * zero-extended if |K| <= B, else H(K) zero-extended.
 *
 * Returns the 32-byte tag as Bytes anchored in the payload arena.
 */
void *lotus_crypto_hmac_sha256(const void *key_b, const void *msg_b) {
    const int B = 64; /* SHA-256 block size */

    int64_t klen = key_b ? lotus_bytes_len(key_b) : 0;
    const unsigned char *kraw =
        key_b ? (const unsigned char *)key_b + sizeof(int64_t) : NULL;
    int64_t mlen = msg_b ? lotus_bytes_len(msg_b) : 0;
    const unsigned char *mraw =
        msg_b ? (const unsigned char *)msg_b + sizeof(int64_t) : NULL;

    /* K' — key normalized to B bytes. */
    unsigned char kprime[64];
    memset(kprime, 0, B);
    if (klen > B) {
        unsigned char khash[32];
        lotus_sha256_compute(kraw, klen, khash);
        memcpy(kprime, khash, 32);
    } else if (klen > 0) {
        memcpy(kprime, kraw, (size_t)klen);
    }

    /* Inner: H((K' xor ipad) || m) */
    int64_t inner_len = (int64_t)B + mlen;
    unsigned char *inner_buf = (unsigned char *)malloc((size_t)inner_len);
    if (!inner_buf) return lotus_bytes_empty_global();
    for (int i = 0; i < B; i++) inner_buf[i] = kprime[i] ^ 0x36;
    if (mlen > 0) memcpy(inner_buf + B, mraw, (size_t)mlen);
    unsigned char inner_hash[32];
    lotus_sha256_compute(inner_buf, inner_len, inner_hash);
    free(inner_buf);

    /* Outer: H((K' xor opad) || inner_hash) */
    unsigned char outer_buf[64 + 32];
    for (int i = 0; i < B; i++) outer_buf[i] = kprime[i] ^ 0x5C;
    memcpy(outer_buf + B, inner_hash, 32);
    unsigned char tag[32];
    lotus_sha256_compute(outer_buf, B + 32, tag);

    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, 32);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) return lotus_bytes_empty_global();
    memcpy(lotus_bytes_data(blob), tag, 32);
    return blob;
}

/*
 * ws-echo `sha1-base64-missing`: Base64 encode a Bytes blob,
 * returning a NUL-terminated String (standard alphabet,
 * with `=` padding to a multiple of 4). Anchored in the
 * payload arena.
 */
static const char b64_alpha[] =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

const char *lotus_text_base64_encode(const void *b) {
    int64_t len = b ? lotus_bytes_len(b) : 0;
    const unsigned char *src =
        b ? (const unsigned char *)b + sizeof(int64_t) : NULL;
    int64_t out_len = ((len + 2) / 3) * 4;

    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    char *out = (char *)lotus_arena_alloc(
        g_bus_payload_arena, (size_t)(out_len + 1), 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return "";

    int64_t i = 0, j = 0;
    while (i + 3 <= len) {
        uint32_t v = ((uint32_t)src[i] << 16)
                   | ((uint32_t)src[i + 1] << 8)
                   |  (uint32_t)src[i + 2];
        out[j + 0] = b64_alpha[(v >> 18) & 0x3F];
        out[j + 1] = b64_alpha[(v >> 12) & 0x3F];
        out[j + 2] = b64_alpha[(v >> 6) & 0x3F];
        out[j + 3] = b64_alpha[v & 0x3F];
        i += 3;
        j += 4;
    }
    int64_t rem = len - i;
    if (rem == 1) {
        uint32_t v = (uint32_t)src[i] << 16;
        out[j + 0] = b64_alpha[(v >> 18) & 0x3F];
        out[j + 1] = b64_alpha[(v >> 12) & 0x3F];
        out[j + 2] = '=';
        out[j + 3] = '=';
        j += 4;
    } else if (rem == 2) {
        uint32_t v = ((uint32_t)src[i] << 16) | ((uint32_t)src[i + 1] << 8);
        out[j + 0] = b64_alpha[(v >> 18) & 0x3F];
        out[j + 1] = b64_alpha[(v >> 12) & 0x3F];
        out[j + 2] = b64_alpha[(v >> 6) & 0x3F];
        out[j + 3] = '=';
        j += 4;
    }
    out[j] = '\0';
    return out;
}

/*
 * v1.x-16: base64::decode. Inverse of lotus_text_base64_encode.
 * Returns a Bytes blob anchored in the bus payload arena.
 * Whitespace inside the input is ignored (RFC 4648 §3.3 — many
 * MIME-style encoders insert line breaks). Strictly rejects any
 * non-alphabet, non-whitespace, non-padding character by
 * returning a zero-length Bytes blob. Returns the empty blob for
 * empty / NULL input as well — callers should treat that as
 * either "empty source" or "decode failed".
 */
static int b64_decode_char(int c) {
    if (c >= 'A' && c <= 'Z') return c - 'A';
    if (c >= 'a' && c <= 'z') return c - 'a' + 26;
    if (c >= '0' && c <= '9') return c - '0' + 52;
    if (c == '+') return 62;
    if (c == '/') return 63;
    return -1;
}

void *lotus_text_base64_decode(const char *s) {
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);

    if (!s) {
        return lotus_bytes_create(g_bus_payload_arena, 0);
    }

    /* Count alphabet chars only (skip whitespace). Padding counts
     * toward the group-of-4 alignment check. */
    size_t alpha_count = 0;
    size_t pad_count = 0;
    for (const char *p = s; *p; p++) {
        unsigned char c = (unsigned char)*p;
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r') continue;
        if (c == '=') { pad_count++; continue; }
        if (b64_decode_char(c) < 0) {
            return lotus_bytes_create(g_bus_payload_arena, 0);
        }
        alpha_count++;
    }
    /* Total chars including padding must be a multiple of 4. */
    if ((alpha_count + pad_count) % 4 != 0) {
        return lotus_bytes_create(g_bus_payload_arena, 0);
    }
    /* At most 2 padding chars. */
    if (pad_count > 2) {
        return lotus_bytes_create(g_bus_payload_arena, 0);
    }
    /* Decoded length: each 4 input chars yield 3 bytes, minus padding. */
    int64_t total_chars = (int64_t)(alpha_count + pad_count);
    int64_t out_len = (total_chars / 4) * 3 - (int64_t)pad_count;
    if (out_len < 0) out_len = 0;

    void *blob = lotus_bytes_create(g_bus_payload_arena, out_len);
    if (!blob || out_len == 0) {
        return blob;
    }
    unsigned char *out = (unsigned char *)lotus_bytes_data(blob);

    uint32_t buf = 0;
    int bits = 0;
    int64_t j = 0;
    for (const char *p = s; *p; p++) {
        unsigned char c = (unsigned char)*p;
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r') continue;
        if (c == '=') break;
        int v = b64_decode_char(c);
        buf = (buf << 6) | (uint32_t)v;
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            if (j < out_len) {
                out[j++] = (unsigned char)((buf >> bits) & 0xFF);
            }
        }
    }
    return blob;
}

/*
 * ws-echo `random-seed-missing`: minimal RNG surface. xorshift64*
 * seeded from monotonic time (cheap, library-internal use only
 * — NOT cryptographic). Suitable for nonces, retry jitter, test
 * shuffles. Single shared state guarded by a mutex; v1 doesn't
 * try to be thread-safe-fast.
 */
static uint64_t g_rand_state = 0;
static pthread_mutex_t g_rand_mutex = PTHREAD_MUTEX_INITIALIZER;

void lotus_rand_seed_from_time(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    uint64_t s = (uint64_t)ts.tv_sec * 1000000000ULL
               + (uint64_t)ts.tv_nsec;
    if (s == 0) s = 0x9E3779B97F4A7C15ULL;     /* avoid 0 */
    pthread_mutex_lock(&g_rand_mutex);
    g_rand_state = s;
    pthread_mutex_unlock(&g_rand_mutex);
}

int64_t lotus_rand_next_int(int64_t max) {
    pthread_mutex_lock(&g_rand_mutex);
    if (g_rand_state == 0) {
        /* Auto-seed on first use so callers that forget the
         * explicit seed still get distinct values per process. */
        struct timespec ts;
        clock_gettime(CLOCK_MONOTONIC, &ts);
        g_rand_state = (uint64_t)ts.tv_sec * 1000000000ULL
                     + (uint64_t)ts.tv_nsec;
        if (g_rand_state == 0) g_rand_state = 0x9E3779B97F4A7C15ULL;
    }
    /* xorshift64* */
    uint64_t x = g_rand_state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    g_rand_state = x;
    uint64_t mixed = x * 0x2545F4914F6CDD1DULL;
    pthread_mutex_unlock(&g_rand_mutex);
    if (max <= 0) return 0;
    return (int64_t)(mixed % (uint64_t)max);
}

/*
 * C7 (pond follow-up): wall-clock seconds since the Unix epoch.
 * Backs `std::time::now() -> Int`. CLOCK_REALTIME is reserved for
 * observation only (NTP slewing / leap seconds can warp the
 * value); CLOCK_MONOTONIC stays the basis for scheduling. Returns
 * tv_sec verbatim — sub-second precision lives in the future
 * `std::time::now_ns` if a consumer surfaces it.
 */
int64_t lotus_time_now_seconds(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (int64_t)ts.tv_sec;
}

/*
 * Phase 2e: index-API surface over the existing
 * lotus_fs_list_dir_global() cache. Returning a real `[String]`
 * waits on dynamic-array codegen support; meanwhile the
 * count + at pair drops every list_dir caller's iteration loop
 * from the manual `index_of("\n") + slice + advance` pattern to
 * a clean while-loop bounded by count. Both walk the cached
 * newline-joined blob, so amortised cost is linear in total
 * bytes once across both calls (no re-stat per entry).
 *
 * Filenames with embedded `\n` are still ill-defined at this
 * substrate — same limitation as list_dir itself (POSIX permits
 * `\n` in path segments; v0 documents the limitation and chooses
 * the simpler newline-joined cache).
 */
int64_t lotus_fs_list_dir_count(const char *path) {
    if (!path) return 0;
    const char *blob = lotus_fs_list_dir_global(path);
    if (!blob || !*blob) return 0;
    /* The cache shape is `entry\nentry\n...\n`. Count the newlines;
     * the last entry always carries a trailing newline (see
     * lotus_fs_list_dir's emit loop). */
    int64_t n = 0;
    for (const char *p = blob; *p; p++) {
        if (*p == '\n') n++;
    }
    return n;
}

const char *lotus_fs_list_dir_at(const char *path, int64_t idx) {
    static const char empty[1] = { 0 };
    if (!path || idx < 0) return empty;
    const char *blob = lotus_fs_list_dir_global(path);
    if (!blob || !*blob) return empty;
    /* Walk to the start of the idx-th entry. */
    const char *p = blob;
    for (int64_t k = 0; k < idx; k++) {
        const char *nl = strchr(p, '\n');
        if (!nl) return empty;
        p = nl + 1;
        if (!*p) return empty;
    }
    /* p points at the start of the idx-th entry. Find its
     * terminating newline and copy the slice into the global
     * payload arena so the returned String outlives the call. */
    const char *end = strchr(p, '\n');
    if (!end) return empty;
    size_t len = (size_t)(end - p);
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return empty;
        }
    }
    char *out = (char *)lotus_arena_alloc(
        g_bus_payload_arena, len + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return empty;
    memcpy(out, p, len);
    out[len] = '\0';
    return out;
}

/*
 * C9 (pond/agent/sandbox): race-free tempfile-path allocator.
 * Assembles `prefix + "XXXXXX" + suffix` into a writable buffer,
 * calls mkstemps(3) to atomically open+create the file (mode
 * 0600) with the XXXXXX template substituted, immediately closes
 * the fd, and returns the resulting path string anchored in the
 * lazy global payload arena. NULL on error (errno set; EINVAL on
 * NULL args, ENOMEM on alloc failure, anything mkstemps can set
 * on its own — typically ENOENT/EACCES on the prefix dir).
 *
 * The caller owns cleanup — they wanted a path, not an fd —
 * matching the pond friction-log ask and the standard mktemp(3)
 * shape. There IS a window between our close() and the caller's
 * reopen (an attacker with write-access to the parent dir could
 * unlink + replace), but the pond contract is "race-free path
 * allocation" rather than "race-free path lifecycle" — that's
 * the standard mktemp shape and the friction-log ask explicitly
 * names this contract. Callers needing a held-open fd should
 * grow a sibling `mkstemp_fd` primitive later.
 */
const char *lotus_fs_mktemp(const char *prefix, const char *suffix) {
    if (!prefix || !suffix) {
        errno = EINVAL;
        return NULL;
    }
    size_t plen = strlen(prefix);
    size_t slen = strlen(suffix);
    /* prefix + "XXXXXX" + suffix + NUL */
    size_t total = plen + 6 + slen + 1;
    char *tmpl = (char *)malloc(total);
    if (!tmpl) {
        errno = ENOMEM;
        return NULL;
    }
    memcpy(tmpl, prefix, plen);
    memcpy(tmpl + plen, "XXXXXX", 6);
    memcpy(tmpl + plen + 6, suffix, slen);
    tmpl[total - 1] = '\0';
    int fd = mkstemps(tmpl, (int)slen);
    if (fd < 0) {
        int saved = errno;
        free(tmpl);
        errno = saved;
        return NULL;
    }
    close(fd);
    /* Anchor the assembled path in the bus payload arena so it
     * outlives this call frame, then drop the malloc buffer. */
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            free(tmpl);
            errno = ENOMEM;
            return NULL;
        }
    }
    char *out = lotus_str_clone(g_bus_payload_arena, tmpl);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    free(tmpl);
    if (!out) {
        errno = ENOMEM;
        return NULL;
    }
    return out;
}

void lotus_bus_remote_destroy_all(void) {
    for (size_t i = 0; i < g_bus_remote_count; i++) {
        lotus_bus_remote_entry_t *e = &g_bus_remote_entries[i];

        /* Wave B: adapter entries own their protocol lifecycle
         * through the adapter locus's own dissolve method, which
         * fires via the regular locus dissolve cascade at program
         * exit. No transport to destroy or reader thread to join
         * here — only the strdup'd subject string to free. */
        if (e->kind == LOTUS_BUS_REMOTE_KIND_ADAPTER) {
            if (e->subject) free(e->subject);
            continue;
        }

        /* m59: for LISTEN role, the reader thread owns the
         * transport's lifecycle. Best-effort shutdown(conn_fd)
         * to unblock recv if the peer hasn't closed yet, then
         * join. The thread destroys the transport itself before
         * exiting, so we don't double-destroy here. */
        if (e->has_reader_thread) {
            if (e->transport && e->transport->conn_fd >= 0) {
                /* SHUT_RDWR turns subsequent recvs into
                 * immediate EOF. Ignore errors — if the peer has
                 * already closed (the common case in a clean
                 * teardown), the fd may already be half-shut. */
                shutdown(e->transport->conn_fd, SHUT_RDWR);
            }
            pthread_join(e->reader_thread, NULL);
            /* Reader thread has already nulled e->transport on
             * its way out, but if it failed before storing
             * (transport_create returned NULL), the field is
             * already NULL — so the CONNECT-style destroy below
             * is a no-op for this slot. */
        }
        if (e->transport) {
            lotus_transport_destroy(e->transport);
        }
        if (e->subject) {
            free(e->subject);
        }
    }
    if (g_bus_remote_entries) free(g_bus_remote_entries);
    g_bus_remote_entries = NULL;
    g_bus_remote_count   = 0;
    g_bus_remote_cap     = 0;

    /* m70: tear down the lazy payload arena (used by deserialize
     * to allocate String byte storage that survives the reader-
     * thread → dispatch → handler chain). Created on first use
     * via lotus_bus_payload_arena_alloc; destroyed here at
     * program shutdown alongside the rest of the bus tables. */
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (g_bus_payload_arena) {
        lotus_arena_destroy(g_bus_payload_arena);
        g_bus_payload_arena = NULL;
    }
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
}

/*
 * v1.x: ASCII case folding. `lower(s)` / `upper(s)` allocate a
 * new NUL-terminated string in the bus payload arena (same
 * lifetime class as parse_int etc.) and copy the input byte-by-
 * byte with the standard ASCII case shift. Non-ASCII bytes pass
 * through unchanged (utf-8 case folding is intentionally NOT
 * attempted at v1 — locale-correct folding requires Unicode
 * tables far heavier than the runtime currently carries).
 */
const char *lotus_str_lower(const char *s) {
    if (!s) return "";
    size_t n = strlen(s);
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return "";
        }
    }
    char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, n + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return "";
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        out[i] = (c >= 'A' && c <= 'Z') ? (char)(c + 32) : (char)c;
    }
    out[n] = '\0';
    return out;
}

const char *lotus_str_trim(const char *s) {
    if (!s) return "";
    /* Whitespace per RFC 7230 / common usage: space, tab, \r, \n. */
    size_t n = strlen(s);
    size_t lo = 0;
    while (lo < n) {
        unsigned char c = (unsigned char)s[lo];
        if (c == ' ' || c == '\t' || c == '\r' || c == '\n') {
            lo++;
        } else {
            break;
        }
    }
    size_t hi = n;
    while (hi > lo) {
        unsigned char c = (unsigned char)s[hi - 1];
        if (c == ' ' || c == '\t' || c == '\r' || c == '\n') {
            hi--;
        } else {
            break;
        }
    }
    size_t out_len = hi - lo;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return "";
        }
    }
    char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, out_len + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return "";
    if (out_len > 0) {
        memcpy(out, s + lo, out_len);
    }
    out[out_len] = '\0';
    return out;
}

/*
 * 2026-05-17 — substring extraction. Returns s[lo..hi) clamped
 * to the input's byte range; negative lo / hi past the end /
 * inverted bounds all collapse to "". Operates on raw bytes
 * (same shape as bytes::slice + str::from_bytes composed), so
 * non-ASCII multi-byte sequences are split at byte boundaries
 * — slice high-byte ASCII / Bytes via std::bytes::slice if you
 * need codepoint discipline. Result lives in the global payload
 * arena.
 */
const char *lotus_str_substring(const char *s, int64_t lo, int64_t hi) {
    if (!s) return "";
    int64_t n = (int64_t)strlen(s);
    if (lo < 0) lo = 0;
    if (hi > n) hi = n;
    if (lo >= hi) return "";
    size_t out_len = (size_t)(hi - lo);
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return "";
        }
    }
    char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, out_len + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return "";
    memcpy(out, s + lo, out_len);
    out[out_len] = '\0';
    return out;
}

/*
 * Replace every occurrence of `needle` with `replacement` in `s`.
 * Naive O(n*m) scan. Empty needle returns `s` unchanged (replacing
 * "" infinitely is undefined). Overlap is greedy-forward — each
 * match advances by `needle_len`, not 1.
 */
const char *lotus_str_replace(const char *s, const char *needle,
                              const char *replacement) {
    if (!s) return "";
    if (!needle || !*needle) {
        /* No-op for empty needle. */
        size_t n = strlen(s);
        pthread_mutex_lock(&g_bus_payload_arena_mutex);
        if (!g_bus_payload_arena) {
            g_bus_payload_arena = lotus_arena_create();
            if (!g_bus_payload_arena) {
                pthread_mutex_unlock(&g_bus_payload_arena_mutex);
                return "";
            }
        }
        char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, n + 1, 1);
        pthread_mutex_unlock(&g_bus_payload_arena_mutex);
        if (!out) return "";
        memcpy(out, s, n);
        out[n] = '\0';
        return out;
    }
    if (!replacement) replacement = "";
    size_t s_len   = strlen(s);
    size_t need    = strlen(needle);
    size_t rep_len = strlen(replacement);

    /* Count occurrences first to right-size the output. */
    size_t count = 0;
    for (size_t i = 0; i + need <= s_len; ) {
        if (memcmp(s + i, needle, need) == 0) {
            count++;
            i += need;
        } else {
            i++;
        }
    }
    size_t out_len;
    if (rep_len >= need) {
        out_len = s_len + count * (rep_len - need);
    } else {
        out_len = s_len - count * (need - rep_len);
    }

    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return "";
        }
    }
    char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, out_len + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return "";

    size_t j = 0;
    for (size_t i = 0; i < s_len; ) {
        if (i + need <= s_len && memcmp(s + i, needle, need) == 0) {
            memcpy(out + j, replacement, rep_len);
            j += rep_len;
            i += need;
        } else {
            out[j++] = s[i++];
        }
    }
    out[out_len] = '\0';
    return out;
}

/*
 * Repeat `s` n times, concatenated. Negative or zero n returns
 * the empty string. NULL s is treated as "". Result is anchored
 * in the bus payload arena.
 */
const char *lotus_str_repeat(const char *s, int64_t n) {
    if (!s || n <= 0) {
        return "";
    }
    size_t sl = strlen(s);
    if (sl == 0) return "";
    size_t total = sl * (size_t)n;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return "";
        }
    }
    char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, total + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return "";
    for (int64_t i = 0; i < n; i++) {
        memcpy(out + i * sl, s, sl);
    }
    out[total] = '\0';
    return out;
}

/*
 * Pad `s` on the LEFT with `pad` until total length is `width`.
 * If `s` is already >= width, returns `s` unchanged (no truncation).
 * `pad` must be a single ASCII char (uses first byte). Common
 * shape for right-aligning numbers in column output.
 */
const char *lotus_str_pad_left(const char *s, int64_t width, const char *pad) {
    if (!s) s = "";
    size_t sl = strlen(s);
    if ((int64_t)sl >= width) {
        /* Already wide enough — return unchanged (arena-copy so the
         * caller doesn't need to distinguish own-vs-borrow). */
        size_t n = sl;
        pthread_mutex_lock(&g_bus_payload_arena_mutex);
        if (!g_bus_payload_arena) {
            g_bus_payload_arena = lotus_arena_create();
            if (!g_bus_payload_arena) {
                pthread_mutex_unlock(&g_bus_payload_arena_mutex);
                return "";
            }
        }
        char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, n + 1, 1);
        pthread_mutex_unlock(&g_bus_payload_arena_mutex);
        if (!out) return "";
        memcpy(out, s, n);
        out[n] = '\0';
        return out;
    }
    char ch = (pad && *pad) ? *pad : ' ';
    size_t pad_count = (size_t)width - sl;
    size_t total = (size_t)width;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return "";
        }
    }
    char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, total + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return "";
    memset(out, ch, pad_count);
    memcpy(out + pad_count, s, sl);
    out[total] = '\0';
    return out;
}

/*
 * Pad `s` on the RIGHT with `pad` until total length is `width`.
 * Same shape as pad_left but the pad bytes go on the trailing side.
 * Common for left-aligning columns in table output.
 */
const char *lotus_str_pad_right(const char *s, int64_t width, const char *pad) {
    if (!s) s = "";
    size_t sl = strlen(s);
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return "";
        }
    }
    size_t total = ((int64_t)sl >= width) ? sl : (size_t)width;
    char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, total + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return "";
    memcpy(out, s, sl);
    if (total > sl) {
        char ch = (pad && *pad) ? *pad : ' ';
        memset(out + sl, ch, total - sl);
    }
    out[total] = '\0';
    return out;
}

const char *lotus_str_upper(const char *s) {
    if (!s) return "";
    size_t n = strlen(s);
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return "";
        }
    }
    char *out = (char *)lotus_arena_alloc(g_bus_payload_arena, n + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) return "";
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        out[i] = (c >= 'a' && c <= 'z') ? (char)(c - 32) : (char)c;
    }
    out[n] = '\0';
    return out;
}

/*
 * v1.x-15: string-builder primitive. Resolves the reader-list_item-
 * quadratic-concat friction: long-running string accumulation can
 * now run in amortized O(N) total cost via doubling realloc, rather
 * than the O(N²) shape that `buf = buf + chunk` collapsed to under
 * Aperio's arena-anchored immutable Strings.
 *
 * The builder is a single contiguous malloc'd buffer with a
 * length and capacity. append() doubles cap as needed. finish()
 * allocates the final NUL-terminated string in the bus payload
 * arena (so it stays live for the rest of the program), copies
 * the buffer into it, frees the builder, and returns the string.
 *
 * Leaks the builder if finish() is never called — acceptable for
 * v1 since the surface fences this off: every builder_new()
 * dominates a builder_finish() in practice, and the worst-case
 * "user forgot to finish" is bounded by the working-set size of
 * one accumulator scope.
 */
typedef struct lotus_str_builder {
    size_t cap;
    size_t len;
    char  *buf;
} lotus_str_builder_t;

void *lotus_str_builder_new(void) {
    lotus_str_builder_t *b = (lotus_str_builder_t *)
        malloc(sizeof(lotus_str_builder_t));
    if (!b) return NULL;
    b->cap = 64;
    b->len = 0;
    b->buf = (char *)malloc(b->cap);
    if (!b->buf) {
        free(b);
        return NULL;
    }
    b->buf[0] = '\0';
    return b;
}

void lotus_str_builder_append(void *handle, const char *s) {
    if (!handle || !s) return;
    lotus_str_builder_t *b = (lotus_str_builder_t *)handle;
    size_t add = strlen(s);
    if (add == 0) return;
    size_t need = b->len + add;
    if (need + 1 > b->cap) {
        size_t new_cap = b->cap ? b->cap : 64;
        while (new_cap < need + 1) {
            new_cap *= 2;
            /* Guard against overflow at unreasonable sizes. */
            if (new_cap < b->cap) {
                /* Saturate: allocate exactly what we need. */
                new_cap = need + 1;
                break;
            }
        }
        char *nb = (char *)realloc(b->buf, new_cap);
        if (!nb) return;
        b->buf = nb;
        b->cap = new_cap;
    }
    memcpy(b->buf + b->len, s, add);
    b->len = need;
    b->buf[b->len] = '\0';
}

int64_t lotus_str_builder_len(const void *handle) {
    if (!handle) return 0;
    const lotus_str_builder_t *b = (const lotus_str_builder_t *)handle;
    return (int64_t)b->len;
}

const char *lotus_str_builder_finish(void *handle) {
    if (!handle) return "";
    lotus_str_builder_t *b = (lotus_str_builder_t *)handle;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            free(b->buf);
            free(b);
            return "";
        }
    }
    char *out = (char *)lotus_arena_alloc(
        g_bus_payload_arena, b->len + 1, 1);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!out) {
        free(b->buf);
        free(b);
        return "";
    }
    memcpy(out, b->buf, b->len);
    out[b->len] = '\0';
    free(b->buf);
    free(b);
    return out;
}

/*
 * C10 (pond follow-up): binary-safe builder mirroring
 * lotus_str_builder_* but using the Bytes ABI on both sides.
 *
 * The underlying buffer struct is identical to lotus_str_builder_t
 * (cap / len / buf) — append is just memcpy with no NUL discipline,
 * so the struct definition is shared. The split is in *what is
 * being appended* and *what finish returns*:
 *
 *   str_builder:   appends a C string (strlen the chunk),
 *                  finish returns a NUL-terminated String pointer.
 *   bytes_builder: appends a Bytes blob (reads `[i64 len]` prefix
 *                  so embedded NULs survive), finish returns a
 *                  freshly allocated `[i64 len][u8 data[len]]`
 *                  Bytes blob anchored in the bus payload arena.
 *
 * Pond consumers: pond/http/client + pond/agent/llm were
 * accumulating message bodies through std::str::builder_* +
 * std::bytes::from_string — lossy on chunks containing NUL.
 * std::bytes::builder_* is the single-step binary-safe path.
 */
typedef struct lotus_str_builder lotus_bytes_builder_t;

void *lotus_bytes_builder_new(void) {
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)
        malloc(sizeof(lotus_bytes_builder_t));
    if (!b) return NULL;
    b->cap = 64;
    b->len = 0;
    b->buf = (char *)malloc(b->cap);
    if (!b->buf) {
        free(b);
        return NULL;
    }
    /* No NUL seed — Bytes is length-prefixed, body is opaque
     * until callers fill it via append. */
    return b;
}

void lotus_bytes_builder_append(void *handle, const void *chunk_blob) {
    if (!handle || !chunk_blob) return;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    /* Bytes ABI: `[i64 len][u8 data[len]]`. Read the explicit
     * length prefix — strlen would truncate at the first NUL. */
    int64_t add_signed = lotus_bytes_len(chunk_blob);
    if (add_signed <= 0) return;
    size_t add = (size_t)add_signed;
    size_t need = b->len + add;
    if (need > b->cap) {
        size_t new_cap = b->cap ? b->cap : 64;
        while (new_cap < need) {
            new_cap *= 2;
            if (new_cap < b->cap) {
                /* Overflow guard — saturate at the exact need. */
                new_cap = need;
                break;
            }
        }
        char *nb = (char *)realloc(b->buf, new_cap);
        if (!nb) return;
        b->buf = nb;
        b->cap = new_cap;
    }
    /* Pull the body bytes out of the Bytes blob (past the
     * length prefix) and append verbatim — NULs included. */
    const char *src = (const char *)chunk_blob + sizeof(int64_t);
    memcpy(b->buf + b->len, src, add);
    b->len = need;
}

int64_t lotus_bytes_builder_len(const void *handle) {
    if (!handle) return 0;
    const lotus_bytes_builder_t *b = (const lotus_bytes_builder_t *)handle;
    return (int64_t)b->len;
}

void *lotus_bytes_builder_finish(void *handle) {
    if (!handle) return lotus_bytes_empty_global();
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            free(b->buf);
            free(b);
            return NULL;
        }
    }
    /* Emit a `[i64 len][u8 data[len]]` blob in the bus payload
     * arena. No trailing NUL — Bytes is length-prefixed. */
    void *blob = lotus_bytes_create(g_bus_payload_arena, (int64_t)b->len);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) {
        free(b->buf);
        free(b);
        return lotus_bytes_empty_global();
    }
    if (b->len > 0) {
        memcpy(lotus_bytes_data(blob), b->buf, b->len);
    }
    free(b->buf);
    free(b);
    return blob;
}

/*
 * Phase-0 in-place Bytes ops for long-lived recv-loop accumulators
 * (pond/websocket FRICTION § "per-frame Bytes allocations
 * accumulate"). The existing builder API was finish-once-and-done;
 * the WS recv loop needs a buffer that grows, drains from the
 * front, snapshots out a view, and disposes without producing a
 * final Bytes blob. These four ops extend the builder lifecycle:
 *
 *   builder_shift_front(b, n) — drop first n bytes via memmove,
 *                               capacity preserved.
 *   builder_clear(b)          — len=0, capacity preserved.
 *   builder_snapshot(b)       — copy current [0..len) into a
 *                               fresh Bytes blob in the bus
 *                               payload arena. Builder unchanged.
 *   builder_free(b)           — dispose the malloc-backed buffer
 *                               with no materialization. The
 *                               "leak unless finish" hazard the
 *                               old comment described is closed
 *                               for long-lived holders that
 *                               never call finish.
 */
void lotus_bytes_builder_shift_front(void *handle, int64_t n) {
    if (!handle) return;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    if (n <= 0) return;
    size_t drop = (size_t)n;
    if (drop >= b->len) {
        b->len = 0;
        return;
    }
    /* memmove handles overlap; src+drop > dst by construction. */
    memmove(b->buf, b->buf + drop, b->len - drop);
    b->len -= drop;
}

void lotus_bytes_builder_clear(void *handle) {
    if (!handle) return;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    b->len = 0;
}

void *lotus_bytes_builder_snapshot(void *handle) {
    if (!handle) return lotus_bytes_empty_global();
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_arena_create();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return lotus_bytes_empty_global();
        }
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, (int64_t)b->len);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    if (!blob) {
        return lotus_bytes_empty_global();
    }
    if (b->len > 0) {
        memcpy(lotus_bytes_data(blob), b->buf, b->len);
    }
    return blob;
}

void lotus_bytes_builder_free(void *handle) {
    if (!handle) return;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    free(b->buf);
    free(b);
}

/*
 * Phase 1: caller-provided destination at the syscall layer. The
 * `lotus_*_recv_bytes` shapes allocate a fresh `[i64 len][body]`
 * blob in g_bus_payload_arena per call — the leak source flagged
 * by pond/websocket's recv loop (~480 KB/s on a Kraken trade
 * feed). recv_into reads directly into the caller's builder
 * buffer. Grows the builder if its remaining headroom (cap - len)
 * is smaller than max_bytes; the builder's len is bumped by the
 * count read. Return semantics:
 *
 *   > 0  bytes appended to the builder
 *   = 0  peer closed cleanly (TCP) / zero-length datagram (UDP)
 *   < 0  fatal error; builder is unchanged
 *
 * Mirrors POSIX read(2) — partial reads are normal, the caller
 * loops or yields. EINTR retried internally. No allocation in
 * g_bus_payload_arena.
 *
 * The reserve / advance pair is exposed for lotus_tls.c (separate
 * translation unit) to implement lotus_tls_recv_into without
 * needing lotus_bytes_builder_t's layout.
 */
void *lotus_bytes_builder_reserve(void *handle, int64_t n) {
    if (!handle || n <= 0) return NULL;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    size_t need = b->len + (size_t)n;
    if (need > b->cap) {
        size_t new_cap = b->cap ? b->cap : 64;
        while (new_cap < need) {
            new_cap *= 2;
            if (new_cap < b->cap) {
                new_cap = need;
                break;
            }
        }
        char *nb = (char *)realloc(b->buf, new_cap);
        if (!nb) return NULL;
        b->buf = nb;
        b->cap = new_cap;
    }
    return b->buf + b->len;
}

void lotus_bytes_builder_advance(void *handle, int64_t n) {
    if (!handle || n <= 0) return;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    b->len += (size_t)n;
}

int64_t lotus_tcp_recv_into(int fd, void *builder, int64_t max_bytes) {
    if (fd < 0 || max_bytes <= 0) return -1;
    char *tail = (char *)lotus_bytes_builder_reserve(builder, max_bytes);
    if (!tail) return -1;
    ssize_t n;
    for (;;) {
        n = read(fd, tail, (size_t)max_bytes);
        if (n >= 0) break;
        if (errno == EINTR) continue;
        return -1;
    }
    lotus_bytes_builder_advance(builder, (int64_t)n);
    return (int64_t)n;
}

int64_t lotus_udp_recv_into(int fd, void *builder, int64_t max_bytes) {
    if (fd < 0 || max_bytes <= 0) return -1;
    char *tail = (char *)lotus_bytes_builder_reserve(builder, max_bytes);
    if (!tail) return -1;
    /* Datagram boundaries preserved: a single recvfrom delivers
     * at most one datagram. EINTR retried; other errors fatal. */
    ssize_t n;
    for (;;) {
        n = recvfrom(fd, tail, (size_t)max_bytes, 0, NULL, NULL);
        if (n >= 0) break;
        if (errno == EINTR) continue;
        return -1;
    }
    lotus_bytes_builder_advance(builder, (int64_t)n);
    return (int64_t)n;
}

/*
 * v1.x-FORM-2 PR6: root-locus value-error panic.
 *
 * Called by codegen when an `or raise` is reached past every
 * enclosing fallible(E) frame — i.e., the value error has
 * escaped the implicit main locus's body. Today: report to
 * stderr and exit(1), reusing the same shape the closure-
 * violation bare-handler fallback uses. Architecturally the
 * seat for a future routing-through-main-locus-on_failure
 * extension; the typename arg is the discriminator a future
 * dispatch would key on, and the payload ptr / size are
 * carried opaquely now so that extension doesn't need an ABI
 * bump.
 */
void lotus_root_panic(
    const void *payload,
    size_t payload_size,
    const char *payload_typename
) {
    (void)payload;
    (void)payload_size;
    const char *tn = payload_typename ? payload_typename : "<unknown>";
    dprintf(2, "Aperio panic: unhandled %s escaping main locus\n", tn);
    exit(1);
}

/*
 * C8 (pond follow-up): IEEE 754 sentinel / classification helpers.
 * Back `std::math::{nan, is_nan, inf}`. `std::math::tanh` does NOT
 * have a wrapper here — it resolves through a direct LLVM extern
 * (mirroring `sqrt` / `exp` / `log` / `floor` / `ceil` / `pow`) so
 * binaries that don't actually call `tanh` aren't burdened with
 * an unresolved libm reference (test helper binaries — bus_config,
 * transport, etc. — link `lotus_arena.c` without `-lm`, so any
 * libm symbol referenced from this file at compile time becomes
 * an unconditional load-bearing dependency).
 *
 * `nan` / `inf` / `is_nan` are SAFE here: they reference only the
 * `<math.h>` macros `NAN` / `INFINITY` (compile-time constants)
 * and the canonical `f != f` test, none of which touch libm at
 * link time.
 *
 * NaN-printing is platform-dependent (`nan` / `NaN` / `-nan` via
 * printf %g); agents test for NaN via `is_nan(x)`, not by
 * comparing the printed value. Driven by pond/ml/neural
 * (hand-rolled tanh from exp) and pond/math/matrix (synthesizes
 * `nan_sentinel()` as `0.0/0.0` and `is_nan(f)` as `f != f`).
 */
double lotus_math_nan(void) {
    return (double)NAN;
}

double lotus_math_inf(void) {
    return (double)INFINITY;
}

/* Canonical IEEE 754 NaN test: a quiet NaN is the only value
 * that is not equal to itself. Returns 1 if `f` is NaN, 0
 * otherwise. Lowers as i1 on the LLVM side via the truncation
 * pattern lotus_fs_file_exists uses for its 0/1 -> Bool. */
int lotus_math_is_nan(double f) {
    return f != f ? 1 : 0;
}
