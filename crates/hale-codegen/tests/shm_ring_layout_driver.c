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
/* A1 zero-copy write: reserve a slot, write directly, commit the length. */
extern void *lotus_bus_reserve_shm_ring_layout(const char *subject, uint64_t max);
extern int lotus_bus_commit_shm_ring_layout(const char *subject, uint64_t len);

/* Native LotusRing (LRSRNG1) producer API — the dogfood test has a
 * layout-`slots` consumer read the very ring this native producer writes. */
typedef enum {
    LOTUS_SHM_OVERFLOW_BLOCK = 0,
    LOTUS_SHM_OVERFLOW_DROP = 1,
    LOTUS_SHM_OVERFLOW_FAIL = 2,
} lotus_shm_overflow_policy_t;
typedef struct lotus_shm_ring lotus_shm_ring_t;  /* opaque */
extern lotus_shm_ring_t *lotus_shm_ring_open(const char *name,
                                             uint64_t slot_size,
                                             uint64_t slot_count,
                                             lotus_shm_overflow_policy_t policy);
extern void *lotus_shm_ring_claim(lotus_shm_ring_t *ring);
extern void lotus_shm_ring_commit(lotus_shm_ring_t *ring);
extern void lotus_shm_ring_close(lotus_shm_ring_t *ring);
#define LOTUS_RING_MAGIC 0x4C5253524E4731ULL  /* "LRSRNG1" */

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

/* ---- raw (BytesView) consumer (value_size == 0) ----
 * The runtime hands a `{ void* src; int64_t epoch }` view by value
 * over a Bytes-shaped blob (`[i64 len][payload]`). We decode a
 * heterogeneous record: an i64 `kind` discriminator at payload offset
 * 0 selects a 16-byte (kind 1) or 24-byte (kind 2) record. */
typedef struct { void *src; int64_t epoch; } RawView;
static _Atomic int g_raw_count = 0;
static int64_t g_raw_kind[MAX_RECV];
static int64_t g_raw_len[MAX_RECV];
static int64_t g_raw_a[MAX_RECV];
static int64_t g_raw_b[MAX_RECV];   /* only meaningful for kind 2 */

static void on_raw(void *self, RawView v) {
    (void)self;
    char *blob = (char *)v.src;
    int64_t len = *(int64_t *)blob;        /* blob prefix */
    char *p = blob + sizeof(int64_t);       /* record payload */
    int i = atomic_fetch_add_explicit(&g_raw_count, 1, memory_order_acq_rel);
    if (i < MAX_RECV) {
        int64_t kind = 0, a = 0, b = 0;  /* memcpy: payload fields are
                                          * byte-aligned, not 8-aligned */
        memcpy(&kind, p + 0, sizeof(int64_t));
        if (len >= 16) memcpy(&a, p + 8, sizeof(int64_t));
        if (len >= 24) memcpy(&b, p + 16, sizeof(int64_t));
        g_raw_len[i] = len;
        g_raw_kind[i] = kind;
        g_raw_a[i] = a;
        g_raw_b[i] = b;
    }
}

