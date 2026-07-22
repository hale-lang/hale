/*
 * GH #244: exercise lotus_spsc_* from the runtime — the SPSC
 * observation ring over caller-provided memory. Built by
 * tests/spsc_ring.rs (clang, linked against lotus_arena.c) and
 * run once; asserts are in-process.
 *
 * Scenario: a 64 KiB "segment" on the heap holding one 64 B
 * descriptor + a 64-slot ring. A producer thread emits 10_000
 * records (seq in w0, seq^mask in w1); a consumer thread
 * snapshot-reads concurrently. Asserts:
 *   - every delivered record is internally consistent
 *     (w1 == w0 ^ mask — no torn 16 B records delivered),
 *   - delivered seqs are strictly increasing (no duplicates,
 *     no reordering),
 *   - delivered + overruns == produced (nothing vanishes
 *     unaccounted),
 *   - the final head equals the produce count (monotonic,
 *     never wrapped).
 */

#include <assert.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define RING_SLOTS 64
#define PRODUCE 10000
#define MASK 0x5A5A5A5A5A5A5A5AULL

void    lotus_spsc_init(void *desc, int64_t data_off,
                        int64_t tag_a, int64_t tag_b);
void    lotus_spsc_emit(void *seg, void *desc, int64_t slots,
                        int64_t w0, int64_t w1);
void    lotus_spsc_note_drop(void *desc);
int64_t lotus_spsc_read(const void *seg, const void *desc,
                        int64_t slots, int64_t *cursor,
                        int64_t *overruns, void *out,
                        int64_t max_records);

static char g_seg[65536];

static void *producer(void *arg) {
    (void)arg;
    for (uint64_t i = 0; i < PRODUCE; i++) {
        lotus_spsc_emit(g_seg, g_seg, RING_SLOTS,
                        (int64_t)i, (int64_t)(i ^ MASK));
    }
    return NULL;
}

int main(void) {
    lotus_spsc_init(g_seg, /*data_off=*/64, /*tag_a=*/7, /*tag_b=*/0);

    pthread_t p;
    if (pthread_create(&p, NULL, producer, NULL) != 0) {
        fprintf(stderr, "pthread_create failed\n");
        return 2;
    }

    int64_t cursor = 0, overruns = 0;
    uint64_t out[32 * 2];
    uint64_t delivered = 0;
    uint64_t last_seq = 0;
    int have_last = 0;
    while (delivered + (uint64_t)overruns < PRODUCE) {
        int64_t n = lotus_spsc_read(g_seg, g_seg, RING_SLOTS,
                                    &cursor, &overruns, out, 32);
        for (int64_t i = 0; i < n; i++) {
            uint64_t w0 = out[i * 2];
            uint64_t w1 = out[i * 2 + 1];
            if (w1 != (w0 ^ MASK)) {
                fprintf(stderr, "torn record: w0=%llu w1=%llu\n",
                        (unsigned long long)w0,
                        (unsigned long long)w1);
                return 1;
            }
            if (have_last && w0 <= last_seq) {
                fprintf(stderr, "non-monotonic seq %llu after %llu\n",
                        (unsigned long long)w0,
                        (unsigned long long)last_seq);
                return 1;
            }
            last_seq = w0;
            have_last = 1;
        }
        delivered += (uint64_t)n;
    }
    pthread_join(p, NULL);

    /* Drain the tail after the producer stopped. */
    for (;;) {
        int64_t n = lotus_spsc_read(g_seg, g_seg, RING_SLOTS,
                                    &cursor, &overruns, out, 32);
        if (n == 0) break;
        delivered += (uint64_t)n;
    }

    uint64_t head;
    memcpy(&head, g_seg + 8, sizeof(head));
    if (head != PRODUCE) {
        fprintf(stderr, "head=%llu, want %d\n",
                (unsigned long long)head, PRODUCE);
        return 1;
    }
    if (delivered + (uint64_t)overruns != PRODUCE) {
        fprintf(stderr,
                "accounting: delivered=%llu overruns=%lld != %d\n",
                (unsigned long long)delivered,
                (long long)overruns, PRODUCE);
        return 1;
    }
    printf("ok delivered=%llu overruns=%lld\n",
           (unsigned long long)delivered, (long long)overruns);
    return 0;
}
