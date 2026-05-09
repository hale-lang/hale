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
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <sched.h>

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
    pthread_mutex_lock(&q->lock);
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
                pthread_mutex_unlock(&q->lock);
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
    pthread_mutex_unlock(&q->lock);
}

/* Drain the queue: pop cells one at a time, copy each cell's
 * inline payload into the subscriber's arena (located at
 * self_ptr+0 by the universal __arena offset), and invoke
 * handler(self, arena_copy). Handlers may enqueue more cells
 * (cooperative-cooperative bus dispatch is the natural
 * interleaving — see The Design / lotus, substrate cells).
 * Loops until the queue is empty AT POP TIME, including any
 * cells enqueued during the drain itself.
 *
 * Lock discipline: take the mutex to pop one cell + read its
 * fields into a local; release before allocating in the
 * subscriber's arena and invoking the handler. Holding the
 * lock across handler invocation would (a) block pinned
 * producers for the entire handler runtime and (b) deadlock
 * if the handler re-enqueues. */
typedef void (*lotus_handler_fn)(void *self, void *payload);

void lotus_bus_queue_drain(lotus_bus_queue_t *q) {
    if (!q) return;
    for (;;) {
        pthread_mutex_lock(&q->lock);
        if (q->head >= q->tail) {
            /* Reset indices so the next batch starts fresh. */
            q->head = 0;
            q->tail = 0;
            pthread_mutex_unlock(&q->lock);
            return;
        }
        lotus_bus_cell_t cell_copy = q->cells[q->head++];
        pthread_mutex_unlock(&q->lock);

        /* Copy inline payload into the subscriber's arena.
         * The arena pointer lives at self_ptr+0 by convention
         * (every locus struct has __arena as its first field). */
        void *payload_in_arena = NULL;
        if (cell_copy.payload_size > 0) {
            lotus_arena_t *sub_arena =
                *(lotus_arena_t **)cell_copy.self_ptr;
            payload_in_arena = lotus_arena_alloc(
                sub_arena, cell_copy.payload_size, 8);
            if (payload_in_arena) {
                memcpy(payload_in_arena,
                       cell_copy.payload_inline,
                       cell_copy.payload_size);
            }
        }
        ((lotus_handler_fn)cell_copy.handler)(
            cell_copy.self_ptr, payload_in_arena);
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

    void *payload_in_arena = NULL;
    if (cell_copy.payload_size > 0) {
        lotus_arena_t *sub_arena =
            *(lotus_arena_t **)cell_copy.self_ptr;
        payload_in_arena = lotus_arena_alloc(
            sub_arena, cell_copy.payload_size, 8);
        if (payload_in_arena) {
            memcpy(payload_in_arena,
                   cell_copy.payload_inline,
                   cell_copy.payload_size);
        }
    }
    ((lotus_handler_fn)cell_copy.handler)(
        cell_copy.self_ptr, payload_in_arena);
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