static void build_desc(uint64_t desc[33]) {
    memset(desc, 0, 33 * sizeof(uint64_t));  /* [16..20]=0 → byte_records */
    desc[0]  = MAGIC;        desc[1]  = 1;            /* magic, has_magic */
    desc[2]  = VERSION_OFF;  desc[3]  = VERSION_WIDTH;
    desc[4]  = VERSION_VAL;  desc[5]  = 1;            /* version_expect, has_version */
    desc[6]  = BUFSZ_OFF;    desc[7]  = BUFSZ_WIDTH;  desc[8] = 1; /* has_buffer_size */
    desc[9]  = DATA_AT;
    desc[10] = CURSOR_OFF;
    desc[11] = LEN_WIDTH;
    desc[12] = ALIGN;
    desc[13] = PAD_SENTINEL; desc[14] = 1;            /* has_pad_sentinel */
    desc[15] = sizeof(Payload);                        /* value_size */
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
    uint64_t desc[33] = {0};
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
    uint64_t desc[33] = {0};
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

/* ---- hostile / edge-case foreign producers (hardening, 2026-06-08) ---- */

/* A foreign producer that advertises a `buffer_size` that is NOT a
 * multiple of `align` (8). The consumer's open_layout must reject the
 * attach (cap % align != 0) rather than read records whose header could
 * land in (cap - len_prefix_width, cap). On rejection the
 * register-subscriber path _exit(1)s with a diagnostic, so this driver
 * exits non-zero *without* an OOB. If the consumer wrongly accepted it,
 * we reach the BUG line and exit 0 (the test then fails). */
static int run_bad_bufsize(const char *name) {
    uint64_t capacity = 4094;  /* not a multiple of ALIGN (8) */
    size_t total = DATA_AT + (size_t)capacity;
    int fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    if (fd < 0) { fprintf(stderr, "shm_open: %s\n", strerror(errno)); return 2; }
    if (ftruncate(fd, (off_t)total) != 0) { shm_unlink(name); return 2; }
    void *map = mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) { shm_unlink(name); return 2; }
    char *base = (char *)map;
    *(uint64_t *)(base + 0) = MAGIC;
    *(uint32_t *)(base + VERSION_OFF) = (uint32_t)VERSION_VAL;
    *(uint32_t *)(base + BUFSZ_OFF) = (uint32_t)capacity;
    atomic_store_explicit((_Atomic uint64_t *)(base + CURSOR_OFF), 0,
                          memory_order_release);
    uint64_t desc[33] = {0};
    build_desc(desc);
    /* Expected: open_layout rejects (cap % align != 0) → _exit(1). */
    lotus_bus_register_subscriber_shm_ring_layout(
        "foreign.ticks", name, desc, NULL, on_record);
    fprintf(stderr, "BUG: consumer accepted a non-align-multiple buffer_size\n");
    munmap(map, total); close(fd); shm_unlink(name);
    return 0;  /* not rejected → test fails on exit==0 */
}

/* A foreign producer that frames a record whose `len` is SHORTER than
 * the bound payload's value_size, positioned so the handler reading
 * value_size bytes would run past the data region. The consumer must
 * detect len != value_size and resync (drop it), never dispatch it.
 * Two good records precede it; the consumer must receive exactly those
 * two and not OOB-read the short one. */
