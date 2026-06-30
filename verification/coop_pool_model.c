/* GenMC model of the lotus COOPERATIVE POOL queue — Phase-1b lock-free
 * conversion. GH issue #18 item 2 (race-completeness).
 *
 * FAITHFUL TRANSCRIPTION of the cooperative pool's CLASSIC consumer path in
 * `lotus_coop_pool_*` / `lotus_mpsc_ring_*` in
 * crates/hale-codegen/runtime/lotus_arena.c after Phase-1b swapped the
 * cell-buffer-under-mutex queue for the Vyukov bounded MPSC ring + the
 * signal-only-when-parked wake handshake.
 *
 * The classic coop-pool ring + wake handshake is BYTE-IDENTICAL to the
 * pinned mailbox's (same `lotus_mpsc_ring_t`, same `_Atomic int parked`,
 * same seq_cst fences in post/drain_one). This model is the coop-pool
 * twin of `mailbox_model.c`; if either diverges from the C, both models
 * must change with it. The ASYNC coop path is NOT modeled here: it keeps
 * epoll_wait as its park and the eventfd as its wake, which is level-
 * triggered and therefore missed-wakeup-safe by construction (a durable
 * counter, not a custom handshake) — there is no bespoke memory-ordering
 * protocol on that path to verify.
 *
 * It is NOT the production code:
 *   - the payload is reduced to an int (the two-tier inline/heap storage
 *     is allocation plumbing, not the concurrency surface);
 *   - the full-ring / self-publish-overflow branch is elided by keeping
 *     posts <= CAP (the cross-thread backpressure path is the same Dekker
 *     handshake the mailbox uses and is validated under TSAN stress
 *     instead — GenMC's job here is the lock-free handoff + the missed
 *     wakeup);
 *   - the condvar is NOT modeled (GenMC has no condvars; they are a
 *     liveness mechanism). The PARK is modeled by its safety core: the
 *     `parked` seq_cst flag + a seq_cst fence + the drainer's recheck, and
 *     the producer's publish + seq_cst fence + load(parked). This is the
 *     Dekker / store-buffer (SB) shape the real wake handshake reduces to.
 *     A `g_signaled` flag stands in for "the producer would cond_signal".
 *
 * Properties GenMC checks across every interleaving:
 *   (1) no message lost           — every posted value drained once,
 *   (2) no message delivered twice — g_seen[v] == 1,
 *   (3) no spurious / torn value  — drained v is a value that was posted,
 *   (4) NO MISSED WAKEUP          — if the drainer commits to parking (its
 *       recheck saw the ring empty), the producer MUST have observed
 *       parked==1 (so it would signal). Dropping either seq_cst fence
 *       admits the SB outcome where the drainer parks AND the producer
 *       skips the signal — a hung pool worker. A negative control (build
 *       with -DBREAK_HANDSHAKE, replacing one seq_cst fence with a release
 *       fence) trips assertion (4), confirming the model has teeth.
 *
 * Run:  genmc -- verification/coop_pool_model.c   (or run_genmc.sh)
 */

#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>
#include <assert.h>

#define CAP 4               /* power of two, > number of posts */
#define MASK (CAP - 1)

/* --- Vyukov bounded MPSC ring (cell reduced to an int `val`) --------- */
typedef struct {
    _Atomic uint64_t seq;
    int              val;
} slot_t;

typedef struct {
    slot_t           slots[CAP];
    _Atomic uint64_t enqueue_pos;
    _Atomic uint64_t dequeue_pos;
} ring_t;

static void ring_init(ring_t *r) {
    for (uint64_t i = 0; i < CAP; i++)
        atomic_store_explicit(&r->slots[i].seq, i, memory_order_relaxed);
    atomic_store_explicit(&r->enqueue_pos, 0, memory_order_relaxed);
    atomic_store_explicit(&r->dequeue_pos, 0, memory_order_relaxed);
}

/* Canonical Vyukov producer. Returns 1 on enqueue, 0 if full. */
static int ring_try_enqueue(ring_t *r, int val) {
    slot_t *slot;
    uint64_t pos = atomic_load_explicit(&r->enqueue_pos, memory_order_relaxed);
    for (;;) {
        slot = &r->slots[pos & MASK];
        uint64_t seq = atomic_load_explicit(&slot->seq, memory_order_acquire);
        int64_t dif = (int64_t)(seq - pos);
        if (dif == 0) {
            if (atomic_compare_exchange_weak_explicit(
                    &r->enqueue_pos, &pos, pos + 1,
                    memory_order_relaxed, memory_order_relaxed))
                break;
        } else if (dif < 0) {
            return 0;                       /* full */
        } else {
            pos = atomic_load_explicit(&r->enqueue_pos, memory_order_relaxed);
        }
    }
    slot->val = val;                        /* plain store — published below */
    atomic_store_explicit(&slot->seq, pos + 1, memory_order_release);
    return 1;
}

/* Canonical Vyukov single-consumer dequeue. 1 + *out on success, else 0. */
static int ring_try_dequeue(ring_t *r, int *out) {
    uint64_t pos = atomic_load_explicit(&r->dequeue_pos, memory_order_relaxed);
    slot_t *slot = &r->slots[pos & MASK];
    uint64_t seq = atomic_load_explicit(&slot->seq, memory_order_acquire);
    int64_t dif = (int64_t)(seq - (pos + 1));
    if (dif == 0) {
        *out = slot->val;                   /* plain load — pairs w/ release */
        atomic_store_explicit(&slot->seq, pos + MASK + 1, memory_order_release);
        atomic_store_explicit(&r->dequeue_pos, pos + 1, memory_order_relaxed);
        return 1;
    }
    return 0;                               /* empty */
}

