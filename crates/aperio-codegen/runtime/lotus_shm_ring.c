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
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#include <errno.h>

#define LOTUS_SHM_RING_MAGIC 0x4C5253524E4730ULL  /* "LRSRNG0" */

/* In-SHM header — exactly 64 bytes (one cache line). Field order
 * is part of the on-disk layout; do not reorder without bumping
 * the magic. */
typedef struct {
    uint64_t magic;
    uint64_t slot_size;
    uint64_t slot_count;
    _Atomic uint64_t seqno;  /* monotonic; 0 = nothing committed */
    uint64_t _pad[4];        /* round to 64 B */
} lotus_shm_ring_header_t;

_Static_assert(sizeof(lotus_shm_ring_header_t) == 64,
               "ring header must be exactly one cache line");
_Static_assert(offsetof(lotus_shm_ring_header_t, seqno) == 24,
               "seqno offset is part of the on-disk layout");

/* Per-process handle. NOT in the SHM region — each process keeps
 * its own. */
typedef struct {
    lotus_shm_ring_header_t *header;
    void *slots_base;       /* points just past the header */
    int fd;                 /* shm_open fd; kept for shm_unlink at close */
    size_t mapped_size;
    char shm_name[96];      /* for shm_unlink at close */
    int owns_unlink;        /* 1 if this handle should shm_unlink on close */
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
                                      uint64_t slot_count) {
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
 * Returns a pointer to the slot the next commit() will publish.
 * The slot's contents are whatever the previous claim of this
 * index left behind — publisher must overwrite, not assume zero.
 *
 * v1 SPSC: never fails. Always returns slots[(seqno + 1) % count].
 */
void *lotus_shm_ring_claim(lotus_shm_ring_t *ring) {
    uint64_t cur = atomic_load_explicit(&ring->header->seqno,
                                         memory_order_relaxed);
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