static int run_short_record(const char *name) {
    uint64_t capacity = 64;  /* multiple of ALIGN(8); data region [128,192) */
    size_t total = DATA_AT + (size_t)capacity;
    int fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    if (fd < 0) { fprintf(stderr, "shm_open: %s\n", strerror(errno)); return 2; }
    if (ftruncate(fd, (off_t)total) != 0) { shm_unlink(name); return 2; }
    void *map = mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) { shm_unlink(name); return 2; }
    char *base = (char *)map;
    *(uint64_t *)(base + 0) = MAGIC;
    *(uint32_t *)(base + VERSION_OFF) = (uint32_t)VERSION_VAL;
    *(uint32_t *)(base + BUFSZ_OFF) = (uint32_t)capacity;
    _Atomic uint64_t *cursor = (_Atomic uint64_t *)(base + CURSOR_OFF);
    atomic_store_explicit(cursor, 0, memory_order_release);

    uint64_t desc[33] = {0};
    build_desc(desc);
    lotus_bus_register_subscriber_shm_ring_layout(
        "foreign.ticks", name, desc, NULL, on_record);
    struct timespec warmup = {0, 5 * 1000 * 1000};
    nanosleep(&warmup, NULL);

    uint64_t step = align_up(LEN_WIDTH + sizeof(Payload), ALIGN);  /* 24 */
    uint64_t local = 0;
    /* two conforming records at pos 0 and 24 */
    for (int i = 0; i < 2; i++) {
        uint64_t off = DATA_AT + local;
        *(uint32_t *)(base + off) = (uint32_t)sizeof(Payload);
        Payload p = { .seq_id = i + 1, .value = (int64_t)(i + 1) * 7 };
        memcpy(base + off + LEN_WIDTH, &p, sizeof(Payload));
        local += step;
        atomic_store_explicit(cursor, local, memory_order_release);
    }
    /* short record at pos 48: len = 8 (< sizeof(Payload)=16). Its own
     * bytes fit ([52,60) within [.,64)), but a handler reading 16 bytes
     * would read [180,196) — past the data region end (192) → OOB. */
    {
        uint64_t off = DATA_AT + local;  /* 128 + 48 */
        *(uint32_t *)(base + off) = (uint32_t)8;
        unsigned char tiny[8] = {1,2,3,4,5,6,7,8};
        memcpy(base + off + LEN_WIDTH, tiny, sizeof(tiny));
        local += align_up(LEN_WIDTH + 8, ALIGN);  /* 16 → local 64 */
        atomic_store_explicit(cursor, local, memory_order_release);
    }
    /* drain window */
    for (int s = 0; s < 500; s++) {
        struct timespec ts = {0, 1000 * 1000};
        nanosleep(&ts, NULL);
        if (atomic_load_explicit(&g_recv_count, memory_order_acquire) >= 2) break;
    }
    int got = atomic_load_explicit(&g_recv_count, memory_order_acquire);
    munmap(map, total); close(fd); shm_unlink(name);
    if (got != 2) {
        fprintf(stderr, "expected exactly 2 conforming records, got %d "
                "(short record must be resynced, not dispatched)\n", got);
        return 3;
    }
    return 0;
}

/* A conforming foreign ring with a u64 length prefix (len_prefix_width
 * == align == 8). Round-trips records through the 8-byte len path. */
static int run_u64_lenprefix(const char *name) {
    enum { LW8 = 8 };
    uint64_t capacity = 4096;
    size_t total = DATA_AT + (size_t)capacity;
    int fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    if (fd < 0) { fprintf(stderr, "shm_open: %s\n", strerror(errno)); return 2; }
    if (ftruncate(fd, (off_t)total) != 0) { shm_unlink(name); return 2; }
    void *map = mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) { shm_unlink(name); return 2; }
    char *base = (char *)map;
    *(uint64_t *)(base + 0) = MAGIC;
    *(uint32_t *)(base + VERSION_OFF) = (uint32_t)VERSION_VAL;
    *(uint32_t *)(base + BUFSZ_OFF) = (uint32_t)capacity;
    _Atomic uint64_t *cursor = (_Atomic uint64_t *)(base + CURSOR_OFF);
    atomic_store_explicit(cursor, 0, memory_order_release);

    uint64_t desc[33] = {0};
    build_desc(desc);
    desc[11] = LW8;   /* len_prefix_width = 8 (u64) */
    lotus_bus_register_subscriber_shm_ring_layout(
        "foreign.ticks", name, desc, NULL, on_record);
    struct timespec warmup = {0, 5 * 1000 * 1000};
    nanosleep(&warmup, NULL);

    int n = 32;
    uint64_t step = align_up(LW8 + sizeof(Payload), ALIGN);
    uint64_t local = 0;
    for (int i = 0; i < n; i++) {
        uint64_t off = DATA_AT + local;
        *(uint64_t *)(base + off) = (uint64_t)sizeof(Payload);  /* u64 len */
        Payload p = { .seq_id = i + 1, .value = (int64_t)(i + 1) * 7 };
        memcpy(base + off + LW8, &p, sizeof(Payload));
        local += step;
        atomic_store_explicit(cursor, local, memory_order_release);
    }
    for (int s = 0; s < 1000; s++) {
        struct timespec ts = {0, 1000 * 1000};
        nanosleep(&ts, NULL);
        if (atomic_load_explicit(&g_recv_count, memory_order_acquire) >= n) break;
    }
    int got = atomic_load_explicit(&g_recv_count, memory_order_acquire);
    munmap(map, total); close(fd); shm_unlink(name);
    if (got != n) {
        fprintf(stderr, "u64-lenprefix: expected %d, got %d\n", n, got);
        return 3;
    }
    for (int i = 0; i < n; i++) {
        if (g_recv[i].seq_id != i + 1 || g_recv[i].value != (int64_t)(i + 1) * 7) {
            fprintf(stderr, "u64-lenprefix record %d mismatch\n", i);
            return 3;
        }
    }
    return 0;
}

