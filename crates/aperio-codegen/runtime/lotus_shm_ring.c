/*
 * Lotus SHM ring — Form K5 substrate for zero-copy bus payload
 * routing.
 *
 * One ring = a POSIX SHM object holding a fixed-layout header
 * followed by N equal-sized slots. The publisher claims the next
 * slot, writes its payload directly into ring memory, then commits
 * by atomic-release-incrementing the published seqno. Subscribers
 * read the seqno (acquire) and access the slot at that index.
 *
 * Layout (in shared memory):
 *
 *   ┌────────────────────────────────┐ offset 0
 *   │ lotus_shm_ring_header_t (64 B) │
 *   ├────────────────────────────────┤ offset 64
 *   │ slot[0]   (slot_size bytes)    │
 *   ├────────────────────────────────┤
 *   │ slot[1]                        │
 *   ├────────────────────────────────┤
 *   │ ...                            │
 *   ├────────────────────────────────┤
 *   │ slot[slot_count - 1]           │
 *   └────────────────────────────────┘
 *
 * v1 scope: SINGLE PRODUCER, multi-consumer. claim() never fails
 * — it always returns the next-slot pointer. Slow consumers risk
 * having their slot overwritten by the next wrap; Form K6's
 * stamped-epoch view guard catches this on the read side.
 *
 * The "ring full" / back-pressure failure path is reserved in
 * the Aperio-side fallible signature (per [[slot-locus-design]])
 * but the v1 implementation never triggers it.
 *
 * Multi-producer (CAS-based claim) and timeout / non-blocking
 * claim modes are post-v1.
 */

#define _GNU_SOURCE
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdatomic.h>
#include <fcntl.h>
#include <pthread.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
#include <errno.h>

/* Magic bumped at K7 (2026-05-20) when the header layout grew
 * the consumer_seqno cache line. Attaches against a different
 * magic value are rejected by lotus_shm_ring_open's validation
 * branch (catches binaries pinned to different ABI generations
 * trying to share a ring). */
#define LOTUS_SHM_RING_MAGIC 0x4C5253524E4731ULL  /* "LRSRNG1" */

/* Publisher-side overflow policy. Stored per-process on the
 * handle (NOT in SHM — different attachers may have different
 * preferences). ABI value is part of the codegen interface;
 * keep in sync with aperio_syntax::ast::ShmRingOverflow.
 * `runtime_tag()`. */
typedef enum {
    LOTUS_SHM_OVERFLOW_BLOCK = 0,
    LOTUS_SHM_OVERFLOW_DROP = 1,
    LOTUS_SHM_OVERFLOW_FAIL = 2,
} lotus_shm_overflow_policy_t;

/* In-SHM header — TWO cache lines (128 bytes). The producer's
 * seqno and the consumer's seqno live on separate cache lines so
 * neither side's writes pingpong the other side's reads. Field
 * order is part of the on-disk layout; do not reorder without
 * bumping LOTUS_SHM_RING_MAGIC. */
typedef struct {
    /* Cache line 0 — owned by the publisher. */
    uint64_t magic;
    uint64_t slot_size;
    uint64_t slot_count;
    _Atomic uint64_t seqno;        /* monotonic published count */
    uint64_t _pad0[4];              /* round to 64 B */
    /* Cache line 1 — owned by the consumer. Form K7 (2026-05-20). */
    _Atomic uint64_t consumer_seqno; /* last fully-consumed seqno */
    uint64_t _pad1[7];              /* round to 64 B */
} lotus_shm_ring_header_t;

_Static_assert(sizeof(lotus_shm_ring_header_t) == 128,
               "ring header must be exactly two cache lines");
_Static_assert(offsetof(lotus_shm_ring_header_t, seqno) == 24,
               "seqno offset is part of the on-disk layout");
_Static_assert(offsetof(lotus_shm_ring_header_t, consumer_seqno) == 64,
               "consumer_seqno must live on its own cache line");

/* Per-process handle. NOT in the SHM region — each process keeps
 * its own, including the publisher's overflow policy choice. */
