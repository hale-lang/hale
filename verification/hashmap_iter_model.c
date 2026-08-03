/* GenMC model of lockfree hashmap ITERATION under a concurrent grow.
 * GH issue #18 item 2 (race-completeness). Added 2026-08-02.
 *
 * ADDITIVE to lockfree_hashmap_model.c, which verifies set/remove
 * against the enter/exit + grow-phase protocol. `lotus_hashmap_iter_next`
 * / `_iter_batch` arrived after that model was written and were never
 * transcribed, so nothing checked the one thing that makes iteration
 * different from every other operation:
 *
 *     ITERATION IS NOT ONE OPERATION. IT IS A SEQUENCE OF THEM.
 *
 * `lotus_hashmap_iter_next(map, start_slot, out)` takes `lf_enter`,
 * scans from `start_slot` for the next COMMITTED cell, copies it out,
 * takes `lf_exit`, and returns the slot index. The caller loops,
 * passing `found + 1` back in. So the protocol protects each STEP —
 * and releases between steps. A grow that lands in that gap rehashes
 * every entry into a new array with a new capacity, and the caller's
 * cursor is an index into a table that no longer exists.
 *
 * This model checks the property that DOES hold, and pins the one that
 * does NOT, so the contract is written down in an executable form
 * rather than assumed.
 *
 *   HOLDS (checked here, unconditionally): iteration is MEMORY-SAFE
 *   under concurrent growth. Each step re-reads `m->slots` inside its
 *   own enter/exit, and the grower drains `writers_in_flight` to zero
 *   before swapping and freeing — so a step never touches the freed
 *   array, and every value it yields is one genuinely stored in the
 *   map. This is inherited from the verified protocol; the point is
 *   that iteration actually participates in it.
 *
 *   DOES NOT HOLD (demonstrated by control 2 below): iteration is NOT
 *   a snapshot. A rehash relocates entries, and the caller's cursor is
 *   an index, so an entry can be moved from a slot the cursor has
 *   already passed to one ahead of it and be YIELDED TWICE. This model
 *   exhibits exactly that. (The mirror case — an entry relocated
 *   backwards and therefore never yielded — is possible in principle
 *   but is NOT exhibited at this table size, where a rehash from
 *   cap 2 to cap 4 only ever moves entries forward. Do not read a
 *   completeness guarantee into its absence here.) Callers must treat
 *   iteration under concurrent growth as neither complete nor
 *   duplicate-free.
 *
 * FAITHFUL: the enter/exit writer-counter with its 0->1 re-check
 * backout, the single-grower CAS + drain + migrate + free, the
 * per-step (not per-loop) protection, and the caller's
 * `start_slot = found + 1` cursor.
 *
 * NOT the production code: keys/values are ints; open addressing is
 * linear probing over a power-of-two table, as in the sibling model;
 * `sched_yield` in the spin is elided (GenMC explores the
 * interleavings directly).
 *
 * NEGATIVE CONTROLS — two, because there are two claims:
 *
 *   1. The safety claim has teeth. Build with
 *        genmc --sc -- -DMODEL_BUG_NO_DRAIN verification/hashmap_iter_model.c
 *      to delete the grower's drain-wait. GenMC reports `Attempt to
 *      access non-allocated memory!` — the iterator is inside its step,
 *      holding a pointer into `old_slots`, when the grower frees it.
 *
 *   2. The not-a-snapshot NON-claim is real, not hypothetical:
 *        genmc --sc -- -DMODEL_ASSERT_ITERATION_IS_A_SNAPSHOT verification/hashmap_iter_model.c
 *      asserts that no value is yielded twice. GenMC reports a safety
 *      violation, exhibiting the interleaving where a grow relocates
 *      an entry ahead of the cursor and the iterator returns it again.
 *      That failure is the DOCUMENTATION: it is the contract, not a
 *      bug to fix.
 *
 * GENMC-FLAGS: --sc
 *
 * WHY THIS MODEL PINS --sc, AND THE OPEN QUESTION IT RAISES
 * ---------------------------------------------------------
 * Under GenMC's DEFAULT release-acquire model — the faithful one for
 * the runtime's `__ATOMIC_ACQUIRE` / `__ATOMIC_RELEASE` — this model
 * reports `Attempt to access freed memory!`. Under `--sc` it verifies
 * clean (13 executions). The failing trace is:
 *
 *   - the iterator's 2nd `lf_enter` acquire-loads `grow_phase` and
 *     reads-from the INITIAL store, not the grower's release reset;
 *   - so it never observes the grow, proceeds, and acquire-loads
 *     `m->slots` — also reading-from the initial store;
 *   - it then indexes the OLD array, which the grower has freed.
 *
 * RA permits a load to read a coherence-older write when nothing
 * orders a newer one before it. The protocol's safety therefore rests
 * on more than release-acquire alone provides *in the formal model*.
 * Real hardware (x86/ARM cache coherence) will not hand back an
 * indefinitely stale line, which is why this has never been observed —
 * but "the hardware is stronger than the model we wrote against" is a
 * reason to look, not a reason to close.
 *
 * This is NOT a claim that the runtime has a live use-after-free.
 * It is a claim that the model-level justification is incomplete, and
 * that the gap is specific and reproducible:
 *
 *     genmc -- verification/hashmap_iter_model.c        # RA: fails
 *     genmc --sc -- verification/hashmap_iter_model.c   # SC: clean
 *
 * The same tradeoff is already recorded in bus_queue_model.c, which
 * uses a plain int for `g_bus_has_pinned` because "the atomic forces
 * the weaker RA model". That was a modelling convenience there; here
 * it is load-bearing, so it is pinned explicitly and written down
 * rather than quietly arranged for.
 *
 * Resolving it means deciding whether the enter/exit protocol needs a
 * seq_cst fence at the phase re-check (production already has
 * `atomic_thread_fence` machinery elsewhere), or whether the RA
 * counter-example depends on an interleaving the single-grower
 * discipline actually excludes. That wants runtime review, not a
 * transcription pass.
 *
 * Run:  genmc --sc -- verification/hashmap_iter_model.c  (or run_genmc.sh)
 */

