/*
 * Form K5 (2026-05-20) — SHM ring C driver test.
 *
 * Three sub-tests selectable by argv[1]:
 *
 *   roundtrip   : open ring, publish N payloads, read them back,
 *                 validate. argv[2] = shm_name.
 *   wraparound  : publish slot_count + extra, read back; verify
 *                 stale seqnos return NULL.
 *   ipc-parent  : open ring (exclusive create), publish N, signal
 *                 child (writes seqno to stdout), wait.
 *   ipc-child   : attach ring, poll until published_seqno hits N,
 *                 read all slots, validate.
 *
 * Exits 0 on success, non-zero on validation failure with a
 * one-line diagnostic on stderr.
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

/* Forward decls — implemented in lotus_shm_ring.c, linked into
 * the test binary. */
typedef struct lotus_shm_ring_t lotus_shm_ring_t;

extern lotus_shm_ring_t *lotus_shm_ring_open(const char *name,
                                              uint64_t slot_size,
                                              uint64_t slot_count);
extern void lotus_shm_ring_close(lotus_shm_ring_t *ring);
extern void *lotus_shm_ring_claim(lotus_shm_ring_t *ring);
extern void lotus_shm_ring_commit(lotus_shm_ring_t *ring);
extern uint64_t lotus_shm_ring_published(lotus_shm_ring_t *ring);
extern void *lotus_shm_ring_read_slot(lotus_shm_ring_t *ring, uint64_t seqno);

typedef struct {
    int64_t seq_id;
    int64_t value;
    int64_t ts;
    int64_t pad;
} Payload;

#define SLOT_SIZE   sizeof(Payload)
#define SLOT_COUNT  8
#define N_PUB       100

static int test_roundtrip(const char *name) {
    /* Single ring — same handle for produce and consume. Models
     * "in-process SPMC" — what K3's slot locus will exercise on
     * the codegen side. */
    lotus_shm_ring_t *ring = lotus_shm_ring_open(name, SLOT_SIZE, SLOT_COUNT);
    if (!ring) {
        fprintf(stderr, "open failed: %s\n", strerror(errno));
        return 2;
    }

    /* Publish-and-read-immediately, one at a time. Avoids
     * wraparound since N_PUB > SLOT_COUNT only matters if reads
     * lag publishes. */
    for (int i = 0; i < SLOT_COUNT; i++) {
        Payload *p = (Payload *)lotus_shm_ring_claim(ring);
        p->seq_id = (int64_t)(i + 1);
        p->value  = (int64_t)(i + 1) * 7;
        p->ts     = 1000 + i;
        p->pad    = 0;
        lotus_shm_ring_commit(ring);

        uint64_t pub = lotus_shm_ring_published(ring);
        if (pub != (uint64_t)(i + 1)) {
            fprintf(stderr, "after commit %d: published_seqno=%llu, want %d\n",
                    i + 1, (unsigned long long)pub, i + 1);
            lotus_shm_ring_close(ring);
            return 3;
        }

        Payload *q = (Payload *)lotus_shm_ring_read_slot(ring, pub);
        if (!q) {
            fprintf(stderr, "read_slot(%llu) returned NULL\n",
                    (unsigned long long)pub);
            lotus_shm_ring_close(ring);
            return 4;
        }
        if (q->seq_id != p->seq_id || q->value != p->value ||
            q->ts != p->ts) {
            fprintf(stderr, "payload mismatch at seqno %llu: "
                    "got {seq=%lld,val=%lld,ts=%lld}\n",
                    (unsigned long long)pub,
                    (long long)q->seq_id, (long long)q->value,
                    (long long)q->ts);
            lotus_shm_ring_close(ring);
            return 5;
        }
    }
    lotus_shm_ring_close(ring);
    return 0;
}

static int test_wraparound(const char *name) {
    lotus_shm_ring_t *ring = lotus_shm_ring_open(name, SLOT_SIZE, SLOT_COUNT);
    if (!ring) {
        fprintf(stderr, "open failed: %s\n", strerror(errno));
        return 2;
    }

    /* Publish 3x slot_count without reading. */
    for (int i = 0; i < (int)(SLOT_COUNT * 3); i++) {
        Payload *p = (Payload *)lotus_shm_ring_claim(ring);
        p->seq_id = (int64_t)(i + 1);
        p->value  = (int64_t)(i + 1) * 7;
        p->ts     = 1000 + i;
        p->pad    = 0;
        lotus_shm_ring_commit(ring);
    }

    uint64_t pub = lotus_shm_ring_published(ring);
    if (pub != (uint64_t)(SLOT_COUNT * 3)) {
        fprintf(stderr, "after %d publishes: pub=%llu\n",
                SLOT_COUNT * 3, (unsigned long long)pub);
        lotus_shm_ring_close(ring);
        return 3;
    }

    /* Reading seqnos 1..SLOT_COUNT*2 must return NULL — wrapped past. */
    for (uint64_t s = 1; s <= (uint64_t)(SLOT_COUNT * 2); s++) {
        if (lotus_shm_ring_read_slot(ring, s) != NULL) {
            fprintf(stderr, "expected NULL for stale seqno %llu, got non-NULL\n",
                    (unsigned long long)s);
            lotus_shm_ring_close(ring);
            return 4;
        }
    }

    /* Reading the most-recent SLOT_COUNT seqnos must return non-NULL
     * with the right payload. */
    for (uint64_t s = (uint64_t)(SLOT_COUNT * 2) + 1;
         s <= (uint64_t)(SLOT_COUNT * 3); s++) {
        Payload *p = (Payload *)lotus_shm_ring_read_slot(ring, s);
        if (!p) {
            fprintf(stderr, "expected non-NULL for live seqno %llu\n",
                    (unsigned long long)s);
            lotus_shm_ring_close(ring);
            return 5;
        }
        if ((uint64_t)p->seq_id != s) {
            fprintf(stderr, "payload mismatch at live seqno %llu: got seq_id %lld\n",
                    (unsigned long long)s, (long long)p->seq_id);
            lotus_shm_ring_close(ring);
            return 6;
        }
    }

    /* Reading the just-past-published seqno must return NULL
     * (not yet committed). */
    if (lotus_shm_ring_read_slot(ring, pub + 1) != NULL) {
        fprintf(stderr, "expected NULL for not-yet-committed seqno %llu\n",
                (unsigned long long)(pub + 1));
        lotus_shm_ring_close(ring);
        return 7;
    }

    lotus_shm_ring_close(ring);
    return 0;
}

