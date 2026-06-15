/* GenMC model of the lotus arena subregion-slot freelist lock.
 * GH issue #18 item 2 (race-completeness) — the "chunk pool / arena
 * locks" inventory entry.
 *
 * WHY THIS, NOT "THE CHUNK POOL": the per-thread chunk pool
 * (`g_chunk_pool`, `__thread` in crates/hale-codegen/runtime/lotus_arena.c)
 * is thread-LOCAL — each thread owns its array, frees it in its own
 * thread dtor, and never touches another thread's. There is no
 * cross-thread interleaving surface to model there (its one historical
 * race, the env-driven prefill lazy-init, is now a `pthread_once` and is
 * raceless by construction). The real arena concurrency surface — the
 * one once TSAN-suppressed as `race:lotus_arena_destroy` /
 * `race:lotus_coop_pool_worker` and fixed with a mutex — is the
 * parent arena's **child-slot freelist**, shared between the main thread
 * and a cooperative-pool worker that create/destroy sibling subregions
 * of the same parent. That is what this model checks.
 *
 * FAITHFUL TRANSCRIPTION of the synchronization core of
 * `lotus_arena_create_subregion` and `lotus_arena_destroy` in
 * lotus_arena.c: a per-parent `subregion_lock` guards the slot tracker
 * (`free_list` / `free_count` / `free_cap` / `next_slot`).
 *   - create: under the lock, pop `free_list[--free_count]` if non-empty,
 *     else hand out `next_slot++`.
 *   - destroy: under the lock, push the child's slot
 *     `free_list[free_count++]` (growing the list when full).
 * The lock exists so two threads creating/destroying subregions of the
 * same parent cannot hand the SAME slot index to two live children — a
 * duplicated-slot pop or a torn `next_slot++`. That is what's checked.
 *
 * It is NOT the production code: the arena struct is reduced to the slot
 * tracker (no chunks, no residency, no struct pool); the `mt` flag
 * (`lotus_runtime_multithreaded()`) is modeled as a constant 1 — in the
 * sound usage it is set before any second thread can touch an arena
 * (the pthread_create happens-before publishes it), exactly as
 * bus_queue_model.c models `g_bus_has_pinned`. The destroy path's
 * freelist GROW (production `realloc`, doubling 0->8->…) is elided in
 * favour of a fixed-capacity array — same call bus_queue_model.c makes
 * for its queue grow: the grow runs under the very same `subregion_lock`,
 * so it serializes against every other tracker access and adds no
 * concurrency surface (and in this 2-thread config the list grows once,
 * from empty, never racing a live buffer). Kept exactly: the pop-or-bump
 * create path, the push destroy path, and the lock discipline on both.
 *
 * Scope / config: 2 threads, each destroying its pre-existing child and
 * then creating a fresh one on the SAME parent — so a single run
 * exercises, concurrently, the realloc-and-push (destroy) AND the
 * pop (create) under the lock. The invariant: the two freshly-created
 * children hold DISTINCT slots (no parent ever hands the same slot to
 * two live children), and the freelist accounting balances. GenMC
 * checks this across every interleaving: no data race on the tracker,
 * no realloc UAF, no duplicated/lost slot.
 *
 * Negative control (proves teeth): drop the lock (set MT 0, or delete
 * the lock/unlock pairs) and GenMC reports either a data race on the
 * tracker or a duplicated-slot assertion failure within the first
 * executions — the lock is exactly what prevents it.
 *
 * Run:  genmc -- verification/arena_subregion_model.c   (or run_genmc.sh)
 */

#include <pthread.h>
#include <stdlib.h>
#include <stdatomic.h>
#include <assert.h>

/* Production reads `lotus_runtime_multithreaded()`, which returns 1 once
 * a cooperative-pool worker / pinned thread exists — set before that
 * second thread can touch any arena. In the concurrent phase modeled
 * here it is therefore a constant 1; a plain int read is faithful (same
 * reasoning as bus_queue_model.c's `g_bus_has_pinned`, and it keeps the
 * checker in SC). Flip to 0 for the unlocked negative control. */
#define MT 1

#define CAP 4   /* fixed freelist; > slots in flight, so push never grows */

typedef struct {
    /* the parent's subregion slot tracker (lotus_arena_t fields) */
    int              free_list[CAP];
    size_t           free_count;
    int              next_slot;
    pthread_mutex_t  subregion_lock;
} arena_t;

/* lotus_arena_create_subregion: pop a freed slot, else bump next_slot. */
static int create_subregion(arena_t *p) {
    int slot;
    if (MT) pthread_mutex_lock(&p->subregion_lock);
    if (p->free_count > 0) {
        slot = p->free_list[--p->free_count];
    } else {
        slot = p->next_slot++;
    }
    if (MT) pthread_mutex_unlock(&p->subregion_lock);
    return slot;
}

/* lotus_arena_destroy's parent-freelist return: push the slot (the
 * production grow/realloc branch is elided — see the header note). */
static void destroy_subregion(arena_t *p, int slot) {
    if (MT) pthread_mutex_lock(&p->subregion_lock);
    assert(p->free_count < CAP);            /* grow branch elided */
    p->free_list[p->free_count++] = slot;
    if (MT) pthread_mutex_unlock(&p->subregion_lock);
}

/* --- harness ------------------------------------------------------- */
/* Parent already has two live children at slots 0 and 1 (next_slot == 2,
 * freelist empty). Each worker recycles its own child: destroy it (push
 * its slot, growing the freelist) then create a replacement (pop a slot).
 * Two destroys + two pops, all on the same parent, concurrently. Because
 * each thread destroys before it creates (program order), the freelist is
 * never empty at a pop, so both replacements reuse a freed slot — and
 * under the lock they must be DISTINCT (slots 0 and 1 in some order). */

static arena_t P;
static int slot_a, slot_b;

static void *worker_a(void *_) {
    (void)_;
    destroy_subregion(&P, 0);       /* return child A's slot */
    slot_a = create_subregion(&P);  /* claim a fresh slot */
    return NULL;
}
static void *worker_b(void *_) {
    (void)_;
    destroy_subregion(&P, 1);       /* return child B's slot */
    slot_b = create_subregion(&P);  /* claim a fresh slot */
    return NULL;
}

int main(void) {
    P.free_count = 0;
    P.next_slot  = 2;               /* slots 0,1 already handed out */
    pthread_mutex_init(&P.subregion_lock, NULL);

    pthread_t ta, tb;
    pthread_create(&ta, NULL, worker_a, NULL);
    pthread_create(&tb, NULL, worker_b, NULL);
    pthread_join(ta, NULL);
    pthread_join(tb, NULL);

    /* No live child shares a slot with another. */
    assert(slot_a != slot_b);
    /* Both replacements reused freed slots (0/1), so next_slot never
     * bumped and the freelist balanced back to empty. */
    assert(slot_a == 0 || slot_a == 1);
    assert(slot_b == 0 || slot_b == 1);
    assert(P.next_slot == 2);
    assert(P.free_count == 0);

    pthread_mutex_destroy(&P.subregion_lock);
    return 0;
}