typedef struct {
    lotus_shm_ring_header_t *header;
    void *slots_base;       /* points just past the header */
    int fd;                 /* shm_open fd; kept for shm_unlink at close */
    size_t mapped_size;
    char shm_name[96];      /* for shm_unlink at close */
    int owns_unlink;        /* 1 if this handle should shm_unlink on close */
    lotus_shm_overflow_policy_t overflow_policy;  /* K7 */
} lotus_shm_ring_t;

/* Open or attach to a SHM ring.
 *
 * `name` is a POSIX SHM object name (must start with '/' on Linux).
 *
 * If the SHM object does not exist, it is created and zero-
 * initialized with the requested slot_size/slot_count, AND the
 * caller becomes the "owner" — close() will shm_unlink it.
 *
 * If it already exists, attach: the requested slot_size and
 * slot_count must match the existing header (otherwise NULL is
 * returned). Caller is NOT the owner; close() will NOT unlink.
 *
 * Returns NULL on error. errno is set.
 */
lotus_shm_ring_t *lotus_shm_ring_open(const char *name,
                                      uint64_t slot_size,
                                      uint64_t slot_count,
                                      lotus_shm_overflow_policy_t policy) {
    if (!name || slot_size == 0 || slot_count == 0) {
        errno = EINVAL;
        return NULL;
    }
    if (strlen(name) + 1 > sizeof(((lotus_shm_ring_t *)0)->shm_name)) {
        errno = ENAMETOOLONG;
        return NULL;
    }

    size_t total = sizeof(lotus_shm_ring_header_t) + (size_t)slot_size * slot_count;

    /* Try create-exclusive first; if it exists, attach. */
    int owns = 0;
    int fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    if (fd >= 0) {
        owns = 1;
        if (ftruncate(fd, (off_t)total) != 0) {
            int save = errno;
            close(fd);
            shm_unlink(name);
            errno = save;
            return NULL;
        }
    } else if (errno == EEXIST) {
        fd = shm_open(name, O_RDWR, 0600);
        if (fd < 0) return NULL;
    } else {
        return NULL;
    }

    void *map = mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        int save = errno;
        close(fd);
        if (owns) shm_unlink(name);
        errno = save;
        return NULL;
    }

    lotus_shm_ring_header_t *hdr = (lotus_shm_ring_header_t *)map;

    if (owns) {
        /* Fresh creation — initialize the header. ftruncate already
         * zeroed the pages, but write the magic + sizes explicitly. */
        hdr->magic = LOTUS_SHM_RING_MAGIC;
        hdr->slot_size = slot_size;
        hdr->slot_count = slot_count;
        atomic_store_explicit(&hdr->seqno, 0, memory_order_release);
        atomic_store_explicit(&hdr->consumer_seqno, 0,
                                memory_order_release);
    } else {
        /* Existing ring — validate the header matches our request. */
        if (hdr->magic != LOTUS_SHM_RING_MAGIC ||
            hdr->slot_size != slot_size ||
            hdr->slot_count != slot_count) {
            munmap(map, total);
            close(fd);
            errno = EBADF;
            return NULL;
        }
    }

    lotus_shm_ring_t *ring = (lotus_shm_ring_t *)calloc(1, sizeof(*ring));
    if (!ring) {
        munmap(map, total);
        close(fd);
        if (owns) shm_unlink(name);
        errno = ENOMEM;
        return NULL;
    }
    ring->header = hdr;
    ring->slots_base = (char *)map + sizeof(lotus_shm_ring_header_t);
    ring->fd = fd;
    ring->mapped_size = total;
    ring->owns_unlink = owns;
    ring->overflow_policy = policy;
    strncpy(ring->shm_name, name, sizeof(ring->shm_name) - 1);
    return ring;
}

/* Close the ring. Unmaps; closes the fd; if this handle owns the
 * SHM object (created it), unlinks it from the SHM namespace. */