/* Raw / heterogeneous foreign ring: records of two different sizes,
 * tagged by an i64 `kind` discriminator at payload offset 0. A single
 * raw (value_size == 0) subscriber receives a BytesView per record and
 * decodes both shapes — the path for real mixed-record external rings. */
static int run_heterogeneous(const char *name) {
    uint64_t capacity = 4096;
    size_t total = DATA_AT + (size_t)capacity;
    int fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    if (fd < 0) { fprintf(stderr, "shm_open: %s\n", strerror(errno)); return 2; }
    if (ftruncate(fd, (off_t)total) != 0) { shm_unlink(name); return 2; }
    void *map = mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) { shm_unlink(name); return 2; }
    char *base = (char *)map;
    *(uint64_t *)(base + 0) = MAGIC;
    *(uint32_t *)(base + VERSION_OFF) = (uint32_t)VERSION_VAL;
    *(uint32_t *)(base + BUFSZ_OFF) = (uint32_t)capacity;
    _Atomic uint64_t *cursor = (_Atomic uint64_t *)(base + CURSOR_OFF);
    atomic_store_explicit(cursor, 0, memory_order_release);

    uint64_t desc[33] = {0};
    build_desc(desc);
    desc[15] = 0;  /* value_size = 0 → raw BytesView path */
    lotus_bus_register_subscriber_shm_ring_layout(
        "foreign.ticks", name, desc, NULL, (void (*)(void *, void *))on_raw);
    struct timespec warmup = {0, 5 * 1000 * 1000};
    nanosleep(&warmup, NULL);

    int n = 8;
    uint64_t local = 0;
    for (int i = 0; i < n; i++) {
        int64_t kind = (i % 2) + 1;                 /* 1 or 2 */
        uint64_t plen = (kind == 1) ? 16 : 24;      /* differently sized */
        uint64_t off = DATA_AT + local;
        *(uint32_t *)(base + off) = (uint32_t)plen;
        char *p = base + off + LEN_WIDTH;
        int64_t a = (int64_t)(100 + i), b = (int64_t)(200 + i);
        memcpy(p + 0, &kind, sizeof(int64_t));
        memcpy(p + 8, &a, sizeof(int64_t));          /* a */
        if (kind == 2) memcpy(p + 16, &b, sizeof(int64_t));  /* b */
        local += align_up(LEN_WIDTH + plen, ALIGN);
        atomic_store_explicit(cursor, local, memory_order_release);
    }
    for (int s = 0; s < 1000; s++) {
        struct timespec ts = {0, 1000 * 1000};
        nanosleep(&ts, NULL);
        if (atomic_load_explicit(&g_raw_count, memory_order_acquire) >= n) break;
    }
    int got = atomic_load_explicit(&g_raw_count, memory_order_acquire);
    munmap(map, total); close(fd); shm_unlink(name);
    if (got != n) {
        fprintf(stderr, "heterogeneous: expected %d records, got %d\n", n, got);
        return 3;
    }
    for (int i = 0; i < n; i++) {
        int64_t exp_kind = (i % 2) + 1;
        int64_t exp_len = (exp_kind == 1) ? 16 : 24;
        if (g_raw_kind[i] != exp_kind || g_raw_len[i] != exp_len ||
            g_raw_a[i] != 100 + i ||
            (exp_kind == 2 && g_raw_b[i] != 200 + i)) {
            fprintf(stderr,
                    "heterogeneous record %d mismatch: kind=%lld len=%lld "
                    "a=%lld b=%lld\n",
                    i, (long long)g_raw_kind[i], (long long)g_raw_len[i],
                    (long long)g_raw_a[i], (long long)g_raw_b[i]);
            return 3;
        }
    }
    return 0;
}

