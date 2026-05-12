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
 *   slab_noop      — release is a no-op; arena keeps working
 *
 * The driver prints exactly `OK <mode>\n` on success or
 * `FAIL <mode> <reason>\n` on failure, and exits nonzero on FAIL.
 */

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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

/* slab_noop: per-child release is no-op and doesn't corrupt the
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

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: recpool_driver <mode>\n");
        return 2;
    }
    const char *mode = argv[1];
    if (!strcmp(mode, "fixed_basic"))    return run_fixed_basic();
    if (!strcmp(mode, "fixed_overflow")) return run_fixed_overflow();
    if (!strcmp(mode, "fixed_alloc"))    return run_fixed_alloc();
    if (!strcmp(mode, "slab_basic"))     return run_slab_basic();
    if (!strcmp(mode, "slab_overflow"))  return run_slab_overflow();
    if (!strcmp(mode, "slab_noop"))      return run_slab_noop();
    fprintf(stderr, "unknown mode: %s\n", mode);
    return 2;
}
