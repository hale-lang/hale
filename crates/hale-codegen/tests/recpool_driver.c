/*
 * v1.x-3 PR1 — C test driver for the recognition pool primitives.
 *
 * Forward-declares the lotus_recpool_* surface and exercises each
 * mode through a small set of asserts. Built by tests/recpool.rs
 * into a single binary that the Rust test invokes per-scenario.
 *
 * Modes:
 *   fixed_basic    — acquire/release round-trip + reuse
 *   fixed_overflow — cap exhaustion returns NULL
 *   fixed_alloc    — arena_alloc inside a cell, including overflow
 *   slab_basic     — every acquire returns same arena, alloc up to budget
 *   slab_overflow  — arena_alloc past slab_bytes returns NULL
 *   slab_noop      — release returns admission; slab storage keeps working
 *   slab_admission — admission exhaustion and reuse without resetting bytes
 *   creation_limits — invalid and overflowing reservations fail closed
 *   fixed_concurrent / slab_concurrent — reservation and release contention
 *
 * The driver prints exactly `OK <mode>\n` on success or
 * `FAIL <mode> <reason>\n` on failure, and exits nonzero on FAIL.
 */

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>

/* Forward declarations of the runtime surface — same shape as
 * fs_driver.c / transport_driver.c (no shared header). */
typedef struct lotus_arena lotus_arena_t;

void *lotus_arena_alloc(lotus_arena_t *a, size_t size, size_t align);

typedef struct lotus_recpool_fixed lotus_recpool_fixed_t;
typedef struct lotus_recpool_slab  lotus_recpool_slab_t;

lotus_recpool_fixed_t *lotus_recpool_fixed_create(size_t cap_count,
                                                  size_t cell_bytes);
lotus_arena_t *lotus_recpool_fixed_acquire(lotus_recpool_fixed_t *p);
void lotus_recpool_fixed_release(lotus_recpool_fixed_t *p,
                                 lotus_arena_t *arena);
void lotus_recpool_fixed_destroy(lotus_recpool_fixed_t *p);

lotus_recpool_slab_t *lotus_recpool_slab_create(size_t cap_count,
                                                size_t slab_bytes);
lotus_arena_t *lotus_recpool_slab_acquire(lotus_recpool_slab_t *p);
void lotus_recpool_slab_release(lotus_recpool_slab_t *p,
                                lotus_arena_t *arena);
void lotus_recpool_slab_destroy(lotus_recpool_slab_t *p);

static int pass(const char *mode) {
    printf("OK %s\n", mode);
    return 0;
}

static int fail(const char *mode, const char *reason) {
    printf("FAIL %s %s\n", mode, reason);
    return 1;
}

/* fixed_basic: acquire cap times, verify distinct arenas, release
 * all, acquire again, verify the bitmap reuses slots. */
static int run_fixed_basic(void) {
    const char *m = "fixed_basic";
    lotus_recpool_fixed_t *p = lotus_recpool_fixed_create(4, 128);
    if (!p) return fail(m, "create_returned_null");

    lotus_arena_t *a[4];
    for (int i = 0; i < 4; i++) {
        a[i] = lotus_recpool_fixed_acquire(p);
        if (!a[i]) { lotus_recpool_fixed_destroy(p); return fail(m, "acquire_returned_null"); }
        for (int j = 0; j < i; j++) {
            if (a[i] == a[j]) {
                lotus_recpool_fixed_destroy(p);
                return fail(m, "acquire_returned_duplicate");
            }
        }
    }
    /* fifth acquire fails — cap exhausted */
    if (lotus_recpool_fixed_acquire(p) != NULL) {
        lotus_recpool_fixed_destroy(p);
        return fail(m, "expected_null_on_overflow");
    }
    /* release all and re-acquire — slots should be reusable */
    for (int i = 0; i < 4; i++) lotus_recpool_fixed_release(p, a[i]);
    lotus_arena_t *r = lotus_recpool_fixed_acquire(p);
    if (!r) { lotus_recpool_fixed_destroy(p); return fail(m, "reacquire_after_release_failed"); }

    lotus_recpool_fixed_destroy(p);
    return pass(m);
}