/* External heterogeneous producer (no in-process consumer): creates the
 * ring, then writes differently-sized records for a SEPARATE process
 * (e.g. a Hale BytesView subscriber) to consume. Creates + writes the
 * header, waits for the consumer to attach, writes N records, waits for
 * it to drain, then unlinks. Used by the cross-process Hale-consumer
 * end-to-end test. */
static int run_produce_external(const char *name) {
    uint64_t capacity = 4096;
    size_t total = DATA_AT + (size_t)capacity;
    int fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    if (fd < 0) { fprintf(stderr, "shm_open: %s\n", strerror(errno)); return 2; }
    if (ftruncate(fd, (off_t)total) != 0) { shm_unlink(name); return 2; }
    void *map = mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) { shm_unlink(name); return 2; }
    char *base = (char *)map;
    *(uint64_t *)(base + 0) = MAGIC;
    *(uint32_t *)(base + VERSION_OFF) = (uint32_t)VERSION_VAL;
    *(uint32_t *)(base + BUFSZ_OFF) = (uint32_t)capacity;
    _Atomic uint64_t *cursor = (_Atomic uint64_t *)(base + CURSOR_OFF);
    atomic_store_explicit(cursor, 0, memory_order_release);

    /* Give the consumer time to attach + park at cursor 0 before we
     * publish (it reads from the live cursor, no replay). */
    struct timespec attach_wait = {0, 250 * 1000 * 1000};  /* 250ms */
    nanosleep(&attach_wait, NULL);

    int n = 6;
    uint64_t local = 0;
    for (int i = 0; i < n; i++) {
        int64_t kind = (i % 2) + 1;
        uint64_t plen = (kind == 1) ? 16 : 24;
        uint64_t off = DATA_AT + local;
        *(uint32_t *)(base + off) = (uint32_t)plen;
        char *p = base + off + LEN_WIDTH;
        int64_t a = (int64_t)(100 + i), b = (int64_t)(200 + i);
        memcpy(p + 0, &kind, sizeof(int64_t));
        memcpy(p + 8, &a, sizeof(int64_t));
        if (kind == 2) memcpy(p + 16, &b, sizeof(int64_t));
        local += align_up(LEN_WIDTH + plen, ALIGN);
        atomic_store_explicit(cursor, local, memory_order_release);
        struct timespec pace = {0, 5 * 1000 * 1000};  /* 5ms */
        nanosleep(&pace, NULL);
    }
    /* Let the consumer finish reading before we unlink. */
    struct timespec drain = {0, 400 * 1000 * 1000};  /* 400ms */
    nanosleep(&drain, NULL);
    munmap(map, total); close(fd); shm_unlink(name);
    return 0;
}

/* record_header external producer (#5, fast-protocol-I/O). Writes
 * ws-fast-shaped records: a fixed 32-byte header (len@0:u32, kind@4:u8
 * — 0 Data, 1 Padding) then the payload, stride = 32 + align8(len).
 * Records never straddle the wrap: when one won't fit before the
 * boundary, a kind==1 pad record fills the remainder. No in-process
 * consumer — a separate Hale process binds `layout:` with
 * record_header_bytes/pad_field/recheck and reads the i64 payload.
 * capacity 272 = 6*40 + 32: six 40-byte records then an exactly-32-byte
 * pad header at the tail, so the kind==1 pad path is exercised every
 * wrap. Paced 2ms/record (20x the reader's 100us poll) so it never
 * laps. */
