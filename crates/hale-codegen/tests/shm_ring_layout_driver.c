/*
 * Proposal B (2026-06-06) — foreign-layout SHM ring C driver test.
 *
 * Exercises the read-only `byte_records` consumer path
 * (lotus_bus_register_subscriber_shm_ring_layout + the layout
 * reader thread) against a foreign-layout producer that this driver
 * plays itself.
 *
 * The driver is BOTH sides over one POSIX SHM segment:
 *   - Producer: shm_open(O_CREAT|O_EXCL), writes the header
 *     (magic @0, version @8:u32, buffer_size @12:u32), then writes
 *     `[u32 len][payload]` records 8-aligned with a pad_sentinel
 *     tail-pad at the wrap, release-publishing a monotonic byte
 *     cursor @64 after each record.
 *   - Consumer: lotus_bus_register_subscriber_shm_ring_layout
 *     attaches read-only, spawns the reader thread, and the handler
 *     records each delivered payload into a global array.
 *
 * Modes (argv[1]):
 *   roundtrip : capacity holds all N records — no wrap, no lap;
 *               asserts exact in-order delivery (deterministic).
 *   wrap      : small capacity forces pad-at-wrap; producer paces
 *               (1ms/record vs the reader's 100us poll, a 10x
 *               margin) so the consumer never laps; asserts exact
 *               in-order delivery of all N (exercises the pad
 *               branch + modular stepping).
 *
 * Exits 0 on success, non-zero with a one-line stderr diagnostic.
 */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

extern void lotus_bus_register_subscriber_shm_ring_layout(
    const char *subject,
    const char *shm_name,
    const uint64_t *desc_words,
    void *self_ptr,
    void (*handler_fn)(void *self, void *slot));

/* Proposal B M3a — producer-side C ABI. */
extern void lotus_bus_register_shm_ring_layout(
    const char *subject,
    const char *shm_name,
    const uint64_t *desc_words,
    uint64_t capacity);
extern int lotus_bus_publish_shm_ring_layout(
    const char *subject,
    const void *value,
    uint64_t value_size);

/* Layout constants — mirror the `ForeignRing` ring_layout used in the
 * Rust-side tests. */
#define MAGIC          0x52494E47464D5431ULL
#define VERSION_OFF    8
#define VERSION_WIDTH  4
#define VERSION_VAL    1
#define BUFSZ_OFF      12
#define BUFSZ_WIDTH    4
#define DATA_AT        128
#define CURSOR_OFF     64
#define LEN_WIDTH      4
#define ALIGN          8
#define PAD_SENTINEL   0xFFFFFFFFULL

typedef struct {
    int64_t seq_id;
    int64_t value;
} Payload;

static uint64_t align_up(uint64_t v, uint64_t a) {
    return (v + a - 1) & ~(a - 1);
}

/* ---- consumer side ---- */
#define MAX_RECV 256
static _Atomic int g_recv_count = 0;
static Payload g_recv[MAX_RECV];

static void on_record(void *self, void *slot) {
    (void)self;
    int i = atomic_fetch_add_explicit(&g_recv_count, 1, memory_order_acq_rel);
    if (i < MAX_RECV) {
        memcpy(&g_recv[i], slot, sizeof(Payload));
    }
}

static void build_desc(uint64_t desc[16]) {
    memset(desc, 0, 16 * sizeof(uint64_t));
    desc[0]  = MAGIC;        desc[1]  = 1;            /* magic, has_magic */
    desc[2]  = VERSION_OFF;  desc[3]  = VERSION_WIDTH;
    desc[4]  = VERSION_VAL;  desc[5]  = 1;            /* version_expect, has_version */
    desc[6]  = BUFSZ_OFF;    desc[7]  = BUFSZ_WIDTH;  desc[8] = 1; /* has_buffer_size */
    desc[9]  = DATA_AT;
    desc[10] = CURSOR_OFF;
    desc[11] = LEN_WIDTH;
    desc[12] = ALIGN;
    desc[13] = PAD_SENTINEL; desc[14] = 1;            /* has_pad_sentinel */
}

