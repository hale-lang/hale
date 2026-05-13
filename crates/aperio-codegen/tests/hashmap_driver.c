/*
 * v1.x-FORM-4 PR4 — C driver for lotus_hashmap_* primitives.
 *
 * Exercises the open-addressing intrusive hashmap directly via
 * its C ABI (no LLVM codegen path involved). Each mode is one
 * test scenario; the driver prints `OK <mode>` on success and
 * `FAIL <mode> <reason>` on failure.
 *
 * The intrusive shape means values carry their own keys. Real
 * programs would GEP the key from the value struct before each
 * call; this driver mimics that by passing key + value as
 * separate pointers.
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Re-declare the public ABI so the driver doesn't depend on
 * exposing the internal `lotus_hashmap_t` struct. The init/grow
 * paths know the layout; callers see an opaque blob big enough
 * to hold it. Size = 5 * size_t/int + pointer = comfortably
 * under 64 bytes on a 64-bit host; allocate generously. */
#define LOTUS_HASHMAP_OPAQUE_SIZE 128

void lotus_hashmap_init(void *map_ptr,
                        size_t key_size,
                        size_t value_size,
                        int key_type_tag);
void lotus_hashmap_set(void *map_ptr,
                       const void *key,
                       const void *value);
int lotus_hashmap_get(void *map_ptr, const void *key, void *out_value);
int lotus_hashmap_has(void *map_ptr, const void *key);
int lotus_hashmap_remove(void *map_ptr, const void *key);
int64_t lotus_hashmap_len(void *map_ptr);
int lotus_hashmap_is_empty(void *map_ptr);
void lotus_hashmap_destroy(void *map_ptr);

#define LOTUS_HASHMAP_KEY_INT    0
#define LOTUS_HASHMAP_KEY_STRING 1

typedef struct {
    int64_t id;
    int64_t payload;
} IntEntry;

typedef struct {
    const char *name;
    int64_t v;
} StringEntry;

static void fail(const char *mode, const char *reason) {
    printf("FAIL %s %s\n", mode, reason);
    exit(1);
}

static void ok(const char *mode) {
    printf("OK %s\n", mode);
}

/* === modes ============================================ */

static void run_int_basic_round_trip(void) {
    char map[LOTUS_HASHMAP_OPAQUE_SIZE];
    memset(map, 0, sizeof(map));
    lotus_hashmap_init(map, sizeof(int64_t), sizeof(IntEntry),
                       LOTUS_HASHMAP_KEY_INT);
    IntEntry e = { .id = 42, .payload = 100 };
    int64_t key = 42;
    lotus_hashmap_set(map, &key, &e);
    if (lotus_hashmap_len(map) != 1) fail("int_basic_round_trip", "len");
    IntEntry out = { 0, 0 };
    if (!lotus_hashmap_get(map, &key, &out)) fail("int_basic_round_trip", "get_missed");
    if (out.id != 42 || out.payload != 100) fail("int_basic_round_trip", "payload_mismatch");
    lotus_hashmap_destroy(map);
    ok("int_basic_round_trip");
}

static void run_string_basic_round_trip(void) {
    char map[LOTUS_HASHMAP_OPAQUE_SIZE];
    memset(map, 0, sizeof(map));
    lotus_hashmap_init(map, sizeof(const char *), sizeof(StringEntry),
                       LOTUS_HASHMAP_KEY_STRING);
    const char *name = "alpha";
    StringEntry e = { .name = name, .v = 7 };
    lotus_hashmap_set(map, &name, &e);
    StringEntry out = { NULL, 0 };
    if (!lotus_hashmap_get(map, &name, &out)) fail("string_basic_round_trip", "get_missed");
    if (out.v != 7) fail("string_basic_round_trip", "value");
    if (strcmp(out.name, "alpha") != 0) fail("string_basic_round_trip", "key_field");
    lotus_hashmap_destroy(map);
    ok("string_basic_round_trip");
}

