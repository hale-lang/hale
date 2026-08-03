/* GenMC model of the bus queue's GROW-vs-DRAIN surface.
 * GH issue #18 item 2 (race-completeness). Added 2026-08-02.
 *
 * ADDITIVE to bus_queue_model.c, which models the conditional-lock
 * discipline and says of this branch:
 *
 *     "the compact/realloc grow branch elided (it runs under the same
 *      mutex, so it serializes — no concurrency surface)"
 *
 * That reasoning holds only while every reader touches `cells` under
 * the lock. It no longer does. `lotus_bus_queue_drain` snapshots the
 * cell and RELEASES THE LOCK before using it:
 *
 *     lotus_bus_cell_t cell_copy = q->cells[q->head++];
 *     pthread_mutex_unlock(&q->lock);
 *     ... handler runs off the snapshot ...
 *
 * and production says why:
 *
 *     "must snapshot each cell under the lock so the cells array can't
 *      be realloc'd out from under the in-flight pop"
 *
 * So the grow branch IS a concurrency surface: a producer's realloc
 * frees the old array while a consumer is between its unlock and its
 * use of the popped cell. The snapshot is the thing that makes that
 * safe — and the old model elides exactly the hazard the snapshot
 * defends against, so it verifies the handoff with nothing to defend
 * against. This model supplies the hazard.
 *
 * FAITHFUL: the conditional lock, the pop-snapshot-under-lock handoff,
 * the grow that reallocates and frees the previous array while other
 * threads hold no lock, and the invariant that growth only ever
 * happens with the lock held.
 *
 * NOT the production code: payload is an int; `realloc` is modeled as
 * malloc+copy+free (same free-of-the-old-block hazard, and GenMC
 * tracks the allocation); the bound/shed policy and the inline-drain
 * batch are separate surfaces (see bus_shed_model.c); CAP starts at 1
 * so a single extra post triggers the grow.
 *
 * NEGATIVE CONTROL — a model that cannot fail proves nothing:
 *
 *     genmc -- -DMODEL_BUG_INDEX_AFTER_UNLOCK verification/bus_grow_model.c
 *
 * makes the consumer keep an INDEX and re-read `q->cells[i]` after
 * releasing the lock, instead of snapshotting the value under it.
 * GenMC reports `Error: Non-atomic race!` on `q.cells` within the
 * first executions — the consumer reads the array POINTER unsynchro-
 * nized while a concurrent grow publishes the new one. That race is
 * the precursor to the use-after-free (the old block is freed on the
 * next line of the grow), and it is what the checker catches first.
 *
 * Note the define goes AFTER `--`: everything past it is passed to
 * clang, not to genmc.
 *
 * Run:  genmc -- verification/bus_grow_model.c   (or run_genmc.sh)
 */

#include <pthread.h>
#include <stdlib.h>
#include <assert.h>

#define POSTS 2   /* CAP starts at 1, so post #2 grows */

typedef struct {
    int            *cells;   /* heap array; REALLOCATED on grow */
    unsigned        head;
    unsigned        tail;
    unsigned        cap;
    pthread_mutex_t lock;
} queue_t;

static queue_t q;

/* Production reads `g_bus_has_pinned` per operation; in the modeled
 * (sound) usage it is a constant 1 during the concurrent phase — the
 * 0->1 transition window is bus_queue_model.c's subject, not this
 * one's. Kept as a plain int for the same reason given there: it keeps
 * the checker in SC. */
static const int locked = 1;

/* enqueue: grow under the lock, then place. Mirrors the
 * `while (q->tail == q->cap) { ... realloc ... }` block of
 * bus_queue_enqueue_inner. */
static void enqueue(int v)
{
    if (locked) pthread_mutex_lock(&q.lock);

    while (q.tail == q.cap) {
        if (q.head > 0) {                      /* compact first */
            unsigned live = q.tail - q.head;
            for (unsigned i = 0; i < live; i++)
                q.cells[i] = q.cells[q.head + i];
            q.head = 0;
            q.tail = live;
        }
        if (q.tail < q.cap) break;             /* compaction freed a slot */

        /* Grow. The old block is FREED here while consumers hold no
         * lock — safe only because a consumer never retains a pointer
         * into it across its unlock. */
        unsigned new_cap = q.cap * 2;
        int *fresh = malloc(new_cap * sizeof(int));
        if (!fresh) {                          /* OOM path: unlock + drop */
            if (locked) pthread_mutex_unlock(&q.lock);
            return;
        }
        for (unsigned i = 0; i < q.tail; i++)
            fresh[i] = q.cells[i];
        free(q.cells);
        q.cells = fresh;
        q.cap   = new_cap;
        break;
    }

    q.cells[q.tail++] = v;
    if (locked) pthread_mutex_unlock(&q.lock);
}

/* drain one: snapshot under the lock, use after releasing it. */
static int drain_one(void)
{
    if (locked) pthread_mutex_lock(&q.lock);
    if (q.head >= q.tail) {
        if (locked) pthread_mutex_unlock(&q.lock);
        return -1;
    }
#ifdef MODEL_BUG_INDEX_AFTER_UNLOCK
    /* The bug the snapshot exists to prevent: keep an INDEX, release
     * the lock, then dereference `cells` — which a concurrent grow may
     * already have freed. */
    unsigned i = q.head++;
    if (locked) pthread_mutex_unlock(&q.lock);
    int v = q.cells[i];
#else
    int v = q.cells[q.head++];   /* snapshot the VALUE under the lock */
    if (locked) pthread_mutex_unlock(&q.lock);
#endif
    /* The handler would run here, off the snapshot, with no lock
     * held — which is the whole point of copying. */
    return v;
}

static void *producer(void *arg)
{
    (void)arg;
    for (int i = 0; i < POSTS; i++) enqueue(i + 1);
    return NULL;
}

static void *consumer(void *arg)
{
    (void)arg;
    /* Bounded attempts, not a spin-to-completion: GenMC explores every
     * interleaving, so a bounded consumer already covers "pops nothing",
     * "pops mid-grow", and "pops after both posts". An unbounded spin
     * would not terminate under exploration. */
    for (int i = 0; i < POSTS; i++) {
        int v = drain_one();
        /* Anything popped must be a value some producer actually
         * posted — never freed memory, never a torn slot. */
        assert(v == -1 || (v >= 1 && v <= POSTS));
    }
    return NULL;
}

int main(void)
{
    pthread_t p, c;

    q.cap   = 1;
    q.cells = malloc(q.cap * sizeof(int));
    q.head  = 0;
    q.tail  = 0;
    pthread_mutex_init(&q.lock, NULL);

    pthread_create(&p, NULL, producer, NULL);
    pthread_create(&c, NULL, consumer, NULL);
    pthread_join(p, NULL);
    pthread_join(c, NULL);

    /* Whatever the consumer left behind must still be intact and
     * within the (possibly grown) array. */
    assert(q.head <= q.tail);
    assert(q.tail <= q.cap);

    free(q.cells);
    pthread_mutex_destroy(&q.lock);
    return 0;
}