void lotus_shm_ring_close(lotus_shm_ring_t *ring) {
    if (!ring) return;
    if (ring->header) munmap(ring->header, ring->mapped_size);
    if (ring->fd >= 0) close(ring->fd);
    if (ring->owns_unlink) shm_unlink(ring->shm_name);
    free(ring);
}

/* Publisher: claim the next slot.
 *
 * Form K7 (2026-05-20): back-pressure semantics determined by
 * the ring's overflow_policy (set at lotus_shm_ring_open time):
 *
 *   - DROP: never fails. Always returns slots[(seqno + 1) %
 *     count]. May overwrite unread slots — silent data loss if
 *     consumers are slow. Pre-K7 behavior; preserved for stale-
 *     is-worthless feeds.
 *   - BLOCK: when the ring is full (published - consumer_seqno
 *     >= slot_count), spin with 100us nanosleeps until the
 *     consumer catches up, then return the slot. No timeout —
 *     deadlocks if the consumer dies.
 *   - FAIL: when the ring is full, returns NULL. Caller (the
 *     publish_shm_ring wrapper) panics with a clear diagnostic.
 *     Post-K7 work will route this through fallible-`<-` for
 *     graceful caller-side handling.
 *
 * The DROP fast path costs one atomic load + arithmetic. The
 * BLOCK/FAIL paths add a second atomic load of consumer_seqno,
 * which lives on its own cache line — so the load only stalls
 * on a true overflow (cache line was last written by the
 * consumer's update, which only matters when we're checking
 * back-pressure anyway).
 *
 * Returns the slot's contents are whatever the previous claim of
 * this index left behind — publisher must overwrite, not assume
 * zero.
 */
void *lotus_shm_ring_claim(lotus_shm_ring_t *ring) {
    uint64_t cur = atomic_load_explicit(&ring->header->seqno,
                                         memory_order_relaxed);
    if (ring->overflow_policy != LOTUS_SHM_OVERFLOW_DROP) {
        uint64_t consumer = atomic_load_explicit(
            &ring->header->consumer_seqno, memory_order_acquire);
        uint64_t in_flight = cur - consumer;
        if (in_flight >= ring->header->slot_count) {
            if (ring->overflow_policy == LOTUS_SHM_OVERFLOW_FAIL) {
                return NULL;
            }
            /* BLOCK: nanosleep until the consumer makes progress. */
            for (;;) {
                struct timespec ts = {0, 100 * 1000};  /* 100us */
                nanosleep(&ts, NULL);
                consumer = atomic_load_explicit(
                    &ring->header->consumer_seqno,
                    memory_order_acquire);
                if (cur - consumer < ring->header->slot_count) break;
            }
        }
    }
    uint64_t idx = (cur + 1) % ring->header->slot_count;
    return (char *)ring->slots_base + idx * ring->header->slot_size;
}

/* Publisher: commit the most recent claim.
 *
 * Release-orders the slot writes before the seqno bump so a
 * subscriber that does an acquire-load and sees the new seqno is
 * guaranteed to see the published bytes.
 *
 * After commit, subscribers can read this seqno. The publisher
 * MUST NOT modify the slot further (subscribers may be reading).
 *
 * Pairs 1:1 with claim() — calling commit() without a preceding
 * claim() advances the seqno over an unwritten slot. Calling
 * claim() twice without a commit between leaks the first slot
 * (publisher discipline).
 */
void lotus_shm_ring_commit(lotus_shm_ring_t *ring) {
    atomic_fetch_add_explicit(&ring->header->seqno, 1, memory_order_release);
}

/* Subscriber: the most recently committed seqno (acquire load).
 *
 * Subscribers track their own last-read seqno; poll this to find
 * out what's available. Returns 0 if nothing has been published.
 */
uint64_t lotus_shm_ring_published(lotus_shm_ring_t *ring) {
    return atomic_load_explicit(&ring->header->seqno,
                                  memory_order_acquire);
}