#define RH_BYTES 32
#define RH_LEN_OFF 0
#define RH_KIND_OFF 4
static int run_produce_record_header(const char *name) {
    uint64_t capacity = 272;  /* multiple of ALIGN(8); 6*40 + 32 pad */
    size_t total = DATA_AT + (size_t)capacity;
    int fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    if (fd < 0) { fprintf(stderr, "shm_open: %s\n", strerror(errno)); return 2; }
    if (ftruncate(fd, (off_t)total) != 0) { shm_unlink(name); return 2; }
    void *map = mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) { shm_unlink(name); return 2; }
    char *base = (char *)map;
    *(uint64_t *)(base + 0) = MAGIC;
    *(uint32_t *)(base + VERSION_OFF) = (uint32_t)VERSION_VAL;
    *(uint32_t *)(base + BUFSZ_OFF) = (uint32_t)capacity;
    _Atomic uint64_t *cursor = (_Atomic uint64_t *)(base + CURSOR_OFF);
    atomic_store_explicit(cursor, 0, memory_order_release);

    struct timespec attach_wait = {0, 250 * 1000 * 1000};  /* 250ms */
    nanosleep(&attach_wait, NULL);

    int n = 40;
    uint64_t plen = 8;  /* one i64 payload */
    uint64_t step = RH_BYTES + align_up(plen, ALIGN);  /* 40 */
    uint64_t local = 0;
    for (int i = 0; i < n; i++) {
        if ((local % capacity) + step > capacity) {
            /* Tail pad: a kind==1 header fills the remainder to the wrap. */
            uint64_t off = DATA_AT + (local % capacity);
            *(uint32_t *)(base + off + RH_LEN_OFF) = 0;
            *(uint8_t *)(base + off + RH_KIND_OFF) = 1;  /* Padding */
            uint64_t rem = capacity - (local % capacity);
            local += rem;
            atomic_store_explicit(cursor, local, memory_order_release);
            struct timespec p = {0, 2 * 1000 * 1000};
            nanosleep(&p, NULL);
        }
        uint64_t off = DATA_AT + (local % capacity);
        *(uint32_t *)(base + off + RH_LEN_OFF) = (uint32_t)plen;
        *(uint8_t *)(base + off + RH_KIND_OFF) = 0;       /* Data */
        /* In-band header fields (ws-fast shape): seq@8, kernel_ns@16,
         * user_ns@24 — surfaced to the consumer via std::shm::last_record_*. */
        *(uint64_t *)(base + off + 8)  = (uint64_t)(i + 1);
        *(uint64_t *)(base + off + 16) = (uint64_t)((i + 1) * 1000);
        *(uint64_t *)(base + off + 24) = (uint64_t)((i + 1) * 1000 + 7);
        int64_t val = (int64_t)(i + 1) * 7;
        memcpy(base + off + RH_BYTES, &val, sizeof(val));  /* payload @ 32 */
        local += step;
        atomic_store_explicit(cursor, local, memory_order_release);
        struct timespec p = {0, 2 * 1000 * 1000};
        nanosleep(&p, NULL);
    }
    struct timespec drain = {0, 400 * 1000 * 1000};
    nanosleep(&drain, NULL);
    munmap(map, total); close(fd); shm_unlink(name);
    return 0;
}

/* Dogfood: the native LotusRing (LRSRNG1) slot ring, read through the
 * `ring_layout` abstraction. The NATIVE producer (lotus_shm_ring_open +
 * claim/commit) writes the ring; a layout-`slots` consumer attaches with
 * a descriptor that *describes our own native format* and reads the same
 * records. Proves the abstraction covers the in-house ring. */