static void run_string_distinct_pointers_equal_bytes(void) {
    /* Two C-string literals with equal bytes but possibly
     * distinct pointers should hash-equal and key-equal. */
    char map[LOTUS_HASHMAP_OPAQUE_SIZE];
    memset(map, 0, sizeof(map));
    lotus_hashmap_init(map, sizeof(const char *), sizeof(StringEntry),
                       LOTUS_HASHMAP_KEY_STRING);
    char a[] = "shared-name";
    char b[] = "shared-name";
    /* Defensive: confirm clang didn't merge the two literal-
     * initialized arrays into one. Cast to void* to dodge the
     * array-compare warning the bare form would trigger. */
    if ((void *)a == (void *)b) fail("string_distinct_pointers", "pointers_collapsed");
    StringEntry e = { .name = a, .v = 99 };
    const char *ka = a;
    const char *kb = b;
    lotus_hashmap_set(map, &ka, &e);
    StringEntry out = { NULL, 0 };
    if (!lotus_hashmap_get(map, &kb, &out)) fail("string_distinct_pointers", "get_with_distinct_ptr");
    if (out.v != 99) fail("string_distinct_pointers", "value");
    lotus_hashmap_destroy(map);
    ok("string_distinct_pointers");
}

static void run_grow_at_load_threshold(void) {
    /* Initial cap is 8, threshold is 0.7 → grow after ~5 entries.
     * Insert 32 entries, verify all retrievable, verify len. */
    char map[LOTUS_HASHMAP_OPAQUE_SIZE];
    memset(map, 0, sizeof(map));
    lotus_hashmap_init(map, sizeof(int64_t), sizeof(IntEntry),
                       LOTUS_HASHMAP_KEY_INT);
    for (int64_t i = 0; i < 32; i++) {
        IntEntry e = { .id = i, .payload = i * 10 };
        lotus_hashmap_set(map, &i, &e);
    }
    if (lotus_hashmap_len(map) != 32) fail("grow_at_load_threshold", "len_after_grow");
    for (int64_t i = 0; i < 32; i++) {
        IntEntry out = { 0, 0 };
        if (!lotus_hashmap_get(map, &i, &out)) fail("grow_at_load_threshold", "missing_after_grow");
        if (out.payload != i * 10) fail("grow_at_load_threshold", "wrong_value_after_grow");
    }
    lotus_hashmap_destroy(map);
    ok("grow_at_load_threshold");
}

static void run_overwrite_on_duplicate_key(void) {
    char map[LOTUS_HASHMAP_OPAQUE_SIZE];
    memset(map, 0, sizeof(map));
    lotus_hashmap_init(map, sizeof(int64_t), sizeof(IntEntry),
                       LOTUS_HASHMAP_KEY_INT);
    int64_t key = 1;
    IntEntry e1 = { .id = 1, .payload = 100 };
    IntEntry e2 = { .id = 1, .payload = 200 };
    lotus_hashmap_set(map, &key, &e1);
    lotus_hashmap_set(map, &key, &e2);
    if (lotus_hashmap_len(map) != 1) fail("overwrite_on_duplicate_key", "len_increased");
    IntEntry out = { 0, 0 };
    if (!lotus_hashmap_get(map, &key, &out)) fail("overwrite_on_duplicate_key", "get_missed");
    if (out.payload != 200) fail("overwrite_on_duplicate_key", "old_value_remains");
    lotus_hashmap_destroy(map);
    ok("overwrite_on_duplicate_key");
}

static void run_remove_basic(void) {
    char map[LOTUS_HASHMAP_OPAQUE_SIZE];
    memset(map, 0, sizeof(map));
    lotus_hashmap_init(map, sizeof(int64_t), sizeof(IntEntry),
                       LOTUS_HASHMAP_KEY_INT);
    int64_t key = 42;
    IntEntry e = { .id = 42, .payload = 100 };
    lotus_hashmap_set(map, &key, &e);
    if (!lotus_hashmap_remove(map, &key)) fail("remove_basic", "remove_missed");
    if (lotus_hashmap_len(map) != 0) fail("remove_basic", "len_after_remove");
    if (lotus_hashmap_has(map, &key)) fail("remove_basic", "still_has_after_remove");
    IntEntry out = { 0, 0 };
    if (lotus_hashmap_get(map, &key, &out)) fail("remove_basic", "still_get_after_remove");
    /* Removing a missing key returns 0. */
    if (lotus_hashmap_remove(map, &key)) fail("remove_basic", "remove_missing_returned_ok");
    lotus_hashmap_destroy(map);
    ok("remove_basic");
}