/* Subscriber: get a pointer to the slot at the given seqno.
 *
 * Caller is responsible for having read the published seqno via
 * lotus_shm_ring_published() and confirming `seqno` ≤ published.
 *
 * Returns NULL if `seqno` is stale (publisher has wrapped past
 * it — slot_count or more publishes have happened since). The
 * Form K6 view-epoch guard wraps this so subscribers can't see
 * a torn read.
 *
 * Returns NULL if `seqno` is 0 (no slot 0 — seqno is 1-based;
 * 0 means "nothing has been committed").
 */
void *lotus_shm_ring_read_slot(lotus_shm_ring_t *ring, uint64_t seqno) {
    if (seqno == 0) return NULL;
    uint64_t published = atomic_load_explicit(&ring->header->seqno,
                                                memory_order_acquire);
    if (seqno > published) return NULL;  /* not yet committed */
    /* Stale: if the publisher has wrapped past this seqno (i.e.,
     * the slot has been re-claimed and possibly re-written), the
     * pointer would be racing with the publisher. Detect by
     * checking published - seqno >= slot_count. */
    if (published - seqno >= ring->header->slot_count) {
        return NULL;
    }
    uint64_t idx = seqno % ring->header->slot_count;
    return (char *)ring->slots_base + idx * ring->header->slot_size;
}

/* === Form K4c (2026-05-20) — bus router integration ====================
 *
 * Per-process subject->ring registry. Aperio's codegen emits a
 * register call per shm_ring binding into main's prelude, and
 * routes `Topic <- value` (Send stmt) publishes through the
 * registry. Single producer model: one binding per subject, one
 * ring per binding.
 *
 * Lookup is linear over the entry array. v1 expects a handful of
 * shm_ring bindings per binary (most apps have one or two
 * high-rate topics on shared rings); a small array beats hash
 * overhead at this scale.
 */

#define LOTUS_SHM_RING_MAX_BINDINGS 64
#define LOTUS_SHM_RING_MAX_SUBSCRIBERS 64

typedef struct {
    char subject[64];           /* matches lotus_bus_remote_entry's shape */
    lotus_shm_ring_t *ring;
    uint64_t slot_size;         /* mirrored from ring->header for memcpy */
} shm_ring_binding_t;

/* Forward decl — defined under the K6b subscriber section. */
typedef struct shm_ring_subscriber_t shm_ring_subscriber_t;

static shm_ring_binding_t g_shm_ring_bindings[LOTUS_SHM_RING_MAX_BINDINGS];
static int g_shm_ring_binding_count = 0;
static shm_ring_subscriber_t *
    g_shm_ring_subscribers[LOTUS_SHM_RING_MAX_SUBSCRIBERS];
static _Atomic int g_shm_ring_subscriber_count = 0;
static _Atomic int g_shm_ring_atexit_registered = 0;

/* Form K-cleanup (2026-05-20): process-exit teardown for all
 * shm_ring resources owned by this process. Registered via
 * atexit on first registration call (publisher or subscriber).
 *
 * Fixes the categorical leaks K4c/K6b shipped with:
 *   - subscriber pthreads were detached (no join) and runs of
 *     malloc'd subscriber state outlived their use
 *   - publisher-side SHM objects were never shm_unlink'd, so
 *     /dev/shm/ accumulated stale entries across process
 *     restarts
 *
 * After this hook runs at clean exit:
 *   - All subscriber reader threads are stopped + joined
 *   - All subscriber state is freed
 *   - All rings are munmap+closed
 *   - Rings created by THIS process are shm_unlink'd (the
 *     owns_unlink branch inside lotus_shm_ring_close)
 *
 * If the process exits via signal / _exit, atexit DOES NOT
 * fire and the SHM namespace entry persists until reboot or
 * manual shm_unlink. v1 accepts this; a future SIGINT/SIGTERM
 * handler can fold into this same teardown.
 */
static void shm_ring_atexit_cleanup(void);

static void ensure_shm_ring_atexit_registered(void) {
    int expected = 0;
    if (atomic_compare_exchange_strong_explicit(
            &g_shm_ring_atexit_registered,
            &expected,
            1,
            memory_order_acq_rel,
            memory_order_relaxed)) {
        atexit(shm_ring_atexit_cleanup);
    }
}