static int run_lotus_dogfood(const char *name) {
    uint64_t slot_count = 16;
    lotus_shm_ring_t *ring = lotus_shm_ring_open(
        name, sizeof(Payload), slot_count, LOTUS_SHM_OVERFLOW_DROP);
    if (!ring) {
        fprintf(stderr, "native lotus_shm_ring_open: %s\n", strerror(errno));
        return 2;
    }

    /* A `ring_layout LotusRing` descriptor for the native LRSRNG1 header:
     * magic@0, slot_size@8, slot_count@16, seqno@24, slots@128. framing =
     * slots; geometry read from the header; cursor = the published seqno. */
    uint64_t desc[33] = {0};
    desc[0] = LOTUS_RING_MAGIC;
    desc[1] = 1;                  /* has_magic */
    desc[9] = 128;                /* data_at (header is two cache lines) */
    desc[10] = 24;               /* cursor = seqno offset */
    desc[15] = sizeof(Payload);  /* value_size (typed payload) */
    desc[16] = 1;                /* framing = slots */
    desc[17] = 8; desc[18] = 8;  /* slot_size  @ 8,  u64 */
    desc[19] = 16; desc[20] = 8; /* slot_count @ 16, u64 */
    lotus_bus_register_subscriber_shm_ring_layout(
        "lotus.dogfood", name, desc, NULL, on_record);

    struct timespec warmup = {0, 5 * 1000 * 1000};
    nanosleep(&warmup, NULL);

    int n = 8;
    for (int i = 0; i < n; i++) {
        Payload *slot = (Payload *)lotus_shm_ring_claim(ring);
        slot->seq_id = i + 1;
        slot->value = (int64_t)(i + 1) * 7;
        lotus_shm_ring_commit(ring);
        struct timespec pace = {0, 2 * 1000 * 1000};
        nanosleep(&pace, NULL);
    }
    for (int s = 0; s < 1000; s++) {
        struct timespec ts = {0, 1000 * 1000};
        nanosleep(&ts, NULL);
        if (atomic_load_explicit(&g_recv_count, memory_order_acquire) >= n) break;
    }
    int got = atomic_load_explicit(&g_recv_count, memory_order_acquire);
    lotus_shm_ring_close(ring);  /* free the native producer handle */
    if (got != n) {
        fprintf(stderr, "lotus_dogfood: expected %d records, got %d\n", n, got);
        return 3;
    }
    for (int i = 0; i < n; i++) {
        if (g_recv[i].seq_id != i + 1 || g_recv[i].value != (int64_t)(i + 1) * 7) {
            fprintf(stderr, "lotus_dogfood record %d mismatch: seq=%lld val=%lld\n",
                    i, (long long)g_recv[i].seq_id, (long long)g_recv[i].value);
            return 3;
        }
    }
    shm_unlink(name);
    return 0;
}

/* External native (LRSRNG1) producer, no in-process consumer — for the
 * Hale end-to-end dogfood: a separate Hale process binds `layout:
 * LotusRing` and reads this native ring. Creates the ring, waits for the
 * Hale consumer to attach, publishes, waits for it to drain, closes. */
static int run_produce_native_external(const char *name) {
    uint64_t slot_count = 16;
    lotus_shm_ring_t *ring = lotus_shm_ring_open(
        name, sizeof(Payload), slot_count, LOTUS_SHM_OVERFLOW_DROP);
    if (!ring) {
        fprintf(stderr, "native lotus_shm_ring_open: %s\n", strerror(errno));
        return 2;
    }
    struct timespec attach = {0, 250 * 1000 * 1000};  /* 250ms to attach */
    nanosleep(&attach, NULL);
    int n = 6;
    for (int i = 0; i < n; i++) {
        Payload *slot = (Payload *)lotus_shm_ring_claim(ring);
        slot->seq_id = i + 1;
        slot->value = (int64_t)(i + 1) * 7;
        lotus_shm_ring_commit(ring);
        struct timespec pace = {0, 5 * 1000 * 1000};
        nanosleep(&pace, NULL);
    }
    struct timespec drain = {0, 400 * 1000 * 1000};
    nanosleep(&drain, NULL);
    lotus_shm_ring_close(ring);
    return 0;
}

/* A1 zero-copy write: reserve a max-sized slot, write differently-sized
 * records DIRECTLY into the mapped ring (no intermediate buffer), commit
 * the actual length. A raw (value_size == 0) consumer reads them back —
 * proving reserve/commit frames variable-length records correctly. */
