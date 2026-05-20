/*
 * Form K4c (2026-05-20) — SHM ring publish-side end-to-end
 * driver. Attaches an Aperio-published SHM ring as a subscriber
 * and validates the payloads.
 *
 * Used by the shm_ring_publish.rs harness: an Aperio publisher
 * (compiled by build_executable + spawned) calls
 * `Topic <- TickPayload { ... }` repeatedly through the
 * lotus_bus_publish_shm_ring path; this driver attaches the same
 * named SHM ring and reads the values back, exiting 0 on
 * validation success.
 */

#define _GNU_SOURCE
#include <errno.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef struct lotus_shm_ring_t lotus_shm_ring_t;

/* Form K7 (2026-05-20): open takes an overflow policy. Reader-
 * only driver — use DROP (the reader never calls claim). */
typedef enum {
    LOTUS_SHM_OVERFLOW_BLOCK = 0,
    LOTUS_SHM_OVERFLOW_DROP = 1,
    LOTUS_SHM_OVERFLOW_FAIL = 2,
} lotus_shm_overflow_policy_t;

extern lotus_shm_ring_t *lotus_shm_ring_open(const char *name,
                                              uint64_t slot_size,
                                              uint64_t slot_count,
                                              lotus_shm_overflow_policy_t policy);
extern void lotus_shm_ring_close(lotus_shm_ring_t *ring);
extern uint64_t lotus_shm_ring_published(lotus_shm_ring_t *ring);
extern void *lotus_shm_ring_read_slot(lotus_shm_ring_t *ring,
                                       uint64_t seqno);

/* Must match the Aperio-side `type Tick { px: Int; sz: Int; }`
 * declared in the test's source. Both fields are i64 (Aperio's
 * default Int is 64-bit). */
typedef struct {
    int64_t px;
    int64_t sz;
} Tick;

int main(int argc, char **argv) {
    if (argc < 4) {
        fprintf(stderr,
                "usage: %s <shm_name> <slot_count> <expected_count>\n",
                argv[0]);
        return 64;
    }
    const char *name = argv[1];
    uint64_t slot_count = (uint64_t)strtoull(argv[2], NULL, 10);
    uint64_t expected = (uint64_t)strtoull(argv[3], NULL, 10);

    /* Retry-attach: the Aperio publisher races to create the
     * ring; we wait for it. */
    lotus_shm_ring_t *ring = NULL;
    for (int tries = 0; tries < 200 && !ring; tries++) {
        ring = lotus_shm_ring_open(name, sizeof(Tick), slot_count, LOTUS_SHM_OVERFLOW_DROP);
        if (!ring) {
            struct timespec ts = {0, 5 * 1000 * 1000};
            nanosleep(&ts, NULL);
        }
    }
    if (!ring) {
        fprintf(stderr, "reader: attach failed: %s\n", strerror(errno));
        return 2;
    }

    /* Poll until we've seen `expected` publishes, validating
     * each. The Aperio publisher writes Tick{px: i+1, sz:
     * (i+1)*7} for i = 0..expected-1. */
    uint64_t last_seen = 0;
    for (int tries = 0; tries < 1000 && last_seen < expected; tries++) {
        uint64_t pub = lotus_shm_ring_published(ring);
        while (last_seen < pub && last_seen < expected) {
            last_seen++;
            Tick *p = (Tick *)lotus_shm_ring_read_slot(ring, last_seen);
            if (!p) {
                fprintf(stderr,
                        "reader: read_slot(%llu) NULL (published=%llu)\n",
                        (unsigned long long)last_seen,
                        (unsigned long long)pub);
                lotus_shm_ring_close(ring);
                return 3;
            }
            int64_t want_px = (int64_t)last_seen;
            int64_t want_sz = (int64_t)last_seen * 7;
            if (p->px != want_px || p->sz != want_sz) {
                fprintf(stderr,
                        "reader: mismatch at seqno %llu: "
                        "got px=%lld sz=%lld, want px=%lld sz=%lld\n",
                        (unsigned long long)last_seen,
                        (long long)p->px, (long long)p->sz,
                        (long long)want_px, (long long)want_sz);
                lotus_shm_ring_close(ring);
                return 4;
            }
        }
        if (last_seen >= expected) break;
        struct timespec ts = {0, 5 * 1000 * 1000};
        nanosleep(&ts, NULL);
    }
    if (last_seen < expected) {
        fprintf(stderr,
                "reader: timed out at last_seen=%llu (wanted %llu)\n",
                (unsigned long long)last_seen,
                (unsigned long long)expected);
        lotus_shm_ring_close(ring);
        return 5;
    }
    lotus_shm_ring_close(ring);
    return 0;
}
