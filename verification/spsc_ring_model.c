/* GenMC model of the lotus SPSC observation ring (GH #244).
 *
 * FAITHFUL TRANSCRIPTION of the distinctive concurrency in
 * `lotus_spsc_emit` / `lotus_spsc_read`
 * (crates/hale-codegen/runtime/lotus_arena.c): the producer
 * plain-writes both slot words and THEN publishes `head+1` with
 * a release store; the consumer acquire-loads the head (h1),
 * copies a batch, acquire-loads again (h2), and DISCARDS every
 * copied record with index <= h2 - slots — because the
 * producer's in-flight write for record h2 is already clobbering
 * slot index h2 - slots before h2+1 is published. (That `<=` is
 * load-bearing: the `<` boundary from the pre-freeze iris
 * PROTOCOL.md sketch delivers a torn/future record — the
 * concurrent driver test in tests/spsc_driver.c refutes it
 * empirically, and flipping DISCARD_BOUNDARY_STRICT below to 1
 * lets GenMC refute it exhaustively.)
 *
 * NOT the production code: ring shrunk to 2 slots, 4 records,
 * batch of 1. Transcription note: production slot writes/reads
 * are PLAIN accesses — torn reads are tolerated by design
 * because every possibly-torn record falls in the discarded
 * window. C11 calls concurrent plain access a data race, so the
 * model expresses the same design as RELAXED atomics on the
 * slot words; the checked invariant is the algorithm's real
 * contract: no DELIVERED record is torn (w1 == w0 ^ MASK) and
 * delivered seqs are strictly increasing.
 *
 * Run:  genmc -- verification/spsc_ring_model.c   (or run_genmc.sh)
 */

#include <assert.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>

#define SLOTS 2u
#define PRODUCE 4u
#define MASK 0xA5u
#define DISCARD_BOUNDARY_STRICT 0 /* 1 = iris's pre-freeze `<`: REFUTED */

static _Atomic uint64_t head;
static _Atomic uint64_t slot_w0[SLOTS];
static _Atomic uint64_t slot_w1[SLOTS];

static void *producer(void *arg) {
    (void)arg;
    for (uint64_t i = 0; i < PRODUCE; i++) {
        uint64_t h = atomic_load_explicit(&head, memory_order_relaxed);
        atomic_store_explicit(&slot_w0[h & (SLOTS - 1)], i,
                              memory_order_relaxed);
        atomic_store_explicit(&slot_w1[h & (SLOTS - 1)], i ^ MASK,
                              memory_order_relaxed);
        atomic_store_explicit(&head, h + 1, memory_order_release);
    }
    return NULL;
}

int main(void) {
    pthread_t p;
    pthread_create(&p, NULL, producer, NULL);

    uint64_t c = 0;
    uint64_t last = 0;
    int have_last = 0;
    /* Bounded consumer: up to PRODUCE single-record snapshot
     * reads interleaved arbitrarily with the producer. */
    for (unsigned round = 0; round < PRODUCE; round++) {
        uint64_t h1 = atomic_load_explicit(&head, memory_order_acquire);
        uint64_t live_min1 =
            h1 >= SLOTS ? h1 - SLOTS + (DISCARD_BOUNDARY_STRICT ? 0 : 1)
                        : 0;
        if (c < live_min1) {
            c = live_min1;
        }
        if (c >= h1) {
            continue; /* nothing published yet */
        }
        uint64_t w0 = atomic_load_explicit(&slot_w0[c & (SLOTS - 1)],
                                           memory_order_relaxed);
        uint64_t w1 = atomic_load_explicit(&slot_w1[c & (SLOTS - 1)],
                                           memory_order_relaxed);
        uint64_t h2 = atomic_load_explicit(&head, memory_order_acquire);
        uint64_t live_min2 =
            h2 >= SLOTS ? h2 - SLOTS + (DISCARD_BOUNDARY_STRICT ? 0 : 1)
                        : 0;
        if (c < live_min2) {
            /* possibly overwritten mid-copy: discard, advance. */
            c = live_min2;
            continue;
        }
        /* DELIVERED record: the algorithm's contract. */
        assert(w1 == (w0 ^ MASK));
        assert(w0 == c);
        if (have_last) {
            assert(w0 > last);
        }
        last = w0;
        have_last = 1;
        c++;
    }

    pthread_join(p, NULL);
    return 0;
}
