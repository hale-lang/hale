/* Shared reclaim admission versus a started run's self-reclaim (GH #1324).
 *
 * One live child, one already-admitted worker run, and one owner requesting
 * reclaim. Either thread may win the instance's claim. Only that thread
 * enters logical teardown and releases the arena; the owner waits for the
 * worker's hold, while self-reclaim excludes its own hold. Both claim before
 * reading the arena. The child's struct outlives these two entrants, as
 * required by their existing owner/run retention.
 *
 * Mirrors lotus_reclaim_try_claim's strong CAS (acq_rel / acquire) and
 * the run-hold mutex. The hold wait uses cascade_model.c's epoch reduction
 * of the timed condvar wait. The arena is represented by one heap cell.
 * This checks safety for competing shared-spine entries, not handler queue
 * progress, inline cascade arbitration, owner reclamation, incarnation
 * reuse, allocation failure, or correspondence of all generated code.
 *
 * Negative controls, each required to fail:
 *   -DMODEL_BUG_THREAD_LOCAL_CLAIM  both threads believe they own reclaim
 *   -DMODEL_BUG_CHECK_THEN_CLAIM    a load/store replaces the atomic claim
 *   -DMODEL_BUG_SKIP_RUN_WAIT      owner frees while worker can read
 */
#include <assert.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>

static _Atomic int64_t claimed;
static _Atomic int logical;
static _Atomic int released;
static pthread_mutex_t hold_lock = PTHREAD_MUTEX_INITIALIZER;
static _Atomic int hold_epoch;
static int held = 1;
static int *arena;

static int try_claim(void) {
#if defined(MODEL_BUG_THREAD_LOCAL_CLAIM)
    return 1;
#elif defined(MODEL_BUG_CHECK_THEN_CLAIM)
    if (atomic_load_explicit(&claimed, memory_order_acquire)) return 0;
    atomic_store_explicit(&claimed, 1, memory_order_release);
    return 1;
#else
    int64_t expected = 0;
    return atomic_compare_exchange_strong_explicit(
        &claimed, &expected, 1, memory_order_acq_rel, memory_order_acquire);
#endif
}

static void wait_for_run(void) {
#ifndef MODEL_BUG_SKIP_RUN_WAIT
    pthread_mutex_lock(&hold_lock);
    while (held) {
        int epoch = atomic_load_explicit(&hold_epoch, memory_order_relaxed);
        pthread_mutex_unlock(&hold_lock);
        while (atomic_load_explicit(&hold_epoch, memory_order_relaxed) == epoch) { }
        pthread_mutex_lock(&hold_lock);
    }
    pthread_mutex_unlock(&hold_lock);
#endif
}

static void reclaim(int own_run) {
    if (!try_claim()) return;
    assert(atomic_fetch_add_explicit(&logical, 1, memory_order_relaxed) == 0);
    if (!own_run) wait_for_run();
    assert(*arena == 7);
    free(arena);
    assert(atomic_fetch_add_explicit(&released, 1, memory_order_relaxed) == 0);
}

static void *worker(void *unused) {
    (void)unused;
    assert(*arena == 7); /* last body access, before run-end self-reclaim */
    reclaim(1);
    pthread_mutex_lock(&hold_lock);
    held = 0;
    atomic_fetch_add_explicit(&hold_epoch, 1, memory_order_relaxed);
    pthread_mutex_unlock(&hold_lock);
    return NULL;
}

int main(void) {
    arena = malloc(sizeof *arena);
    assert(arena);
    *arena = 7;
    pthread_t thread;
    pthread_create(&thread, NULL, worker, NULL);
    reclaim(0);
    pthread_join(thread, NULL);
    assert(atomic_load_explicit(&logical, memory_order_relaxed) == 1);
    assert(atomic_load_explicit(&released, memory_order_relaxed) == 1);
    return 0;
}
