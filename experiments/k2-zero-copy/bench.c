/*
 * Form K2 (2026-05-20) — zero-copy vs memcpy bus boundary
 * microbench.
 *
 * Question this answers
 * ---------------------
 * The bus contract today copies payloads at every locus
 * boundary (m28b's cross-thread mailbox does TWO memcpys per
 * cell: publisher -> cell, cell -> subscriber arena). Form K's
 * proposed zero-copy route writes directly into a ring slot —
 * publisher's writes land in the slot, subscriber's view points
 * at the same slot, no copy at the boundary.
 *
 * Before building the full SHM ring substrate (K5/K6), this
 * bench validates that the win is large enough to justify the
 * design. If the in-place path isn't measurably faster than
 * the memcpy path for a realistic flat payload, the whole
 * design loses its rationale.
 *
 * Three paths measured
 * --------------------
 *   1. m28b-shape (today's worst case): construct a local
 *      payload + memcpy into a "cell" + memcpy into the
 *      subscriber's "arena". Two memcpys per dispatch.
 *
 *   2. implicit zero_copy lowering: construct + memcpy into
 *      a ring slot. ONE memcpy per dispatch. This is what
 *      `topics.X.publish(v)` would lower to under Form K
 *      when the topic's bus is zero_copy.
 *
 *   3. explicit zero_copy claim/commit: writes go directly
 *      into the ring slot, no separate local-construct phase.
 *      ZERO memcpys per dispatch. This is the
 *      `with topics.X.claim() as slot { slot.bid = ... }`
 *      surface.
 *
 * Each path ends with a release-ordered atomic seqno bump
 * (modelling the ring's commit barrier).
 *
 * Payload shape
 * -------------
 * 80-byte struct of 10 i64 fields. Roughly the shape of an L2
 * book-level update (bid/ask px+sz as i128 split into hi/lo
 * pairs, plus venue_ts + recv_ts). The exact value is irrelevant;
 * what matters is that it's a flat, fixed-layout payload that
 * fits the predicate `is_flat_shapeable`.
 *
 * Methodology
 * -----------
 * Per [[bench-methodology]]: 5 rounds of 100k iterations each;
 * report median ns/op per path. Each round is timed
 * separately so the median absorbs scheduler / cache jitter.
 *
 * Build & run
 * -----------
 *   gcc -O2 -o experiments/k2-zero-copy/bench experiments/k2-zero-copy/bench.c
 *   ./experiments/k2-zero-copy/bench
 *
 * Or use run.sh in this directory.
 */

#define _GNU_SOURCE
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define ITER_PER_ROUND 100000
#define N_ROUNDS       5

typedef struct {
    int64_t bid_px_hi, bid_px_lo;
    int64_t ask_px_hi, ask_px_lo;
    int64_t bid_sz_hi, bid_sz_lo;
    int64_t ask_sz_hi, ask_sz_lo;
    int64_t venue_ts;
    int64_t recv_ts;
} L2Update;

_Static_assert(sizeof(L2Update) == 80, "L2Update should be 80 bytes");

/* Ring slot — payload + atomic seqno for commit barrier. */
typedef struct {
    L2Update payload;
    _Atomic int64_t seqno;
    char _pad[64 - 8];  /* avoid false sharing between rounds */
} Slot;

/* Constructor for the publisher's locally-built payload. Marked
 * noinline so the compiler can't fold the construction into the
 * caller's memcpy site — modelling a real publisher where the
 * payload value comes from a function call (parse_message,
 * normalize_tick, etc.) the optimizer can't see through.
 *
 * Without this barrier the compiler folds local-construct +
 * memcpy into direct slot writes, making paths 2 and 3 appear
 * identical at -O2. Real publisher code rarely hits that ideal
 * (the payload usually flows out of some non-inlinable producer)
 * so the noinline boundary is the honest model. */
__attribute__((noinline))
static void build_payload(L2Update *out, int64_t recv_ts) {
    out->bid_px_hi = 100;
    out->bid_px_lo = 200;
    out->ask_px_hi = 300;
    out->ask_px_lo = 400;
    out->bid_sz_hi = 500;
    out->bid_sz_lo = 600;
    out->ask_sz_hi = 700;
    out->ask_sz_lo = 800;
    out->venue_ts  = 900;
    out->recv_ts   = recv_ts;
}

