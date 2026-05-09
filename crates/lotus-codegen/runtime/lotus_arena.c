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
 * Future strategies (chunked sub-regions per F.3, recognition
 * pools) will land as additional functions, not edits to this
 * one.
 *
 * Backed by libc malloc for the chunks themselves. That's not a
 * cheat — the substrate's job is wholesale-region management;
 * the underlying *page* supplier can be libc, mmap, or a
 * pre-reserved pool, and the arena interface above doesn't
 * change. Replace this file's malloc/free with mmap when the
 * scheduler lands and we want page-aligned regions.
 */

#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>

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

/* Public ABI ---------------------------------------------------- */

lotus_arena_t *lotus_arena_create(void) {
    lotus_arena_t *a = (lotus_arena_t *)malloc(sizeof(lotus_arena_t));
    if (!a) return NULL;
    a->default_chunk_size = LOTUS_ARENA_CHUNK_BYTES;
    a->head = lotus_arena_new_chunk(a->default_chunk_size);
    if (!a->head) {
        free(a);
        return NULL;
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
    lotus_arena_chunk_t *c = a->head;
    while (c) {
        lotus_arena_chunk_t *next = c->next;
        free(c);
        c = next;
    }
    free(a);
}