/* Codegen emits one call per shm_ring binding in main's prelude.
 * Idempotent across the process — registering the same subject
 * twice is a programmer bug (the typechecker catches duplicate
 * topic bindings); the runtime asserts on collision. */
void lotus_bus_register_shm_ring(const char *subject,
                                 uint64_t slot_size,
                                 uint64_t slot_count,
                                 const char *shm_name,
                                 int32_t overflow_policy) {
    ensure_shm_ring_atexit_registered();
    if (g_shm_ring_binding_count >= LOTUS_SHM_RING_MAX_BINDINGS) {
        fprintf(stderr,
                "lotus_bus_register_shm_ring: exceeded "
                "LOTUS_SHM_RING_MAX_BINDINGS (%d) — bump the cap "
                "or split into multiple binaries\n",
                LOTUS_SHM_RING_MAX_BINDINGS);
        _exit(1);
    }
    /* Reject duplicate subjects — should be caught by the
     * typechecker upstream (Form K4a + the existing "topic may
     * appear at most once across all bindings" rule), but runtime
     * assert is a defence-in-depth. */
    for (int i = 0; i < g_shm_ring_binding_count; i++) {
        if (strcmp(g_shm_ring_bindings[i].subject, subject) == 0) {
            fprintf(stderr,
                    "lotus_bus_register_shm_ring: duplicate "
                    "registration for subject `%s`\n",
                    subject);
            _exit(1);
        }
    }
    lotus_shm_ring_t *ring = lotus_shm_ring_open(
        shm_name, slot_size, slot_count,
        (lotus_shm_overflow_policy_t)overflow_policy);
    if (!ring) {
        fprintf(stderr,
                "lotus_bus_register_shm_ring(`%s`, %s): open failed: %s\n",
                subject, shm_name, strerror(errno));
        _exit(1);
    }
    shm_ring_binding_t *b = &g_shm_ring_bindings[g_shm_ring_binding_count++];
    strncpy(b->subject, subject, sizeof(b->subject) - 1);
    b->subject[sizeof(b->subject) - 1] = '\0';
    b->ring = ring;
    b->slot_size = slot_size;
}

/* Publish-side dispatch. Called from Aperio-codegen's lower_send
 * when the target topic is shm_ring-bound. The implicit one-
 * memcpy path: claim the next slot, copy the payload bytes into
 * it, commit. K2's bench measured this at ~7.3 ns/op.
 *
 * Returns 0 on success, -1 if the subject isn't registered
 * (programmer bug — codegen should never emit this call for an
 * unregistered subject, but defence-in-depth). */
int lotus_bus_publish_shm_ring(const char *subject,
                               const void *value,
                               uint64_t value_size) {
    for (int i = 0; i < g_shm_ring_binding_count; i++) {
        shm_ring_binding_t *b = &g_shm_ring_bindings[i];
        if (strcmp(b->subject, subject) != 0) continue;
        if (value_size != b->slot_size) {
            fprintf(stderr,
                    "lotus_bus_publish_shm_ring(`%s`): payload size %llu "
                    "doesn't match registered slot_size %llu — payload "
                    "type changed between codegen and link?\n",
                    subject,
                    (unsigned long long)value_size,
                    (unsigned long long)b->slot_size);
            _exit(1);
        }
        void *slot = lotus_shm_ring_claim(b->ring);
        if (!slot) {
            /* Form K7 (2026-05-20): FAIL policy fired. v1 panics
             * with a clear diagnostic — process exits non-zero so
             * supervisors see the back-pressure event instead of
             * silently losing data. Post-K7 work will route this
             * through fallible-`<-` so callers can address it
             * gracefully. */
            fprintf(stderr,
                    "lotus_bus_publish_shm_ring(`%s`): ring full and "
                    "`on_overflow: fail` policy is set — consumer not "
                    "draining fast enough. To handle gracefully, "
                    "switch to `on_overflow: block` (rate-match) or "
                    "`on_overflow: drop` (accept loss).\n",
                    subject);
            _exit(1);
        }
        memcpy(slot, value, value_size);
        lotus_shm_ring_commit(b->ring);
        return 0;
    }
    fprintf(stderr,
            "lotus_bus_publish_shm_ring(`%s`): no registration for "
            "this subject\n",
            subject);
    return -1;
}

