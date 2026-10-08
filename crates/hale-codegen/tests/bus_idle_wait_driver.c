/*
 * The main bus queue's bounded wait: lotus_bus_queue_idle_wait. Built
 * by tests/bus_idle_wait.rs (clang, linked against lotus_arena.c) and
 * run once; asserts are in-process.
 *
 *   - a wait with nothing enqueued returns at its deadline (not before,
 *     not long after);
 *   - an enqueue from another thread ends a 1 s wait promptly and the
 *     wait runs the cell's handler before it returns;
 *   - a cell already queued when the wait begins does not wait at all.
 */

#include <assert.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>

typedef struct lotus_bus_queue lotus_bus_queue_t;

lotus_bus_queue_t *lotus_bus_queue_create(void);
void lotus_bus_queue_destroy(lotus_bus_queue_t *q);
void lotus_bus_queue_enqueue(lotus_bus_queue_t *q, void *handler,
                             void *self_ptr, const void *payload_src,
                             size_t payload_size);
void lotus_bus_queue_idle_wait(lotus_bus_queue_t *q, int64_t ns);
void lotus_bus_mark_pinned(void);

static volatile int g_ran = 0;
static int64_t g_enqueued_at = 0;

static int64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

static void handler(void *self, void *payload) {
    (void)self;
    (void)payload;
    g_ran++;
}

static void *producer(void *arg) {
    lotus_bus_queue_t *q = arg;
    usleep(50 * 1000);
    g_enqueued_at = now_ns();
    lotus_bus_queue_enqueue(q, (void *)handler, NULL, NULL, 0);
    return NULL;
}

int main(void) {
    lotus_bus_queue_t *q = lotus_bus_queue_create();
    assert(q);
    lotus_bus_mark_pinned();

    /* Nothing enqueued: the wait ends at its deadline. */
    int64_t t0 = now_ns();
    lotus_bus_queue_idle_wait(q, 50 * 1000000LL);
    int64_t waited = now_ns() - t0;
    assert(waited >= 49 * 1000000LL);
    assert(waited < 500 * 1000000LL);
    assert(g_ran == 0);

    /* A foreign enqueue ends a 1 s wait, and the handler has run. */
    pthread_t th;
    pthread_create(&th, NULL, producer, q);
    t0 = now_ns();
    lotus_bus_queue_idle_wait(q, 1000 * 1000000LL);
    int64_t t1 = now_ns();
    pthread_join(th, NULL);
    assert(g_ran == 1);
    int64_t wake_ns = t1 - g_enqueued_at;
    assert(t1 - t0 < 500 * 1000000LL);
    assert(wake_ns < 5 * 1000000LL);

    /* A cell queued before the wait begins: no wait. */
    lotus_bus_queue_enqueue(q, (void *)handler, NULL, NULL, 0);
    t0 = now_ns();
    lotus_bus_queue_idle_wait(q, 1000 * 1000000LL);
    assert(now_ns() - t0 < 100 * 1000000LL);
    assert(g_ran == 2);

    lotus_bus_queue_destroy(q);
    printf("ok wake_us=%lld\n", (long long)(wake_ns / 1000));
    return 0;
}