static void run_remove_with_probe_chain(void) {
    /* Insert several entries with adjacent natural slots so
     * probing kicks in. Remove a middle entry. Verify the rest
     * are still findable (backward-shift kept the chain
     * intact).
     *
     * With cap=8 and mask=7, Int keys that hash to the same
     * slot after Knuth-multiplicative are hard to construct
     * deterministically; instead, insert N=6 entries (under
     * threshold so no grow), remove one in the middle, verify
     * all others still findable. The hashmap's internal
     * placement may or may not chain — but removal must always
     * preserve `has` and `get` for non-removed entries.
     */
    char map[LOTUS_HASHMAP_OPAQUE_SIZE];
    memset(map, 0, sizeof(map));
    lotus_hashmap_init(map, sizeof(int64_t), sizeof(IntEntry),
                       LOTUS_HASHMAP_KEY_INT);
    int64_t keys[6] = { 11, 22, 33, 44, 55, 66 };
    for (int i = 0; i < 6; i++) {
        IntEntry e = { .id = keys[i], .payload = keys[i] * 10 };
        lotus_hashmap_set(map, &keys[i], &e);
    }
    if (lotus_hashmap_len(map) != 6) fail("remove_with_probe_chain", "len_pre_remove");
    /* Remove the middle entry. */
    if (!lotus_hashmap_remove(map, &keys[2])) fail("remove_with_probe_chain", "remove_failed");
    if (lotus_hashmap_len(map) != 5) fail("remove_with_probe_chain", "len_after_remove");
    /* All other keys still findable. */
    for (int i = 0; i < 6; i++) {
        if (i == 2) {
            if (lotus_hashmap_has(map, &keys[i])) fail("remove_with_probe_chain", "removed_still_present");
            continue;
        }
        IntEntry out = { 0, 0 };
        if (!lotus_hashmap_get(map, &keys[i], &out)) fail("remove_with_probe_chain", "neighbor_lost");
        if (out.payload != keys[i] * 10) fail("remove_with_probe_chain", "neighbor_corrupted");
    }
    lotus_hashmap_destroy(map);
    ok("remove_with_probe_chain");
}

static void run_len_and_is_empty(void) {
    char map[LOTUS_HASHMAP_OPAQUE_SIZE];
    memset(map, 0, sizeof(map));
    lotus_hashmap_init(map, sizeof(int64_t), sizeof(IntEntry),
                       LOTUS_HASHMAP_KEY_INT);
    if (lotus_hashmap_len(map) != 0) fail("len_and_is_empty", "initial_len");
    if (!lotus_hashmap_is_empty(map)) fail("len_and_is_empty", "initial_is_empty");
    int64_t k = 1;
    IntEntry e = { .id = 1, .payload = 1 };
    lotus_hashmap_set(map, &k, &e);
    if (lotus_hashmap_len(map) != 1) fail("len_and_is_empty", "len_after_insert");
    if (lotus_hashmap_is_empty(map)) fail("len_and_is_empty", "is_empty_after_insert");
    lotus_hashmap_remove(map, &k);
    if (lotus_hashmap_len(map) != 0) fail("len_and_is_empty", "len_after_remove");
    if (!lotus_hashmap_is_empty(map)) fail("len_and_is_empty", "is_empty_after_remove");
    lotus_hashmap_destroy(map);
    ok("len_and_is_empty");
}

static void run_get_missing_returns_zero(void) {
    char map[LOTUS_HASHMAP_OPAQUE_SIZE];
    memset(map, 0, sizeof(map));
    lotus_hashmap_init(map, sizeof(int64_t), sizeof(IntEntry),
                       LOTUS_HASHMAP_KEY_INT);
    int64_t k = 999;
    IntEntry out = { 0, 0 };
    if (lotus_hashmap_get(map, &k, &out)) fail("get_missing", "got_value");
    if (lotus_hashmap_has(map, &k)) fail("get_missing", "has_returned_true");
    lotus_hashmap_destroy(map);
    ok("get_missing");
}

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <mode>\n", argv[0]);
        return 2;
    }
    const char *mode = argv[1];
    if (strcmp(mode, "int_basic_round_trip") == 0) run_int_basic_round_trip();
    else if (strcmp(mode, "string_basic_round_trip") == 0) run_string_basic_round_trip();
    else if (strcmp(mode, "string_distinct_pointers") == 0) run_string_distinct_pointers_equal_bytes();
    else if (strcmp(mode, "grow_at_load_threshold") == 0) run_grow_at_load_threshold();
    else if (strcmp(mode, "overwrite_on_duplicate_key") == 0) run_overwrite_on_duplicate_key();
    else if (strcmp(mode, "remove_basic") == 0) run_remove_basic();
    else if (strcmp(mode, "remove_with_probe_chain") == 0) run_remove_with_probe_chain();
    else if (strcmp(mode, "len_and_is_empty") == 0) run_len_and_is_empty();
    else if (strcmp(mode, "get_missing") == 0) run_get_missing_returns_zero();
    else {
        fprintf(stderr, "unknown mode: %s\n", mode);
        return 2;
    }
    return 0;
}