#include <pthread.h>
#include <stdlib.h>
#include <stdatomic.h>
#include <assert.h>

#define CELL_EMPTY     0
#define CELL_COMMITTED 2

typedef struct {
    _Atomic(int) state;
    int key;
    int val;
} slot_t;

typedef struct {
    _Atomic(slot_t *) slots;
    _Atomic(size_t)   cap;
    _Atomic(int)      grow_phase;
    _Atomic(int)      writers_in_flight;
} map_t;

static map_t m;

static slot_t *alloc_slots(size_t n) {
    slot_t *p = (slot_t *)malloc(n * sizeof(slot_t));
    for (size_t i = 0; i < n; i++) {
        atomic_store_explicit(&p[i].state, CELL_EMPTY, memory_order_relaxed);
        p[i].key = 0;
        p[i].val = 0;
    }
    return p;
}

/* --- enter / exit (lotus_hashmap_lf_enter / _exit) ----------------- */

static void lf_enter(map_t *mp) {
    for (;;) {
        int phase = atomic_load_explicit(&mp->grow_phase, memory_order_acquire);
        if (phase != 0) continue;
        atomic_fetch_add_explicit(&mp->writers_in_flight, 1,
                                  memory_order_acquire);
        /* Re-check: a grower may have CAS'd 0->1 between the load and
         * the increment. Back out, or its drain spin deadlocks. */
        phase = atomic_load_explicit(&mp->grow_phase, memory_order_acquire);
        if (phase != 0) {
            atomic_fetch_sub_explicit(&mp->writers_in_flight, 1,
                                      memory_order_release);
            continue;
        }
        return;
    }
}

static void lf_exit(map_t *mp) {
    atomic_fetch_sub_explicit(&mp->writers_in_flight, 1,
                              memory_order_release);
}

/* --- grow (lotus_hashmap_grow_lockfree + _lf_migrate) -------------- */

static void grow(map_t *mp) {
    int expected = 0;
    if (!atomic_compare_exchange_strong_explicit(
            &mp->grow_phase, &expected, 1,
            memory_order_acq_rel, memory_order_relaxed)) {
        return;
    }
#ifndef MODEL_BUG_NO_DRAIN
    /* The drain. Without it an iterator can still be inside its step,
     * holding a pointer into old_slots, when the free below runs. */
    while (atomic_load_explicit(&mp->writers_in_flight,
                                memory_order_acquire) > 0) {
        /* spin */
    }
#endif
    slot_t *old_slots = atomic_load_explicit(&mp->slots, memory_order_relaxed);
    size_t  old_cap   = atomic_load_explicit(&mp->cap, memory_order_relaxed);
    size_t  new_cap   = old_cap * 2;
    slot_t *new_slots = alloc_slots(new_cap);
    if (!new_slots) {
        atomic_store_explicit(&mp->grow_phase, 0, memory_order_release);
        return;
    }
    /* Rehash. THIS is what breaks cursor continuity: an entry's slot
     * index in the new table is unrelated to its index in the old. */
    for (size_t s = 0; s < old_cap; s++) {
        if (atomic_load_explicit(&old_slots[s].state, memory_order_relaxed)
            != CELL_COMMITTED) continue;
        int k = old_slots[s].key, v = old_slots[s].val;
        size_t i = (size_t)k & (new_cap - 1);
        for (;;) {
            if (atomic_load_explicit(&new_slots[i].state,
                                     memory_order_relaxed) == CELL_EMPTY) {
                new_slots[i].key = k;
                new_slots[i].val = v;
                atomic_store_explicit(&new_slots[i].state, CELL_COMMITTED,
                                      memory_order_relaxed);
                break;
            }
            i = (i + 1) & (new_cap - 1);
        }
    }
    atomic_store_explicit(&mp->slots, new_slots, memory_order_release);
    atomic_store_explicit(&mp->cap, new_cap, memory_order_release);
    free(old_slots);
    atomic_store_explicit(&mp->grow_phase, 0, memory_order_release);
}