/* fixed_overflow: cap=2 hits NULL on third acquire. */
static int run_fixed_overflow(void) {
    const char *m = "fixed_overflow";
    lotus_recpool_fixed_t *p = lotus_recpool_fixed_create(2, 64);
    if (!p) return fail(m, "create_null");
    if (!lotus_recpool_fixed_acquire(p)) { lotus_recpool_fixed_destroy(p); return fail(m, "a1_null"); }
    if (!lotus_recpool_fixed_acquire(p)) { lotus_recpool_fixed_destroy(p); return fail(m, "a2_null"); }
    if (lotus_recpool_fixed_acquire(p) != NULL) {
        lotus_recpool_fixed_destroy(p);
        return fail(m, "a3_should_be_null");
    }
    lotus_recpool_fixed_destroy(p);
    return pass(m);
}

/* fixed_alloc: arena_alloc inside one cell, verify the pointer
 * lands inside the cell's payload range, and that overflow yields
 * NULL (fixed_size flag honored). */
static int run_fixed_alloc(void) {
    const char *m = "fixed_alloc";
    /* cell_bytes=256 of payload; the inline header is separate. */
    lotus_recpool_fixed_t *p = lotus_recpool_fixed_create(1, 256);
    if (!p) return fail(m, "create_null");
    lotus_arena_t *a = lotus_recpool_fixed_acquire(p);
    if (!a) { lotus_recpool_fixed_destroy(p); return fail(m, "acquire_null"); }

    /* Fits comfortably. */
    void *x = lotus_arena_alloc(a, 64, 8);
    if (!x) { lotus_recpool_fixed_destroy(p); return fail(m, "small_alloc_null"); }
    /* Pointer must sit *after* the arena handle (the inline header). */
    if ((char *)x < (char *)a) {
        lotus_recpool_fixed_destroy(p);
        return fail(m, "alloc_below_arena");
    }
    /* Write/read round-trip — verify the bytes are usable. */
    memset(x, 0xAB, 64);
    for (int i = 0; i < 64; i++) {
        if (((unsigned char *)x)[i] != 0xAB) {
            lotus_recpool_fixed_destroy(p);
            return fail(m, "alloc_bytes_corrupt");
        }
    }
    /* Big alloc that won't fit in remaining budget — overflow. */
    void *too_big = lotus_arena_alloc(a, 4096, 8);
    if (too_big != NULL) {
        lotus_recpool_fixed_destroy(p);
        return fail(m, "fixed_size_grew_anyway");
    }
    lotus_recpool_fixed_destroy(p);
    return pass(m);
}

/* slab_basic: every acquire returns SAME arena; arena_alloc works
 * up to the slab budget. */
static int run_slab_basic(void) {
    const char *m = "slab_basic";
    lotus_recpool_slab_t *p = lotus_recpool_slab_create(8, 1024);
    if (!p) return fail(m, "create_null");
    lotus_arena_t *a1 = lotus_recpool_slab_acquire(p);
    lotus_arena_t *a2 = lotus_recpool_slab_acquire(p);
    if (!a1 || !a2) { lotus_recpool_slab_destroy(p); return fail(m, "acquire_null"); }
    if (a1 != a2) { lotus_recpool_slab_destroy(p); return fail(m, "siblings_got_different_arenas"); }
    void *x = lotus_arena_alloc(a1, 256, 8);
    void *y = lotus_arena_alloc(a2, 256, 8);
    if (!x || !y) { lotus_recpool_slab_destroy(p); return fail(m, "interleaved_alloc_null"); }
    if (x == y) { lotus_recpool_slab_destroy(p); return fail(m, "interleaved_alloc_aliased"); }
    lotus_recpool_slab_destroy(p);
    return pass(m);
}

/* slab_overflow: arena_alloc past slab_bytes returns NULL — no
 * silent malloc. */