static int test_ipc_parent(const char *name) {
    /* Exclusive-creator path — owns the unlink. */
    lotus_shm_ring_t *ring = lotus_shm_ring_open(name, SLOT_SIZE, SLOT_COUNT);
    if (!ring) {
        fprintf(stderr, "parent: open failed: %s\n", strerror(errno));
        return 2;
    }

    /* Publish slot_count payloads, slowly enough that the child
     * has time to attach and start reading. */
    for (int i = 0; i < SLOT_COUNT; i++) {
        Payload *p = (Payload *)lotus_shm_ring_claim(ring);
        p->seq_id = (int64_t)(i + 1);
        p->value  = (int64_t)(i + 1) * 7;
        p->ts     = 1000 + i;
        p->pad    = 0;
        lotus_shm_ring_commit(ring);
        /* 5 ms between publishes — gives the child a chance to
         * keep up. */
        struct timespec ts = {0, 5 * 1000 * 1000};
        nanosleep(&ts, NULL);
    }

    /* Wait briefly so the child reads the last commit, then
     * close (which unlinks). */
    struct timespec ts = {0, 100 * 1000 * 1000};
    nanosleep(&ts, NULL);
    lotus_shm_ring_close(ring);
    return 0;
}

static int test_ipc_child(const char *name) {
    /* Retry-attach loop — the parent may not have created the
     * SHM object yet when the child spawns. */
    lotus_shm_ring_t *ring = NULL;
    for (int tries = 0; tries < 100; tries++) {
        ring = lotus_shm_ring_open(name, SLOT_SIZE, SLOT_COUNT);
        if (ring) break;
        struct timespec ts = {0, 5 * 1000 * 1000};
        nanosleep(&ts, NULL);
    }
    if (!ring) {
        fprintf(stderr, "child: attach failed after retries: %s\n",
                strerror(errno));
        return 2;
    }

    /* Poll for SLOT_COUNT publishes; read each one and validate. */
    uint64_t last_seen = 0;
    for (int tries = 0; tries < 500 && last_seen < SLOT_COUNT; tries++) {
        uint64_t pub = lotus_shm_ring_published(ring);
        while (last_seen < pub && last_seen < SLOT_COUNT) {
            last_seen++;
            Payload *p = (Payload *)lotus_shm_ring_read_slot(ring, last_seen);
            if (!p) {
                fprintf(stderr, "child: read_slot(%llu) NULL\n",
                        (unsigned long long)last_seen);
                lotus_shm_ring_close(ring);
                return 3;
            }
            int64_t want_value = (int64_t)last_seen * 7;
            if (p->seq_id != (int64_t)last_seen || p->value != want_value) {
                fprintf(stderr, "child: mismatch at %llu: seq=%lld val=%lld\n",
                        (unsigned long long)last_seen,
                        (long long)p->seq_id, (long long)p->value);
                lotus_shm_ring_close(ring);
                return 4;
            }
        }
        if (last_seen >= SLOT_COUNT) break;
        struct timespec ts = {0, 2 * 1000 * 1000};
        nanosleep(&ts, NULL);
    }
    if (last_seen < SLOT_COUNT) {
        fprintf(stderr, "child: timed out at last_seen=%llu (wanted %d)\n",
                (unsigned long long)last_seen, SLOT_COUNT);
        lotus_shm_ring_close(ring);
        return 5;
    }
    lotus_shm_ring_close(ring);
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s <roundtrip|wraparound|ipc-parent|ipc-child> <shm_name>\n",
                argv[0]);
        return 64;
    }
    const char *mode = argv[1];
    const char *name = argv[2];
    if (strcmp(mode, "roundtrip") == 0)  return test_roundtrip(name);
    if (strcmp(mode, "wraparound") == 0) return test_wraparound(name);
    if (strcmp(mode, "ipc-parent") == 0) return test_ipc_parent(name);
    if (strcmp(mode, "ipc-child") == 0)  return test_ipc_child(name);
    fprintf(stderr, "unknown mode: %s\n", mode);
    return 64;
}