/* === Form K6b (2026-05-20) — subscriber-side reader thread ============
 *
 * Each subscriber locus that has a `bus { subscribe Foo as on_foo; }`
 * declaration for a shm_ring-bound topic gets a dedicated reader
 * thread spawned at locus birth. The reader polls the ring's
 * published seqno; on each advance it calls
 * `handler_fn(locus_self, slot_ptr)` for every newly committed
 * slot.
 *
 * v1 simplifications:
 *   - Handler runs on the reader thread (NOT the cooperative
 *     scheduler). Documented constraint: shm_ring subscriber
 *     handlers must be thread-safe and not touch shared
 *     scheduler state. Users who need cooperative dispatch
 *     should use `unix(...)` instead.
 *   - No stamped-epoch staleness guard. Slow handlers risk
 *     reading torn slot bytes if the publisher wraps past
 *     during a read. v2 will add the F.30b-style guard.
 *   - Thread is daemon-detached at v1. Runs until process exit.
 *
 * Poll cadence: 100us sleep between empty polls. Burns minimal
 * CPU when idle; ~10-100us tail latency on receive. Tuneable
 * post-v1.
 */

struct shm_ring_subscriber_t {
    lotus_shm_ring_t *ring;
    void (*handler_fn)(void *self, void *slot);
    void *self_ptr;
    pthread_t thread;
    /* Form K-cleanup (2026-05-20): atexit hook sets this to 1
     * before pthread_join. Reader loop checks each iteration
     * and exits cleanly when set. Acquire-load matches the
     * release-store in the atexit hook so the reader sees
     * the signal promptly. */
    _Atomic int should_stop;
};

static void *shm_ring_reader_thread(void *arg) {
    shm_ring_subscriber_t *sub = (shm_ring_subscriber_t *)arg;
    uint64_t last_seen = 0;
    while (!atomic_load_explicit(&sub->should_stop,
                                  memory_order_acquire)) {
        uint64_t pub = lotus_shm_ring_published(sub->ring);
        if (pub > last_seen) {
            while (last_seen < pub) {
                last_seen++;
                void *slot = lotus_shm_ring_read_slot(sub->ring, last_seen);
                if (slot) {
                    sub->handler_fn(sub->self_ptr, slot);
                }
                /* slot == NULL means the publisher already wrapped
                 * past this seqno — the slot's contents are racing
                 * with a fresh publish. Skip silently at v1; the
                 * post-v1 epoch guard will surface this. */
            }
            /* Form K7 (2026-05-20): release-publish the consumer
             * cursor after the batch. Publisher's back-pressure
             * check (BLOCK/FAIL policies) reads this with acquire
             * semantics; consumer_seqno living on its own cache
             * line keeps the producer's `seqno` writes from
             * pingponging this update. */
            atomic_store_explicit(&sub->ring->header->consumer_seqno,
                                    last_seen, memory_order_release);
        } else {
            struct timespec ts = {0, 100 * 1000};  /* 100us */
            nanosleep(&ts, NULL);
        }
    }
    return NULL;
}

/* Codegen emits one call per shm_ring subscriber registration at
 * locus birth. self_ptr is the subscriber locus instance;
 * handler_fn has signature `void(void *self, void *slot)` —
 * codegen produces a shim that casts `slot` to the topic's
 * payload type pointer and invokes the user's `fn on_foo(p:
 * Payload)`. */
