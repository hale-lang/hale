/* GenMC model of the lotus pinned-locus mailbox.
 * GH issue #18 item 2 (race-completeness).
 *
 * FAITHFUL TRANSCRIPTION of the monitor in `lotus_mailbox_*` in
 * crates/hale-codegen/runtime/lotus_arena.c — the mutex-protected
 * post/drain/shutdown handoff. It is NOT the production code:
 *   - the payload is reduced to an int (the two-tier inline/heap
 *     storage is allocation plumbing, not the concurrency surface);
 *   - the compact/realloc grow branch is elided by keeping posts <=
 *     CAP (grow is a single-threaded resize under the same lock, a
 *     separate concern from the monitor handoff);
 *   - the `pthread_cond_wait`/`broadcast` is modeled as a lock-guarded
 *     SPIN preserving the exact `while (empty && !shutdown)` predicate.
 *     GenMC does not model condition variables (they're a liveness
 *     mechanism); the mailbox's SAFETY — mutual exclusion on
 *     head/tail/cells, no lost/duplicated cell, no use-after-free — is
 *     independent of whether the drainer sleeps or spins, and that's
 *     what this model checks. (Missed-wakeup liveness would need
 *     condvar modeling and is out of scope here.)
 * What's kept exactly: every lock/unlock boundary, the wait predicate,
 * and the "drain pending even after shutdown" exit condition. If those
 * change in the C, this model must change with them.
 *
 * Properties GenMC checks across every interleaving:
 *   - no data race on head/tail/cells (the mutex must cover them),
 *   - no lost or duplicated cell (every posted value drained once),
 *   - the drainer terminates once shutdown + empty,
 *   - no assertion failure (invariants in the harness).
 *
 * Run:  genmc -- verification/mailbox_model.c   (or run_genmc.sh)
 */

#include <pthread.h>
#include <stdlib.h>
#include <assert.h>

#define CAP 4   /* > number of posts, so the grow branch never fires */

typedef struct {
    int             cells[CAP];   /* payload reduced to an int value */
    size_t          head;
    size_t          tail;
    int             shutdown;
    pthread_mutex_t lock;
} mailbox_t;

static void mb_post(mailbox_t *mb, int val) {
    pthread_mutex_lock(&mb->lock);
    /* Production compacts/reallocs here when tail == cap; the harness
     * keeps posts <= CAP so that branch is unreachable. Guard it. */
    assert(mb->tail < CAP);
    mb->cells[mb->tail++] = val;
    /* (production broadcasts the not_empty condvar here) */
    pthread_mutex_unlock(&mb->lock);
}

/* Returns 1 and writes the dequeued value to *out; or 0 on
 * shutdown-with-empty. Mirrors lotus_mailbox_drain_one, with the
 * cond_wait modeled as a lock-guarded spin on the same predicate. */
static int mb_drain_one(mailbox_t *mb, int *out) {
    for (;;) {
        pthread_mutex_lock(&mb->lock);
        if (mb->head < mb->tail) {             /* a cell is waiting */
            int val = mb->cells[mb->head++];   /* drains even if shutdown */
            if (mb->head >= mb->tail) {
                mb->head = 0;
                mb->tail = 0;
            }
            pthread_mutex_unlock(&mb->lock);
            /* Production hands the cell to the handler here, outside
             * the lock. */
            *out = val;
            return 1;
        }
        if (mb->shutdown) {                    /* shutdown, empty */
            mb->head = 0;
            mb->tail = 0;
            pthread_mutex_unlock(&mb->lock);
            return 0;
        }
        pthread_mutex_unlock(&mb->lock);       /* empty: wait (spin) */
    }
}

static void mb_shutdown(mailbox_t *mb) {
    pthread_mutex_lock(&mb->lock);
    mb->shutdown = 1;
    /* (production broadcasts the not_empty condvar here) */
    pthread_mutex_unlock(&mb->lock);
}

/* --- harness ------------------------------------------------------- */
/* Two posters drop one distinct value each; one drainer (the pinned
 * thread) loops draining until shutdown-with-empty, tallying what it
 * received. After the posts are in, main shuts the mailbox down. The
 * drainer must receive both values exactly once and then terminate —
 * across every interleaving of post / drain / shutdown. */

static mailbox_t MB;
static int g_seen[2];      /* g_seen[v] counts how often value v drained */
static int g_count;        /* total cells drained */

static void *poster_a(void *_) { (void)_; mb_post(&MB, 0); return NULL; }
static void *poster_b(void *_) { (void)_; mb_post(&MB, 1); return NULL; }

static void *drainer(void *_) {
    (void)_;
    int v;
    while (mb_drain_one(&MB, &v)) {
        assert(v == 0 || v == 1);
        g_seen[v]++;
        g_count++;
    }
    return NULL;
}

int main(void) {
    MB.head = 0;
    MB.tail = 0;
    MB.shutdown = 0;
    g_seen[0] = 0;
    g_seen[1] = 0;
    g_count = 0;
    pthread_mutex_init(&MB.lock, NULL);

    pthread_t pa, pb, dr;
    pthread_create(&dr, NULL, drainer, NULL);
    pthread_create(&pa, NULL, poster_a, NULL);
    pthread_create(&pb, NULL, poster_b, NULL);

    /* Both posts are committed once the poster threads join; only then
     * do we signal shutdown, so the drain set is exactly {0, 1}. */
    pthread_join(pa, NULL);
    pthread_join(pb, NULL);
    mb_shutdown(&MB);
    pthread_join(dr, NULL);

    /* Every posted cell drained exactly once; the drainer terminated. */
    assert(g_count == 2);
    assert(g_seen[0] == 1);
    assert(g_seen[1] == 1);
    return 0;
}