/* ---- producer side ---- */
static int run(const char *name, uint64_t capacity, int n, int pace_us) {
    size_t total = DATA_AT + (size_t)capacity;
    int fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    if (fd < 0) {
        fprintf(stderr, "producer shm_open: %s\n", strerror(errno));
        return 2;
    }
    if (ftruncate(fd, (off_t)total) != 0) {
        fprintf(stderr, "ftruncate: %s\n", strerror(errno));
        shm_unlink(name);
        return 2;
    }
    void *map = mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        fprintf(stderr, "producer mmap: %s\n", strerror(errno));
        shm_unlink(name);
        return 2;
    }
    char *base = (char *)map;

    /* Header. */
    *(uint64_t *)(base + 0) = MAGIC;
    *(uint32_t *)(base + VERSION_OFF) = (uint32_t)VERSION_VAL;
    *(uint32_t *)(base + BUFSZ_OFF) = (uint32_t)capacity;
    _Atomic uint64_t *cursor = (_Atomic uint64_t *)(base + CURSOR_OFF);
    atomic_store_explicit(cursor, 0, memory_order_release);

    /* Consumer attaches after the header is valid; it starts reading
     * from the current cursor (0), so it sees every record below. */
    uint64_t desc[16];
    build_desc(desc);
    lotus_bus_register_subscriber_shm_ring_layout(
        "foreign.ticks", name, desc, NULL, on_record);
    /* Give the reader a moment to attach + park at cursor 0. */
    struct timespec warmup = {0, 5 * 1000 * 1000};  /* 5ms */
    nanosleep(&warmup, NULL);

    uint64_t local = 0;
    uint64_t rec = LEN_WIDTH + sizeof(Payload);
    uint64_t step = align_up(rec, ALIGN);
    for (int i = 0; i < n; i++) {
        /* Tail pad if this record would straddle the wrap. */
        if ((local % capacity) + step > capacity) {
            uint64_t off = DATA_AT + (local % capacity);
            *(uint32_t *)(base + off) = (uint32_t)PAD_SENTINEL;
            uint64_t rem = capacity - (local % capacity);
            local += rem;
            atomic_store_explicit(cursor, local, memory_order_release);
            if (pace_us > 0) {
                struct timespec ts = {0, (long)pace_us * 1000};
                nanosleep(&ts, NULL);
            }
        }
        uint64_t off = DATA_AT + (local % capacity);
        *(uint32_t *)(base + off) = (uint32_t)sizeof(Payload);
        Payload p = { .seq_id = i + 1, .value = (int64_t)(i + 1) * 7 };
        memcpy(base + off + LEN_WIDTH, &p, sizeof(Payload));
        local += step;
        atomic_store_explicit(cursor, local, memory_order_release);
        if (pace_us > 0) {
            struct timespec ts = {0, (long)pace_us * 1000};
            nanosleep(&ts, NULL);
        }
    }

    /* Wait for the consumer to drain (bounded). */
    for (int spins = 0; spins < 2000; spins++) {
        if (atomic_load_explicit(&g_recv_count, memory_order_acquire) >= n) {
            break;
        }
        struct timespec ts = {0, 1000 * 1000};  /* 1ms */
        nanosleep(&ts, NULL);
    }

    int got = atomic_load_explicit(&g_recv_count, memory_order_acquire);
    int rc = 0;
    if (got != n) {
        fprintf(stderr, "expected %d records, got %d\n", n, got);
        rc = 3;
    } else {
        for (int i = 0; i < n; i++) {
            if (g_recv[i].seq_id != i + 1 ||
                g_recv[i].value != (int64_t)(i + 1) * 7) {
                fprintf(stderr,
                        "record %d mismatch: seq_id=%lld value=%lld\n",
                        i, (long long)g_recv[i].seq_id,
                        (long long)g_recv[i].value);
                rc = 3;
                break;
            }
        }
    }

    /* atexit teardown (in lotus_shm_ring.c) joins the reader + closes
     * the consumer attach. The producer owns the segment: unlink it. */
    munmap(map, total);
    close(fd);
    shm_unlink(name);
    return rc;
}

/* Proposal B M3a — round-trip through the PRODUCER C ABI: register a
 * producer (creates the ring), register a consumer (attaches), publish
 * N records via lotus_bus_publish_shm_ring_layout, validate the
 * consumer reads them back in order. Exercises producer + consumer
 * framing symmetry end to end. `pace_us > 0` (with a small capacity)
 * forces the pad-at-wrap path on the producer side too. */
static int run_producer(const char *name, uint64_t capacity, int n, int pace_us) {
    uint64_t desc[16];
    build_desc(desc);

    /* Producer creates + owns the ring. */
    lotus_bus_register_shm_ring_layout("foreign.ticks", name, desc, capacity);
    /* Consumer attaches read-only and parks at cursor 0. */
    lotus_bus_register_subscriber_shm_ring_layout(
        "foreign.ticks", name, desc, NULL, on_record);
    struct timespec warmup = {0, 5 * 1000 * 1000};  /* 5ms */
    nanosleep(&warmup, NULL);

    for (int i = 0; i < n; i++) {
        Payload p = { .seq_id = i + 1, .value = (int64_t)(i + 1) * 7 };
        if (lotus_bus_publish_shm_ring_layout(
                "foreign.ticks", &p, sizeof(p)) != 0) {
            fprintf(stderr, "publish %d failed\n", i);
            return 2;
        }
        if (pace_us > 0) {
            struct timespec ts = {0, (long)pace_us * 1000};
            nanosleep(&ts, NULL);
        }
    }

    for (int spins = 0; spins < 2000; spins++) {
        if (atomic_load_explicit(&g_recv_count, memory_order_acquire) >= n) {
            break;
        }
        struct timespec ts = {0, 1000 * 1000};
        nanosleep(&ts, NULL);
    }
    int got = atomic_load_explicit(&g_recv_count, memory_order_acquire);
    if (got != n) {
        fprintf(stderr, "expected %d records, got %d\n", n, got);
        return 3;
    }
    for (int i = 0; i < n; i++) {
        if (g_recv[i].seq_id != i + 1 ||
            g_recv[i].value != (int64_t)(i + 1) * 7) {
            fprintf(stderr, "record %d mismatch: seq_id=%lld value=%lld\n",
                    i, (long long)g_recv[i].seq_id,
                    (long long)g_recv[i].value);
            return 3;
        }
    }
    /* atexit teardown joins the reader + closes/unlinks both handles. */
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr,
                "usage: %s <roundtrip|wrap|producer|producer_wrap> <shm_name>\n",
                argv[0]);
        return 1;
    }
    if (strcmp(argv[1], "roundtrip") == 0) {
        /* capacity 4096 holds 64 * 24 = 1536 bytes — no wrap. */
        return run(argv[2], 4096, 64, 0);
    }
    if (strcmp(argv[1], "wrap") == 0) {
        /* capacity 256 wraps every ~10 records; pace 1ms (10x the
         * reader's 100us poll) so the consumer never laps. */
        return run(argv[2], 256, 40, 1000);
    }
    if (strcmp(argv[1], "producer") == 0) {
        return run_producer(argv[2], 4096, 64, 0);
    }
    if (strcmp(argv[1], "producer_wrap") == 0) {
        return run_producer(argv[2], 256, 40, 1000);
    }
    fprintf(stderr, "unknown mode `%s`\n", argv[1]);
    return 1;
}
