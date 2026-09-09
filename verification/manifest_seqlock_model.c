/* GenMC model of the observation-protocol manifest append
 * (PROTOCOL.md §4; reference impl observe/glue.c manifest_add
 * and consumer/obs_attach.c rescan).
 *
 * FAITHFUL TRANSCRIPTION of the distinctive concurrency: the
 * emitter writes the entry fields and string bytes PLAIN, then
 * release-stores entry_count, then release-fetch-adds
 * manifest_gen; a consumer acquire-loads manifest_gen, and for
 * any generation it observes, acquire-loads entry_count and
 * reads entries below it. The checked invariant is the
 * protocol's real contract: every entry at index < an observed
 * entry_count is COMPLETE (never partially initialized), and
 * entry_count under an observed generation covers the appends
 * that produced that generation.
 *
 * NOT the production code: 2 entries, 1 field standing in for
 * the 32-byte record + string bytes. Production entry writes
 * are plain (entries are never re-read by the emitter); C11
 * calls the concurrent plain access a race, so the model uses
 * RELAXED atomics for the field and asserts the contract.
 *
 * Run:  genmc -- verification/manifest_seqlock_model.c
 */
#include <assert.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>

#define ENTRIES 2
#define COMPLETE 0xC0FFEE

static _Atomic uint64_t field[ENTRIES];   /* stands in for entry+strings */
static _Atomic uint32_t entry_count;
static _Atomic uint64_t manifest_gen;

static void *emitter(void *arg) {
  (void)arg;
  for (uint32_t i = 0; i < ENTRIES; i++) {
    atomic_store_explicit(&field[i], COMPLETE, memory_order_relaxed);
    atomic_store_explicit(&entry_count, i + 1, memory_order_release);
    atomic_fetch_add_explicit(&manifest_gen, 1, memory_order_release);
  }
  return 0;
}

static void *consumer(void *arg) {
  (void)arg;
  uint64_t gen = atomic_load_explicit(&manifest_gen, memory_order_acquire);
  uint32_t n = atomic_load_explicit(&entry_count, memory_order_acquire);
  /* generation g implies at least g completed appends visible */
  assert(n >= gen);
  for (uint32_t i = 0; i < n && i < ENTRIES; i++)
    assert(atomic_load_explicit(&field[i], memory_order_relaxed) == COMPLETE);
  return 0;
}

int main(void) {
  pthread_t e, c;
  pthread_create(&e, 0, emitter, 0);
  pthread_create(&c, 0, consumer, 0);
  pthread_join(e, 0);
  pthread_join(c, 0);
  return 0;
}
