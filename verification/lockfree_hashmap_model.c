/* GenMC model of the lotus lockfree hashmap concurrency protocol.
 * GH issue #18 item 2 (race-completeness), proof-of-concept.
 *
 * This is a FAITHFUL TRANSCRIPTION of the synchronization core of
 * `lotus_hashmap_*_lockfree` in
 * crates/hale-codegen/runtime/lotus_arena.c — the enter/exit
 * writer-counter protocol, the single-grower grow phase, the
 * writers-in-flight drain, and the EMPTY→CLAIMED→COMMITTED set state
 * machine. It is NOT the production code; it strips the payload
 * (key/value are ints), the hash (identity), tombstones, and the
 * striped/serialized modes, keeping exactly the atomics and orderings
 * that decide thread-safety. If the production atomics or orderings
 * change, this model must change with them.
 *
 * GenMC explores EVERY thread interleaving permitted by the C11
 * memory model and checks, automatically:
 *   - no data race (non-atomic conflicting access),
 *   - no use-after-free (a reader/writer touching old `slots` after
 *     grow frees it — the property the enter/drain protocol exists to
 *     guarantee),
 *   - no assertion failure (our hand-written invariants below).
 *
 * Run:  genmc -- verification/lockfree_hashmap_model.c
 * (or:  verification/run_genmc.sh)
 */

#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdatomic.h>
#include <assert.h>

/* Cell occupancy states — mirror lotus_arena.c. */
#define CELL_EMPTY     0
#define CELL_CLAIMED   1
#define CELL_COMMITTED 2
/* TOMBSTONE (3) omitted: the remove path is a separate, simpler CAS;
 * the grow/drain race surface this PoC targets lives in set+grow. */

/* Load factor: grow when len*10 > cap*6 (LOTUS_HASHMAP_LF_LOAD_NUM/
 * LOAD_DEN). Kept identical so the grow trigger fires at the same
 * point the production map grows. */
#define LOAD_NUM 6
#define LOAD_DEN 10

typedef struct {
    _Atomic(unsigned char) state;
    int key;
    int val;
} slot_t;

typedef struct {
    _Atomic(slot_t *) slots;
    _Atomic(size_t)   cap;
    _Atomic(int)      grow_phase;        /* 0 = open, 1 = growing */
    _Atomic(int)      writers_in_flight;
    _Atomic(size_t)   len;
} map_t;


/* GenMC's runtime models malloc/free but not calloc; allocate + zero
 * explicitly. Single-threaded at every call site (init / mid-grow). */
static slot_t *alloc_slots(size_t n) {
    slot_t *p = (slot_t *)malloc(n * sizeof(slot_t));
    for (size_t i = 0; i < n; i++) { p[i].state = CELL_EMPTY; p[i].key = 0; p[i].val = 0; }
    return p;
}

/* --- enter / exit (lotus_hashmap_lf_enter / _exit) ----------------- */

static void lf_enter(map_t *m) {
    for (;;) {
        int phase = atomic_load_explicit(&m->grow_phase, memory_order_acquire);
        if (phase != 0) { continue; }          /* spin: grow in progress */
        atomic_fetch_add_explicit(&m->writers_in_flight, 1,
                                   memory_order_acquire);
        /* Re-check: a grower could have CAS'd 0→1 between our load and
         * our fetch_add. If so, back out so the grower's drain can
         * reach 0. */
        phase = atomic_load_explicit(&m->grow_phase, memory_order_acquire);
        if (phase != 0) {
            atomic_fetch_sub_explicit(&m->writers_in_flight, 1,
                                       memory_order_release);
            continue;
        }
        return;
    }
}

static void lf_exit(map_t *m) {
    atomic_fetch_sub_explicit(&m->writers_in_flight, 1, memory_order_release);
}

/* --- grow (lotus_hashmap_grow_lockfree + _lf_migrate) -------------- */

static void grow(map_t *m) {
    int expected = 0;
    if (!atomic_compare_exchange_strong_explicit(
            &m->grow_phase, &expected, 1,
            memory_order_acq_rel, memory_order_relaxed)) {
        return;                                 /* another grower won */
    }
    /* Drain: wait for every in-flight writer to exit. New writers
     * back off via the enter re-check. */
    while (atomic_load_explicit(&m->writers_in_flight,
                                memory_order_acquire) > 0) {
        /* spin */
    }
    slot_t *old_slots = atomic_load_explicit(&m->slots, memory_order_relaxed);
    size_t  old_cap   = atomic_load_explicit(&m->cap, memory_order_relaxed);
    size_t  new_cap   = old_cap * 2;
    slot_t *new_slots = alloc_slots(new_cap);
    if (!new_slots) { atomic_store_explicit(&m->grow_phase, 0,
                                             memory_order_release); return; }
    /* Single-threaded migration — drain guarantees no concurrent op
     * touches old_slots. Rehash live (COMMITTED) cells. */
    size_t live = 0;
    for (size_t s = 0; s < old_cap; s++) {
        if (atomic_load_explicit(&old_slots[s].state, memory_order_relaxed)
            != CELL_COMMITTED) continue;
        int k = old_slots[s].key, v = old_slots[s].val;
        size_t i = (size_t)k & (new_cap - 1);
        for (;;) {
            if (new_slots[i].state == CELL_EMPTY) {
                new_slots[i].key = k;
                new_slots[i].val = v;
                new_slots[i].state = CELL_COMMITTED;
                live++;
                break;
            }
            i = (i + 1) & (new_cap - 1);
        }
    }
    atomic_store_explicit(&m->len, live, memory_order_relaxed);
    atomic_store_explicit(&m->slots, new_slots, memory_order_release);
    atomic_store_explicit(&m->cap, new_cap, memory_order_release);
    free(old_slots);                            /* the UAF cliff edge */
    atomic_store_explicit(&m->grow_phase, 0, memory_order_release);
}

