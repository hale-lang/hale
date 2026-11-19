/*
 * The main bus queue's bounded wait: lotus_bus_queue_idle_wait. Built
 * by tests/bus_idle_wait.rs (clang, linked against lotus_arena.c) and
 * run once; asserts are in-process.
 *
 *   - a wait with nothing enqueued returns at its deadline (not before,
 *     not long after);
 *   - an enqueue from another thread ends a 1 s wait promptly and the
 *     wait runs the cell's handler before it returns;
 *   - a cell already queued when the wait begins does not wait at all;
 *   - a sleep's slice on the queue (lotus_bus_queue_sleep, GH #2091)
 *     runs a cell a foreign thread enqueues mid-slice when it arrives,
 *     and still lasts its whole length;
 *   - a handler of the queue's drain that sleeps on the queue (a handler
 *     on main that sleeps) runs no cell inside itself, and its slice is a
 *     plain wait that costs no CPU, not a busy loop.
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
void lotus_bus_queue_sleep(lotus_bus_queue_t *q, int64_t ns);
void lotus_bus_mark_pinned(void);

static volatile int g_ran = 0;
static int64_t g_enqueued_at = 0;

static int64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

static int64_t g_ran_at = 0;

static void handler(void *self, void *payload) {
    (void)self;
    (void)payload;
    g_ran++;
    g_ran_at = now_ns();
}

static void *producer(void *arg) {
    lotus_bus_queue_t *q = arg;
    usleep(50 * 1000);
    g_enqueued_at = now_ns();
    lotus_bus_queue_enqueue(q, (void *)handler, NULL, NULL, 0);
    return NULL;
}

static int64_t thread_cpu_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts);
    return (int64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

/* A handler of the queue's drain that sleeps on the queue while another
 * thread enqueues a cell 50 ms into its 200 ms slice. */
static lotus_bus_queue_t *g_q = NULL;
static int g_ran_inside = -1;
static int64_t g_inside_wall_ns = 0;
static int64_t g_inside_cpu_ns = 0;

static void sleeper(void *self, void *payload) {
    (void)self;
    (void)payload;
    pthread_t th;
    pthread_create(&th, NULL, producer, g_q);
    int before = g_ran;
    int64_t c0 = thread_cpu_ns();
    int64_t t0 = now_ns();
    lotus_bus_queue_sleep(g_q, 200 * 1000000LL);
    g_inside_wall_ns = now_ns() - t0;
    g_inside_cpu_ns = thread_cpu_ns() - c0;
    pthread_join(th, NULL);
    g_ran_inside = g_ran - before;
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

    /* A sleep's slice: the cell enqueued 50 ms into a 300 ms slice runs
     * when it arrives, and the slice still lasts its 300 ms. */
    pthread_create(&th, NULL, producer, q);
    t0 = now_ns();
    lotus_bus_queue_sleep(q, 300 * 1000000LL);
    t1 = now_ns();
    pthread_join(th, NULL);
    assert(g_ran == 3);
    int64_t slice_wake_ns = g_ran_at - g_enqueued_at;
    assert(slice_wake_ns < 5 * 1000000LL);
    assert(t1 - t0 >= 299 * 1000000LL);
    assert(t1 - t0 < 800 * 1000000LL);

    /* Nothing enqueued: the slice is its length. */
    t0 = now_ns();
    lotus_bus_queue_sleep(q, 30 * 1000000LL);
    waited = now_ns() - t0;
    assert(waited >= 29 * 1000000LL);
    assert(waited < 500 * 1000000LL);
    assert(g_ran == 3);

    /* A handler that sleeps on its own queue (GH #2091): the cell
     * enqueued during its slice runs after it returns, never inside it;
     * the slice lasts its 200 ms; and it is a wait, not a loop that wakes
     * for the queued cell and finds it still queued at every look after
     * (that spun out the slice: ~150 ms of CPU). */
    g_q = q;
    lotus_bus_queue_enqueue(q, (void *)sleeper, NULL, NULL, 0);
    lotus_bus_queue_idle_wait(q, 1000 * 1000000LL);
    assert(g_ran_inside == 0);
    assert(g_ran == 4);
    assert(g_inside_wall_ns >= 199 * 1000000LL);
    assert(g_inside_cpu_ns < 50 * 1000000LL);

    lotus_bus_queue_destroy(q);
    printf("ok wake_us=%lld slice_wake_us=%lld inside_cpu_us=%lld\n",
           (long long)(wake_ns / 1000), (long long)(slice_wake_ns / 1000),
           (long long)(g_inside_cpu_ns / 1000));
    return 0;
}