/* ==================================================================== *
 * Phase 1 — ring safety: 2 concurrent cross-pool producers + 1 worker.
 * Checks (1) no loss, (2) no dup, (3) no torn value under concurrent
 * enqueue-CAS contention and the enqueue/dequeue handoff. This is exactly
 * the cross-pool-flood shape (N publishers → one coop worker draining).
 * ==================================================================== */
static ring_t R1;
static int    g_seen[3];     /* values posted are 1 and 2 */

static void *p1_poster_a(void *_) { (void)_; int ok = ring_try_enqueue(&R1, 1); assert(ok); return NULL; }
static void *p1_poster_b(void *_) { (void)_; int ok = ring_try_enqueue(&R1, 2); assert(ok); return NULL; }

static void *p1_worker(void *_) {
    (void)_;
    for (int got = 0; got < 2; got++) {
        int v;
        while (!ring_try_dequeue(&R1, &v)) { /* spin until a cell arrives */ }
        assert(v == 1 || v == 2);            /* (3) no torn / spurious value */
        g_seen[v]++;
    }
    return NULL;
}

static void phase1_ring_safety(void) {
    ring_init(&R1);
    g_seen[1] = 0;
    g_seen[2] = 0;
    pthread_t pa, pb, dr;
    pthread_create(&dr, NULL, p1_worker, NULL);
    pthread_create(&pa, NULL, p1_poster_a, NULL);
    pthread_create(&pb, NULL, p1_poster_b, NULL);
    pthread_join(pa, NULL);
    pthread_join(pb, NULL);
    pthread_join(dr, NULL);
    assert(g_seen[1] == 1);                  /* (1) no loss + (2) no dup */
    assert(g_seen[2] == 1);
}

/* ==================================================================== *
 * Phase 2 — missed-wakeup handshake (SB litmus on ring-vs-parked).
 *
 * One cross-pool producer posts a single cell, then (fence) checks `parked`
 * and records whether it would cond_signal. The coop worker attempts a
 * dequeue; if empty it sets parked=1, (fence) rechecks; if STILL empty it
 * commits to parking. The Dekker/SB property of the two seq_cst fences
 * forbids (producer saw parked==0) AND (worker saw ring empty on recheck).
 * So: worker parked ⇒ producer signaled = "no missed wakeup": the coop
 * worker never sleeps on a cell that no producer will wake it for.
 * ==================================================================== */
static ring_t        R2;
static _Atomic int   g_parked;
static int           g_signaled;     /* producer observed parked → would signal */
static int           g_worker_parked;
static int           g_drained_val;  /* -1 if the worker parked instead */

static void *p2_poster(void *_) {
    (void)_;
    int ok = ring_try_enqueue(&R2, 7);       /* publish (release on slot seq) */
    assert(ok);
#ifdef BREAK_HANDSHAKE
    /* Negative control: a release fence does NOT order the prior store
     * before the later load (it is not a full StoreLoad barrier), so the SB
     * outcome reappears and assertion (4) fires. */
    atomic_thread_fence(memory_order_release);
#else
    atomic_thread_fence(memory_order_seq_cst);
#endif
    if (atomic_load_explicit(&g_parked, memory_order_seq_cst))
        g_signaled = 1;                      /* models pthread_cond_signal */
    return NULL;
}

static void *p2_worker(void *_) {
    (void)_;
    int v;
    if (ring_try_dequeue(&R2, &v)) {         /* got it without parking */
        g_drained_val = v;
        return NULL;
    }
    atomic_store_explicit(&g_parked, 1, memory_order_seq_cst);
#ifdef BREAK_HANDSHAKE
    atomic_thread_fence(memory_order_release);
#else
    atomic_thread_fence(memory_order_seq_cst);
#endif
    if (ring_try_dequeue(&R2, &v)) {         /* recheck saw the cell */
        atomic_store_explicit(&g_parked, 0, memory_order_seq_cst);
        g_drained_val = v;
        return NULL;
    }
    g_worker_parked = 1;                     /* committed to sleeping */
    g_drained_val = -1;
    return NULL;
}

static void phase2_missed_wakeup(void) {
    ring_init(&R2);
    atomic_store_explicit(&g_parked, 0, memory_order_relaxed);
    g_signaled = 0;
    g_worker_parked = 0;
    g_drained_val = 0;
    pthread_t po, dr;
    pthread_create(&dr, NULL, p2_worker, NULL);
    pthread_create(&po, NULL, p2_poster, NULL);
    pthread_join(po, NULL);
    pthread_join(dr, NULL);

    /* (4) NO MISSED WAKEUP: if the worker committed to parking, the
     * producer must have seen parked==1 and signaled — otherwise the real
     * coop worker would sleep forever on an already-posted cell. */
    if (g_worker_parked) {
        assert(g_signaled == 1);
    } else {
        /* Did not park ⇒ it dequeued the cell exactly (no torn value). */
        assert(g_drained_val == 7);
    }
}

int main(void) {
    phase1_ring_safety();
    phase2_missed_wakeup();
    return 0;
}