static int run_reserve_commit(const char *name) {
    uint64_t desc[33] = {0};
    build_desc(desc);
    desc[15] = 0;  /* value_size = 0 → raw consumer (variable records) */
    lotus_bus_register_shm_ring_layout("foreign.ticks", name, desc, 4096);
    lotus_bus_register_subscriber_shm_ring_layout(
        "foreign.ticks", name, desc, NULL, (void (*)(void *, void *))on_raw);
    struct timespec warmup = {0, 5 * 1000 * 1000};
    nanosleep(&warmup, NULL);

    int n = 6;
    for (int i = 0; i < n; i++) {
        int64_t kind = (i % 2) + 1;
        uint64_t plen = (kind == 1) ? 16 : 24;
        /* Reserve room for the largest record (24), write only `plen`. */
        char *slot = (char *)lotus_bus_reserve_shm_ring_layout("foreign.ticks", 24);
        if (!slot) {
            fprintf(stderr, "reserve %d failed\n", i);
            return 2;
        }
        int64_t a = (int64_t)(100 + i), b = (int64_t)(200 + i);
        memcpy(slot + 0, &kind, sizeof(int64_t));
        memcpy(slot + 8, &a, sizeof(int64_t));
        if (kind == 2) memcpy(slot + 16, &b, sizeof(int64_t));
        if (lotus_bus_commit_shm_ring_layout("foreign.ticks", plen) != 0) {
            fprintf(stderr, "commit %d failed\n", i);
            return 2;
        }
        struct timespec pace = {0, 5 * 1000 * 1000};
        nanosleep(&pace, NULL);
    }
    for (int s = 0; s < 2000; s++) {
        if (atomic_load_explicit(&g_raw_count, memory_order_acquire) >= n) break;
        struct timespec ts = {0, 1000 * 1000};
        nanosleep(&ts, NULL);
    }
    int got = atomic_load_explicit(&g_raw_count, memory_order_acquire);
    if (got != n) {
        fprintf(stderr, "reserve_commit: expected %d records, got %d\n", n, got);
        return 3;
    }
    for (int i = 0; i < n; i++) {
        int64_t exp_kind = (i % 2) + 1;
        int64_t exp_len = (exp_kind == 1) ? 16 : 24;
        if (g_raw_kind[i] != exp_kind || g_raw_len[i] != exp_len ||
            g_raw_a[i] != 100 + i ||
            (exp_kind == 2 && g_raw_b[i] != 200 + i)) {
            fprintf(stderr, "reserve_commit record %d mismatch\n", i);
            return 3;
        }
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr,
                "usage: %s <roundtrip|wrap|producer|producer_wrap|"
                "bad_bufsize|short_record|u64_lenprefix|heterogeneous|"
                "produce_external|lotus_dogfood|produce_native_external|"
                "reserve_commit> <shm_name>\n",
                argv[0]);
        return 1;
    }
    if (strcmp(argv[1], "reserve_commit") == 0) {
        return run_reserve_commit(argv[2]);
    }
    if (strcmp(argv[1], "lotus_dogfood") == 0) {
        return run_lotus_dogfood(argv[2]);
    }
    if (strcmp(argv[1], "produce_native_external") == 0) {
        return run_produce_native_external(argv[2]);
    }
    if (strcmp(argv[1], "produce_external") == 0) {
        return run_produce_external(argv[2]);
    }
    if (strcmp(argv[1], "produce_record_header") == 0) {
        return run_produce_record_header(argv[2]);
    }
    if (strcmp(argv[1], "heterogeneous") == 0) {
        return run_heterogeneous(argv[2]);
    }
    if (strcmp(argv[1], "bad_bufsize") == 0) {
        return run_bad_bufsize(argv[2]);
    }
    if (strcmp(argv[1], "short_record") == 0) {
        return run_short_record(argv[2]);
    }
    if (strcmp(argv[1], "u64_lenprefix") == 0) {
        return run_u64_lenprefix(argv[2]);
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