void lotus_bus_register_subscriber_shm_ring(const char *subject,
                                             uint64_t slot_size,
                                             uint64_t slot_count,
                                             const char *shm_name,
                                             void *self_ptr,
                                             void (*handler_fn)(void *self,
                                                                 void *slot)) {
    ensure_shm_ring_atexit_registered();

    /* Subscriber side never calls claim(), so the policy is
     * effectively irrelevant — use DROP (zero overhead in claim
     * if the subscriber accidentally publishes too). */
    lotus_shm_ring_t *ring = lotus_shm_ring_open(
        shm_name, slot_size, slot_count, LOTUS_SHM_OVERFLOW_DROP);
    if (!ring) {
        fprintf(stderr,
                "lotus_bus_register_subscriber_shm_ring(`%s`, `%s`): open "
                "failed: %s\n",
                subject, shm_name, strerror(errno));
        _exit(1);
    }

    shm_ring_subscriber_t *sub =
        (shm_ring_subscriber_t *)calloc(1, sizeof(*sub));
    if (!sub) {
        fprintf(stderr,
                "lotus_bus_register_subscriber_shm_ring(`%s`): calloc "
                "failed\n",
                subject);
        _exit(1);
    }
    sub->ring = ring;
    sub->handler_fn = handler_fn;
    sub->self_ptr = self_ptr;
    atomic_store_explicit(&sub->should_stop, 0, memory_order_relaxed);

    int rc = pthread_create(&sub->thread, NULL, shm_ring_reader_thread, sub);
    if (rc != 0) {
        fprintf(stderr,
                "lotus_bus_register_subscriber_shm_ring(`%s`): pthread_create "
                "failed: %d\n",
                subject, rc);
        free(sub);
        _exit(1);
    }
    /* Register for atexit cleanup. NOT pthread_detach'd at K6b:
     * the atexit hook signals should_stop and pthread_joins so
     * the reader thread + its calloc'd state are released
     * cleanly on a normal exit. */
    int idx = atomic_fetch_add_explicit(
        &g_shm_ring_subscriber_count, 1, memory_order_acq_rel);
    if (idx >= LOTUS_SHM_RING_MAX_SUBSCRIBERS) {
        fprintf(stderr,
                "lotus_bus_register_subscriber_shm_ring(`%s`): exceeded "
                "LOTUS_SHM_RING_MAX_SUBSCRIBERS (%d)\n",
                subject, LOTUS_SHM_RING_MAX_SUBSCRIBERS);
        _exit(1);
    }
    g_shm_ring_subscribers[idx] = sub;
}

/* Form K-cleanup (2026-05-20) — atexit teardown. Signals all
 * subscriber readers to stop, joins them, frees their state,
 * closes all rings (which shm_unlink's the creator-side ones).
 *
 * Two-pass on subscribers: first set should_stop on all, then
 * join + free. This lets all reader threads start winding down
 * in parallel instead of serially waiting for each. */
static void shm_ring_atexit_cleanup(void) {
    int n_subs = atomic_load_explicit(
        &g_shm_ring_subscriber_count, memory_order_acquire);
    if (n_subs > LOTUS_SHM_RING_MAX_SUBSCRIBERS) {
        /* Capped at register time; if we overflowed the counter
         * past the array we'd have _exit'd already, but guard
         * against UB in the atexit hook. */
        n_subs = LOTUS_SHM_RING_MAX_SUBSCRIBERS;
    }
    for (int i = 0; i < n_subs; i++) {
        shm_ring_subscriber_t *sub = g_shm_ring_subscribers[i];
        if (sub) {
            atomic_store_explicit(
                &sub->should_stop, 1, memory_order_release);
        }
    }
    for (int i = 0; i < n_subs; i++) {
        shm_ring_subscriber_t *sub = g_shm_ring_subscribers[i];
        if (sub) {
            pthread_join(sub->thread, NULL);
            if (sub->ring) {
                lotus_shm_ring_close(sub->ring);
            }
            free(sub);
            g_shm_ring_subscribers[i] = NULL;
        }
    }
    /* Publisher-side rings: close (and shm_unlink the
     * creator-owned ones). */
    for (int i = 0; i < g_shm_ring_binding_count; i++) {
        if (g_shm_ring_bindings[i].ring) {
            lotus_shm_ring_close(g_shm_ring_bindings[i].ring);
            g_shm_ring_bindings[i].ring = NULL;
        }
    }
}
