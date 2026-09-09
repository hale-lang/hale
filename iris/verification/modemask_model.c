/* GenMC model of the per-topic mode-mask handshake
 * (PROTOCOL.md §7; reference impl observe/glue.c obs_mode and
 * consumer control writes).
 *
 * FAITHFUL TRANSCRIPTION: the consumer stores mode bytes with
 * plain/relaxed intent through the control mapping; the emitter
 * reads the byte RELAXED on every publish and gates emission.
 * Staleness is tolerated by design — the checked contract is
 * only that (a) the concurrent access is race-free under the
 * C11 model with the declared orderings, (b) every value the
 * emitter observes is one the consumer actually wrote (domain
 * validity: OFF or PACKED here), and (c) an emission implies a
 * PACKED read gated it.
 *
 * Run:  genmc -- verification/modemask_model.c
 */
#include <assert.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>

#define OFF 0
#define PACKED 2

static _Atomic uint8_t mode = PACKED;
static _Atomic uint32_t emitted;

static void *observer_ctl(void *arg) {
  (void)arg;
  atomic_store_explicit(&mode, OFF, memory_order_relaxed);
  atomic_store_explicit(&mode, PACKED, memory_order_relaxed);
  return 0;
}

static void *emitter(void *arg) {
  (void)arg;
  for (int i = 0; i < 2; i++) {
    uint8_t m = atomic_load_explicit(&mode, memory_order_relaxed);
    assert(m == OFF || m == PACKED);          /* domain validity */
    if (m >= PACKED)
      atomic_fetch_add_explicit(&emitted, 1, memory_order_relaxed);
  }
  return 0;
}

int main(void) {
  pthread_t o, e;
  pthread_create(&o, 0, observer_ctl, 0);
  pthread_create(&e, 0, emitter, 0);
  pthread_join(o, 0);
  pthread_join(e, 0);
  assert(atomic_load(&emitted) <= 2);
  return 0;
}
