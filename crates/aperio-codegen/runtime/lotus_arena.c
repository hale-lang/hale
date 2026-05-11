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
#include <unistd.h>
#include <errno.h>
#include <time.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <dirent.h>

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
     * the existing lotus_bus_remote_fanout iteration. When the
     * remote-entries table is empty for this subject (config-
     * not-set or all entries are LISTEN-role) the fanout is a
     * cheap loop. The serializer cost runs unconditionally
     * when serialize_fn is non-NULL — local-only programs that
     * never set LOTUS_BUS_CONFIG pay one extra serialize per
     * publish, which is bounded (LOTUS_PAYLOAD_MAX bytes). A
     * future polish could gate this behind a "any remote
     * entry for subject" check via a new helper, but the
     * minimal-coupling shape is preferable for v1.
     *
     * m58: local + remote share the same subject namespace per
     * notes/open-questions #9 (emergent cardinality). */
    if (!serialize_fn) return;
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
     * shape (~1s window). */
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    const char *h = host ? host : "127.0.0.1";
    if (inet_pton(AF_INET, h, &addr.sin_addr) != 1) {
        fprintf(stderr, "lotus_tcp_connect: invalid host %s\n", h);
        errno = EINVAL;
        return -1;
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

typedef struct lotus_bus_remote_entry {
    char              *subject;       /* owned (strdup'd at register) */
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
} lotus_bus_remote_entry_t;

static lotus_bus_remote_entry_t *g_bus_remote_entries = NULL;
static size_t g_bus_remote_count = 0;
static size_t g_bus_remote_cap   = 0;

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
    /* v0.1 only handles unix:// URLs; future schemes (tcp://, etc.)
     * grow into this dispatch. Reject unknown schemes so the user
     * sees a clear error rather than a confusing transport-create
     * failure later. */
    static const char unix_scheme[] = "unix://";
    size_t scheme_len = sizeof(unix_scheme) - 1;
    if (strncmp(url, unix_scheme, scheme_len) != 0) {
        fprintf(stderr,
                "lotus_bus_register_remote: unsupported URL scheme "
                "(only unix:// in m58): %s\n",
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
    e->transport         = NULL;
    e->role              = role;
    e->has_reader_thread = 0;

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
        if (!e->subject || !e->transport) continue;
        if (strcmp(e->subject, subject) != 0) continue;
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

void lotus_bus_remote_destroy_all(void) {
    for (size_t i = 0; i < g_bus_remote_count; i++) {
        lotus_bus_remote_entry_t *e = &g_bus_remote_entries[i];

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
