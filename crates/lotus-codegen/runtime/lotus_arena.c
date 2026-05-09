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

#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

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
 * Cooperative scheduler — bus dispatch queue (m26).
 *
 * Per The Design / lotus, every bus dispatch is a substrate
 * cell. The cooperative scheduler enqueues these cells at
 * publish time and pops them one at a time at drain time.
 * Each pop runs the handler to completion (handler-atomic;
 * cooperative yields BETWEEN cells, not within). Handlers may
 * publish more events, which enqueue more cells; drain
 * continues until the queue is empty.
 *
 * v0 is single-threaded — one queue, one drain loop, runs at
 * end of main and at strategic scope-exit points. m27 will
 * spawn dedicated threads for pinned-class loci, with
 * cross-thread mailbox post for any-class → pinned dispatch.
 *
 * The queue stores (handler, self, payload) triples. The
 * payload is already in the subscriber's arena (memcpy'd at
 * enqueue time, per spec/memory.md "A typed message crossing
 * a locus boundary is a copy, not a pointer."). At drain
 * time we just call handler(self, payload).
 */

typedef struct lotus_bus_cell {
    void *handler;          /* void (*)(void *self, void *payload) */
    void *self_ptr;         /* subscriber's locus ptr */
    void *payload;          /* copy in subscriber's arena */
} lotus_bus_cell_t;

typedef struct lotus_bus_queue {
    lotus_bus_cell_t *cells;
    size_t            head;     /* next slot to pop */
    size_t            tail;     /* next slot to fill */
    size_t            cap;
} lotus_bus_queue_t;

#define LOTUS_BUS_QUEUE_INITIAL_CAP 64

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
    return q;
}

/* Enqueue (handler, self, payload). Grows the cell array
 * geometrically when full; head/tail are monotonic indices
 * (compacted on grow). v0 keeps it simple — no ring buffer,
 * no power-of-two mask, just a linear array with grow-on-full
 * semantics. Trellis-grade workloads typically have queues of
 * a few hundred cells max; the full memcpy on grow is fine. */
void lotus_bus_queue_enqueue(lotus_bus_queue_t *q,
                             void *handler,
                             void *self_ptr,
                             void *payload) {
    if (!q) return;
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
            if (!new_cells) return;     /* drop on OOM */
            q->cells = new_cells;
            q->cap   = new_cap;
        }
    }
    q->cells[q->tail].handler  = handler;
    q->cells[q->tail].self_ptr = self_ptr;
    q->cells[q->tail].payload  = payload;
    q->tail++;
}

/* Drain the queue: pop cells one at a time and invoke
 * `handler(self, payload)`. Handlers may enqueue more cells
 * (cooperative-cooperative bus dispatch is the natural
 * interleaving — see The Design / lotus, substrate cells).
 * Loops until the queue is empty AT POP TIME, including any
 * cells enqueued during the drain itself. */
typedef void (*lotus_handler_fn)(void *self, void *payload);

void lotus_bus_queue_drain(lotus_bus_queue_t *q) {
    if (!q) return;
    while (q->head < q->tail) {
        lotus_bus_cell_t cell = q->cells[q->head++];
        ((lotus_handler_fn)cell.handler)(cell.self_ptr, cell.payload);
    }
    /* Reset indices so subsequent enqueues start fresh — this
     * is functionally optional (tail can keep growing), but
     * makes peek/inspect easier and keeps the working-set
     * memory smaller across long-running drains. */
    q->head = 0;
    q->tail = 0;
}

void lotus_bus_queue_destroy(lotus_bus_queue_t *q) {
    if (!q) return;
    if (q->cells) free(q->cells);
    free(q);
}

/*
 * Pinned-thread entry (m28a).
 *
 * The C-runtime adapter `lotus_thread_entry` is gone — m28a
 * synthesizes a per-locus `__pinned_main_<LocusName>` LLVM
 * function whose signature is exactly pthread's `void *(*)(void *)`.
 * That function takes self_ptr as its sole argument and runs
 * birth → run → drain → dissolve in sequence (each only if the
 * locus declared it) before returning NULL. Codegen passes that
 * function directly to pthread_create, with self_ptr as the arg.
 *
 * Bus subscribe / publish on pinned loci still wait on m28b
 * (cross-thread mailbox).
 */