/* --- set (lotus_hashmap_set_lockfree) ------------------------------ */

static void set(map_t *m, int key, int val) {
    int did_grow_check = 0;
set_retry:
    lf_enter(m);
    size_t cap  = atomic_load_explicit(&m->cap, memory_order_relaxed);
    slot_t *slots = atomic_load_explicit(&m->slots, memory_order_relaxed);
    size_t mask = cap - 1;
    size_t i = (size_t)key & mask;
    size_t probes = 0;
    for (;;) {
        if (probes >= cap) {                    /* probe exhausted → grow */
            lf_exit(m);
            grow(m);
            did_grow_check = 1;
            goto set_retry;
        }
        unsigned char st = atomic_load_explicit(&slots[i].state,
                                                 memory_order_acquire);
        if (st == CELL_EMPTY) {
            unsigned char exp = CELL_EMPTY;
            if (atomic_compare_exchange_strong_explicit(
                    &slots[i].state, &exp, CELL_CLAIMED,
                    memory_order_acquire, memory_order_relaxed)) {
                slots[i].key = key;
                slots[i].val = val;
                atomic_store_explicit(&slots[i].state, CELL_COMMITTED,
                                      memory_order_release);
                atomic_fetch_add_explicit(&m->len, 1, memory_order_relaxed);
                goto set_done;
            }
            continue;                           /* CAS lost; re-read */
        }
        if (st == CELL_CLAIMED) continue;       /* spin */
        if (slots[i].key == key) {              /* COMMITTED, same key */
            unsigned char exp = CELL_COMMITTED;
            if (atomic_compare_exchange_strong_explicit(
                    &slots[i].state, &exp, CELL_CLAIMED,
                    memory_order_acquire, memory_order_relaxed)) {
                slots[i].val = val;
                atomic_store_explicit(&slots[i].state, CELL_COMMITTED,
                                      memory_order_release);
                lf_exit(m);
                return;                         /* update: no grow */
            }
            continue;
        }
        i = (i + 1) & mask;
        probes++;
    }
set_done:
    {
        size_t cap_now = atomic_load_explicit(&m->cap, memory_order_relaxed);
        size_t live    = atomic_load_explicit(&m->len, memory_order_relaxed);
        lf_exit(m);
        if (!did_grow_check && live * LOAD_DEN > cap_now * LOAD_NUM) {
            grow(m);
        }
    }
}

/* --- harness ------------------------------------------------------- */
/* Two writers insert distinct keys into a cap-2 map. The second
 * insert pushes the load factor over the threshold, forcing exactly
 * one grow that races against the other in-flight writer — the
 * grow-during-write interleaving TSan can only sometimes hit and that
 * GenMC explores exhaustively. */

#define INIT_CAP 2

static map_t M;

static void *writer_a(void *_) { (void)_; set(&M, 0, 100); return NULL; }
static void *writer_b(void *_) { (void)_; set(&M, 1, 200); return NULL; }

int main(void) {
    M.slots = alloc_slots(INIT_CAP);
    M.cap = INIT_CAP;
    M.grow_phase = 0;
    M.writers_in_flight = 0;
    M.len = 0;

    pthread_t a, b;
    pthread_create(&a, NULL, writer_a, NULL);
    pthread_create(&b, NULL, writer_b, NULL);
    pthread_join(a, NULL);
    pthread_join(b, NULL);

    /* Invariant: both distinct keys survived every interleaving
     * (no lost insert, no corruption), and the live count agrees. */
    slot_t *s = atomic_load_explicit(&M.slots, memory_order_relaxed);
    size_t  c = atomic_load_explicit(&M.cap, memory_order_relaxed);
    int seen0 = 0, seen1 = 0;
    size_t committed = 0;
    for (size_t i = 0; i < c; i++) {
        if (s[i].state == CELL_COMMITTED) {
            committed++;
            if (s[i].key == 0 && s[i].val == 100) seen0 = 1;
            if (s[i].key == 1 && s[i].val == 200) seen1 = 1;
        }
    }
    assert(seen0 && seen1);
    assert(committed == 2);
    assert(atomic_load_explicit(&M.len, memory_order_relaxed) == 2);

    free(s);
    return 0;
}