static int run_slab_overflow(void) {
    const char *m = "slab_overflow";
    lotus_recpool_slab_t *p = lotus_recpool_slab_create(4, 256);
    if (!p) return fail(m, "create_null");
    lotus_arena_t *a = lotus_recpool_slab_acquire(p);
    if (!a) { lotus_recpool_slab_destroy(p); return fail(m, "acquire_null"); }
    void *first = lotus_arena_alloc(a, 200, 8);
    if (!first) { lotus_recpool_slab_destroy(p); return fail(m, "first_alloc_null"); }
    void *over = lotus_arena_alloc(a, 200, 8);
    if (over != NULL) {
        lotus_recpool_slab_destroy(p);
        return fail(m, "second_alloc_should_overflow");
    }
    lotus_recpool_slab_destroy(p);
    return pass(m);
}

/* slab_noop: per-child release retains and does not corrupt the
 * shared slab. */
static int run_slab_noop(void) {
    const char *m = "slab_noop";
    lotus_recpool_slab_t *p = lotus_recpool_slab_create(4, 512);
    if (!p) return fail(m, "create_null");
    lotus_arena_t *a1 = lotus_recpool_slab_acquire(p);
    lotus_recpool_slab_release(p, a1);
    /* After release, the slab must still serve a fresh acquire +
     * allocate from the SAME backing memory (no double free). */
    lotus_arena_t *a2 = lotus_recpool_slab_acquire(p);
    if (a1 != a2) { lotus_recpool_slab_destroy(p); return fail(m, "release_changed_arena"); }
    void *x = lotus_arena_alloc(a2, 64, 8);
    if (!x) { lotus_recpool_slab_destroy(p); return fail(m, "post_release_alloc_null"); }
    lotus_recpool_slab_destroy(p);
    return pass(m);
}

/* The cap counts simultaneous reservations, independently of bytes. */
static int run_slab_admission(void) {
    const char *m = "slab_admission";
    lotus_recpool_slab_t *p = lotus_recpool_slab_create(1, 128);
    if (!p) return fail(m, "create_null");
    lotus_arena_t *a = lotus_recpool_slab_acquire(p);
    if (!a || lotus_recpool_slab_acquire(p)) return fail(m, "cap_not_enforced");
    unsigned char *bytes = lotus_arena_alloc(a, 8, 8);
    if (!bytes) return fail(m, "allocation_failed");
    bytes[0] = 0xAB;
    lotus_recpool_slab_release(p, a);
    lotus_arena_t *b = lotus_recpool_slab_acquire(p);
    if (b != a || bytes[0] != 0xAB || lotus_recpool_slab_acquire(p))
        return fail(m, "release_changed_storage_or_lost_cap");
    unsigned char *more = lotus_arena_alloc(b, 8, 8);
    if (!more || more == bytes || bytes[0] != 0xAB) return fail(m, "release_rewound_slab");
    lotus_recpool_slab_release(p, b);
    lotus_recpool_slab_destroy(p);
    return pass(m);
}

static int run_creation_limits(void) {
    const char *m = "creation_limits";
    if (lotus_recpool_fixed_create(0, 1) || lotus_recpool_fixed_create(1, 0)
        || lotus_recpool_fixed_create(1, SIZE_MAX) || lotus_recpool_fixed_create(SIZE_MAX, 1)
        || lotus_recpool_slab_create(0, 128) || lotus_recpool_slab_create(1, SIZE_MAX))
        return fail(m, "invalid_or_wrapping_reservation_accepted");
    return pass(m);
}

/* Every worker acquires before ANY worker may release. The burst must
 * reserve exactly cap, without two fixed-cell callers owning one cell.
 * Release then occurs concurrently; every slot must become reusable.
 * 65 spans a bitmap-word boundary and leaves 63 unusable tail bits. */
#define BURST_THREADS 72
#define BURST_CAP 65
struct burst {
    pthread_mutex_t lock;
    pthread_cond_t changed;
    int started, releasing, attempted;
    int slab;
    void *pool;
    lotus_arena_t *result[BURST_THREADS];
};
struct burst_arg { struct burst *b; int index; };