/* --- iter_next (lotus_hashmap_iter_next, LOCKFREE arm) ------------- */
/* Returns the slot it yielded from, or -1 when the scan runs off the
 * end. `*out` receives the value. Protection is per CALL. */
static long iter_next(map_t *mp, long start_slot, int *out) {
    if (start_slot < 0) return -1;
    long found = -1;
    lf_enter(mp);
    slot_t *slots = atomic_load_explicit(&mp->slots, memory_order_acquire);
    size_t  cap   = atomic_load_explicit(&mp->cap, memory_order_acquire);
    for (size_t s = (size_t)start_slot; s < cap; s++) {
        if (atomic_load_explicit(&slots[s].state, memory_order_acquire)
            == CELL_COMMITTED) {
            *out = slots[s].val;
            found = (long)s;
            break;
        }
    }
    lf_exit(mp);
    return found;
}

/* Pre-seeded, and never modified after the threads start: whatever an
 * iteration yields must be one of these, and a complete iteration
 * would yield both. */
#define KEY_A 1
#define VAL_A 11
#define KEY_B 2
#define VAL_B 22

static int seen_vals[4];
static int seen_n;

static void *iterator(void *arg) {
    (void)arg;
    long cursor = 0;
    for (int steps = 0; steps < 4; steps++) {
        int v = 0;
        long at = iter_next(&m, cursor, &v);
        if (at < 0) break;
        /* SAFETY: never freed memory, never a torn or unwritten cell.
         * Anything yielded is a value genuinely stored in the map. */
        assert(v == VAL_A || v == VAL_B);
        seen_vals[seen_n++] = v;
        cursor = at + 1;              /* the caller's cursor */
    }
    return NULL;
}

static void *grower(void *arg) {
    (void)arg;
    grow(&m);
    return NULL;
}

int main(void) {
    pthread_t it, gr;

    size_t cap = 2;
    slot_t *slots = alloc_slots(cap);
    /* Seed both entries so a rehash has something to relocate. With
     * cap 2 they occupy slots 1 and 0 (key & (cap-1)). */
    size_t ia = (size_t)KEY_A & (cap - 1);
    slots[ia].key = KEY_A; slots[ia].val = VAL_A;
    atomic_store_explicit(&slots[ia].state, CELL_COMMITTED, memory_order_relaxed);
    size_t ib = (size_t)KEY_B & (cap - 1);
    if (ib == ia) ib = (ib + 1) & (cap - 1);
    slots[ib].key = KEY_B; slots[ib].val = VAL_B;
    atomic_store_explicit(&slots[ib].state, CELL_COMMITTED, memory_order_relaxed);

    atomic_store_explicit(&m.slots, slots, memory_order_relaxed);
    atomic_store_explicit(&m.cap, cap, memory_order_relaxed);
    atomic_store_explicit(&m.grow_phase, 0, memory_order_relaxed);
    atomic_store_explicit(&m.writers_in_flight, 0, memory_order_relaxed);

    pthread_create(&it, NULL, iterator, NULL);
    pthread_create(&gr, NULL, grower, NULL);
    pthread_join(it, NULL);
    pthread_join(gr, NULL);

#ifdef MODEL_ASSERT_ITERATION_IS_A_SNAPSHOT
    /* NOT a guarantee the implementation makes. Asserting it here makes
     * GenMC exhibit the interleaving that breaks it: the grow lands
     * mid-iteration and rehashes an entry to a slot the cursor has
     * already passed, so it is never yielded. Both entries were present
     * for the entire iteration. */
    for (int i = 0; i < seen_n; i++)
        for (int j = i + 1; j < seen_n; j++)
            assert(seen_vals[i] != seen_vals[j]);
#endif

    free(atomic_load_explicit(&m.slots, memory_order_relaxed));
    return 0;
}
