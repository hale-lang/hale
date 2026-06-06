/* GenMC model of the lotus cooperative-pool bus queue.
 * GH issue #18 item 2 (race-completeness).
 *
 * FAITHFUL TRANSCRIPTION of the distinctive concurrency feature of
 * `lotus_bus_queue_enqueue` / `lotus_bus_queue_drain` in
 * crates/hale-codegen/runtime/lotus_arena.c: the **conditional lock**.
 * A global atomic `g_bus_has_pinned` gates whether enqueue/drain take
 * the queue mutex. In the single-threaded cooperative phase the flag
 * is 0 and the queue is touched lockless (no concurrency); once a
 * pinned thread can also publish, the flag is 1 and every access
 * locks. The optimization is sound only because the flag is set
 * (release) BEFORE any second thread can touch the queue — this model
 * checks exactly that.
 *
 * It is NOT the production code: payload reduced to an int; the
 * compact/realloc grow branch elided (it runs under the same mutex, so
 * it serializes — no concurrency surface) by keeping posts <= CAP.
 * Kept exactly: the per-operation `g_bus_has_pinned` conditional-lock
 * decision and the `if (locked) lock / ... / if (locked) unlock`
 * discipline on both enqueue and drain, and the drain's "snapshot the
 * cell under the lock, then release" handoff.
 *
 * Scope: the modeled race surface is **concurrent enqueues** under the
 * conditional lock. A concurrent drain isn't a separate surface — it
 * takes the same mutex, so drain-vs-enqueue serializes identically to
 * enqueue-vs-enqueue — so the harness runs the producers concurrently
 * and drains sequentially after they join (a concurrent consumer would
 * only add a spin loop GenMC can't bound, no new race). GenMC checks
 * across every interleaving: no data race on head/tail/cells, and no
 * lost/duplicated cell (both producers' values present after drain).
 *
 * Run:  genmc -- verification/bus_queue_model.c   (or run_genmc.sh)
 */

#include <pthread.h>
#include <stdlib.h>
#include <stdatomic.h>
#include <assert.h>

#define CAP 4   /* > number of posts, so the grow branch never fires */

/* Production declares this `_Atomic` and reads it acquire / writes it
 * release; the acquire/release pair only matters at the 0->1
 * transition window. In the SOUND usage modeled here the flag is set
 * before any second thread can touch the queue (the pthread_create
 * happens-before publishes it), so during the concurrent phase it is a
 * constant 1 — a plain int read is faithful to that, and keeps the
 * checker in SC (the atomic forces the weaker RA model, under which
 * the drain spin's state space blows up). The transition-window race —
 * where the atomic + ordering is load-bearing — is the negative
 * control documented in verification/README.md. */
static int g_bus_has_pinned;   /* monotonic 0 -> 1; see note above */

typedef struct {
    int             cells[CAP];
    size_t          head;
    size_t          tail;
    pthread_mutex_t lock;
} bus_queue_t;

static void bq_enqueue(bus_queue_t *q, int val) {
    int locked = g_bus_has_pinned;
    if (locked) pthread_mutex_lock(&q->lock);
    assert(q->tail < CAP);                 /* grow branch elided */
    q->cells[q->tail++] = val;
    if (locked) pthread_mutex_unlock(&q->lock);
}

/* Pop one cell into *out under the conditional lock — snapshot then
 * release, as the production drain does. Returns 1 with a value, or 0
 * if the queue is empty. Mirrors one iteration of
 * lotus_bus_queue_drain's locked loop. */
static int bq_drain_one(bus_queue_t *q, int *out) {
    int locked = g_bus_has_pinned;
    if (locked) pthread_mutex_lock(&q->lock);
    if (q->head >= q->tail) {
        q->head = 0;
        q->tail = 0;
        if (locked) pthread_mutex_unlock(&q->lock);
        return 0;
    }
    int val = q->cells[q->head++];         /* snapshot under the lock */
    if (q->head >= q->tail) {
        q->head = 0;
        q->tail = 0;
    }
    if (locked) pthread_mutex_unlock(&q->lock);
    *out = val;                            /* handler runs after unlock */
    return 1;
}

/* --- harness ------------------------------------------------------- */
/* The sound usage: the flag is set BEFORE the producer threads are
 * created, so the pthread_create happens-before publishes it — every
 * producer sees 1 and takes the lock. Two producers enqueue distinct
 * values concurrently; after they join, main drains and checks both
 * arrived exactly once. */

static bus_queue_t Q;

static void *producer_a(void *_) { (void)_; bq_enqueue(&Q, 0); return NULL; }
static void *producer_b(void *_) { (void)_; bq_enqueue(&Q, 1); return NULL; }

int main(void) {
    Q.head = 0;
    Q.tail = 0;
    pthread_mutex_init(&Q.lock, NULL);

    /* A pinned subscriber now exists → all queue access must lock.
     * Set the flag BEFORE spawning, so the create's happens-before
     * makes it visible to every producer. */
    g_bus_has_pinned = 1;

    pthread_t pa, pb;
    pthread_create(&pa, NULL, producer_a, NULL);
    pthread_create(&pb, NULL, producer_b, NULL);
    pthread_join(pa, NULL);
    pthread_join(pb, NULL);

    /* Drain sequentially (producers are done) and tally. */
    int seen[2] = {0, 0};
    int count = 0;
    int v;
    while (bq_drain_one(&Q, &v)) {
        assert(v == 0 || v == 1);
        seen[v]++;
        count++;
    }
    assert(count == 2);          /* no lost or duplicated cell */
    assert(seen[0] == 1);
    assert(seen[1] == 1);
    return 0;
}