/* Path 1: m28b-shape — TWO memcpys per dispatch. */
static int64_t bench_m28b(Slot *cell, Slot *subscriber_arena) {
    L2Update local;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (int i = 0; i < ITER_PER_ROUND; i++) {
        build_payload(&local, i);

        /* memcpy 1: publisher -> cell */
        memcpy(&cell->payload, &local, sizeof(L2Update));
        atomic_fetch_add_explicit(&cell->seqno, 1, memory_order_release);

        /* memcpy 2: cell -> subscriber arena */
        memcpy(&subscriber_arena->payload, &cell->payload, sizeof(L2Update));
        atomic_fetch_add_explicit(&subscriber_arena->seqno, 1, memory_order_release);
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    return (t1.tv_sec - t0.tv_sec) * 1000000000LL + (t1.tv_nsec - t0.tv_nsec);
}

/* Path 2: implicit zero_copy lowering — ONE memcpy per dispatch.
 *
 * Publisher constructs locally (build_payload); codegen lowers
 * publish(v) to claim()+memcpy(slot, &v)+commit. Subscriber
 * gets a view-into-slot, no second copy. */
static int64_t bench_one_memcpy(Slot *ring) {
    L2Update local;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (int i = 0; i < ITER_PER_ROUND; i++) {
        build_payload(&local, i);
        memcpy(&ring->payload, &local, sizeof(L2Update));
        atomic_fetch_add_explicit(&ring->seqno, 1, memory_order_release);
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    return (t1.tv_sec - t0.tv_sec) * 1000000000LL + (t1.tv_nsec - t0.tv_nsec);
}

/* Path 3: explicit zero_copy claim/commit — ZERO memcpys per dispatch.
 *
 * Publisher writes directly into the ring slot via the
 * slot-as-locus surface (`with topics.X.claim() as slot { ... }`).
 * No intermediate local-construct phase. */
static int64_t bench_zero_memcpy(Slot *ring) {
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (int i = 0; i < ITER_PER_ROUND; i++) {
        L2Update *slot = &ring->payload;
        slot->bid_px_hi = 100;
        slot->bid_px_lo = 200;
        slot->ask_px_hi = 300;
        slot->ask_px_lo = 400;
        slot->bid_sz_hi = 500;
        slot->bid_sz_lo = 600;
        slot->ask_sz_hi = 700;
        slot->ask_sz_lo = 800;
        slot->venue_ts  = 900;
        slot->recv_ts   = i;
        atomic_fetch_add_explicit(&ring->seqno, 1, memory_order_release);
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    return (t1.tv_sec - t0.tv_sec) * 1000000000LL + (t1.tv_nsec - t0.tv_nsec);
}

static int cmp_int64(const void *a, const void *b) {
    int64_t aa = *(const int64_t *)a;
    int64_t bb = *(const int64_t *)b;
    return (aa > bb) - (aa < bb);
}

static int64_t median(int64_t *xs, int n) {
    qsort(xs, n, sizeof(int64_t), cmp_int64);
    return xs[n / 2];
}

int main(void) {
    Slot *cell = aligned_alloc(64, sizeof(Slot));
    Slot *sub  = aligned_alloc(64, sizeof(Slot));
    Slot *ring = aligned_alloc(64, sizeof(Slot));
    memset(cell, 0, sizeof(*cell));
    memset(sub,  0, sizeof(*sub));
    memset(ring, 0, sizeof(*ring));

    int64_t r_m28b[N_ROUNDS], r_one[N_ROUNDS], r_zero[N_ROUNDS];

    /* Warm-up round (discarded) — get the caches hot, kick
     * the scheduler around, fault in the slot pages. */
    bench_m28b(cell, sub);
    bench_one_memcpy(ring);
    bench_zero_memcpy(ring);

    for (int r = 0; r < N_ROUNDS; r++) {
        r_m28b[r] = bench_m28b(cell, sub);
        r_one[r]  = bench_one_memcpy(ring);
        r_zero[r] = bench_zero_memcpy(ring);
    }

    int64_t m_m28b = median(r_m28b, N_ROUNDS);
    int64_t m_one  = median(r_one,  N_ROUNDS);
    int64_t m_zero = median(r_zero, N_ROUNDS);

    double ns_m28b = (double)m_m28b / ITER_PER_ROUND;
    double ns_one  = (double)m_one  / ITER_PER_ROUND;
    double ns_zero = (double)m_zero / ITER_PER_ROUND;

    printf("Form K2 zero-copy bench (median over %d rounds of %d iter)\n",
           N_ROUNDS, ITER_PER_ROUND);
    printf("  payload size: %zu bytes (L2Update-shaped, flat)\n",
           sizeof(L2Update));
    printf("\n");
    printf("  m28b (2 memcpy):    %7.2f ns/op    %7.2f M ops/sec\n",
           ns_m28b, 1000.0 / ns_m28b);
    printf("  one-memcpy:         %7.2f ns/op    %7.2f M ops/sec    (%.2fx vs m28b)\n",
           ns_one, 1000.0 / ns_one, ns_m28b / ns_one);
    printf("  zero-memcpy:        %7.2f ns/op    %7.2f M ops/sec    (%.2fx vs m28b, %.2fx vs one-memcpy)\n",
           ns_zero, 1000.0 / ns_zero, ns_m28b / ns_zero, ns_one / ns_zero);
    printf("\n");
    printf("  delta (m28b - zero-memcpy): %.2f ns/op  =  %.1f cycles saved\n",
           ns_m28b - ns_zero, (ns_m28b - ns_zero) * 3.0);  /* ~3 GHz */

    free(cell);
    free(sub);
    free(ring);
    return 0;
}