static lotus_arena_t *burst_acquire(struct burst *b) {
    return b->slab ? lotus_recpool_slab_acquire(b->pool) : lotus_recpool_fixed_acquire(b->pool);
}
static void burst_release(struct burst *b, lotus_arena_t *a) {
    if (b->slab) lotus_recpool_slab_release(b->pool, a);
    else lotus_recpool_fixed_release(b->pool, a);
}
static void *burst_worker(void *raw) {
    struct burst_arg *arg = raw;
    struct burst *b = arg->b;
    pthread_mutex_lock(&b->lock);
    while (!b->started) pthread_cond_wait(&b->changed, &b->lock);
    pthread_mutex_unlock(&b->lock);
    lotus_arena_t *a = burst_acquire(b);
    pthread_mutex_lock(&b->lock);
    b->result[arg->index] = a;
    b->attempted++;
    pthread_cond_broadcast(&b->changed);
    while (!b->releasing) pthread_cond_wait(&b->changed, &b->lock);
    pthread_mutex_unlock(&b->lock);
    if (a) burst_release(b, a);
    return NULL;
}

static int run_concurrent(int slab) {
    const char *m = slab ? "slab_concurrent" : "fixed_concurrent";
    struct burst b = { .slab = slab };
    b.pool = slab ? (void *)lotus_recpool_slab_create(BURST_CAP, 1024)
                  : (void *)lotus_recpool_fixed_create(BURST_CAP, 128);
    if (!b.pool) return fail(m, "create_null");
    pthread_mutex_init(&b.lock, NULL);
    pthread_cond_init(&b.changed, NULL);
    pthread_t threads[BURST_THREADS];
    struct burst_arg args[BURST_THREADS];
    for (int i = 0; i < BURST_THREADS; i++) {
        args[i] = (struct burst_arg){ &b, i };
        if (pthread_create(&threads[i], NULL, burst_worker, &args[i])) abort();
    }
    pthread_mutex_lock(&b.lock);
    b.started = 1;
    pthread_cond_broadcast(&b.changed);
    while (b.attempted != BURST_THREADS) pthread_cond_wait(&b.changed, &b.lock);
    int count = 0, duplicate = 0;
    for (int i = 0; i < BURST_THREADS; i++) if (b.result[i]) {
        count++;
        if (!slab) for (int j = 0; j < i; j++) if (b.result[j] == b.result[i]) duplicate = 1;
    }
    int overflow = burst_acquire(&b) != NULL;
    b.releasing = 1;
    pthread_cond_broadcast(&b.changed);
    pthread_mutex_unlock(&b.lock);
    for (int i = 0; i < BURST_THREADS; i++) pthread_join(threads[i], NULL);
    int refilled = 0;
    for (int i = 0; i < BURST_CAP; i++) if (burst_acquire(&b)) refilled++;
    int overfill = burst_acquire(&b) != NULL;
    if (slab) lotus_recpool_slab_destroy(b.pool); else lotus_recpool_fixed_destroy(b.pool);
    pthread_cond_destroy(&b.changed);
    pthread_mutex_destroy(&b.lock);
    if (count != BURST_CAP || duplicate || overflow || refilled != BURST_CAP || overfill)
        return fail(m, "lost_or_duplicated_reservation");
    return pass(m);
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: recpool_driver <mode>\n");
        return 2;
    }
    const char *mode = argv[1];
    if (!strcmp(mode, "slab_admission")) return run_slab_admission();
    if (!strcmp(mode, "creation_limits")) return run_creation_limits();
    if (!strcmp(mode, "fixed_concurrent")) return run_concurrent(0);
    if (!strcmp(mode, "slab_concurrent")) return run_concurrent(1);
    if (!strcmp(mode, "fixed_basic"))    return run_fixed_basic();
    if (!strcmp(mode, "fixed_overflow")) return run_fixed_overflow();
    if (!strcmp(mode, "fixed_alloc"))    return run_fixed_alloc();
    if (!strcmp(mode, "slab_basic"))     return run_slab_basic();
    if (!strcmp(mode, "slab_overflow"))  return run_slab_overflow();
    if (!strcmp(mode, "slab_noop"))      return run_slab_noop();
    fprintf(stderr, "unknown mode: %s\n", mode);
    return 2;
}
