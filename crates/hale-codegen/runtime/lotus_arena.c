/*
 * Lotus region allocator — v0 substrate.
 *
 * One arena = a linked list of bump chunks. Allocation bumps a
 * pointer in the head chunk; if the head can't fit the request,
 * a fresh chunk is malloc'd and pushed on the front. Destruction
 * walks the list and frees every chunk wholesale — no per-object
 * free, ever (matching spec/memory.md: "When the locus dissolves,
 * the region is freed wholesale.").
 *
 * v0 lives behind a stable C ABI so the LLVM-IR side of the
 * compiler doesn't need to know about the chunk-list shape.
 * m22 added per-coordinatee sub-regions (chunked-class
 * projection): a parent arena can carve "sub-region" arenas for
 * its accepted children, and tracks the slot indices via a
 * free-list so children can come and go without the parent's
 * bookkeeping growing unbounded. Sub-regions still hold their
 * own chunk lists — they're independent allocations — but they
 * register with the parent on creation and return their slot to
 * the parent's free-list on destroy.
 *
 * Backed by libc malloc for the chunks themselves. That's not a
 * cheat — the substrate's job is wholesale-region management;
 * the underlying *page* supplier can be libc, mmap, or a
 * pre-reserved pool, and the arena interface above doesn't
 * change. Replace this file's malloc/free with mmap when the
 * scheduler lands and we want page-aligned regions.
 */

#define _GNU_SOURCE
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdatomic.h>
#include <inttypes.h>
#include <limits.h>
#include <malloc.h>
#include <string.h>
#include <pthread.h>
#include <sched.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <unistd.h>
#include <errno.h>
#include <time.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <dirent.h>
#include <math.h>
/* C4: getrandom(2). Glibc 2.25+ exposes the declaration via
 * <sys/random.h>; we still gate the call site on a feature
 * macro so platforms that lack the syscall fall through to
 * the /dev/urandom path cleanly. */
#if defined(__linux__) || defined(__GLIBC__)
#include <sys/random.h>
#endif
/* C2 (pond/subprocess + pond/agent/sandbox): fork/exec/wait,
 * kill signals, poll for non-blocking pipe reads. */
#include <signal.h>
#include <sys/wait.h>
#include <poll.h>
/* F.35 Slice 1 (2026-05-28): per-pool epoll for async_io cooperative
 * pools + ucontext-backed coroutine save/restore so blocking syscalls
 * inside locus methods can park-and-resume instead of blocking the
 * pool's OS thread. Dormant in this slice — the `async_io_enabled`
 * flag stays 0 until Slice 2 wires placement constraint to set it. */
#include <sys/epoll.h>
#include <ucontext.h>

/* F.32-1γ-v2 session 2 (2026-05-26): TSAN suppressions.
 *
 * When the binary is built with `-fsanitize=thread` (driven by
 * `LOTUS_TSAN=1` in `build_executable`), ThreadSanitizer
 * intercepts every memory access and reports inter-thread
 * races. The lockfree hashmap is the target of this validation
 * sweep, but TSAN sees the entire process — it surfaces
 * pre-existing races in the *substrate* (arena allocator,
 * bus queue, scheduler shutdown) that predate γ-v2 and have
 * their own follow-up work to harden.
 *
 * `__tsan_default_suppressions` is TSAN's hook for the program
 * to embed its own suppression list at link time, so users
 * running `LOTUS_TSAN=1 cargo test` don't need to manage
 * `TSAN_OPTIONS=suppressions=…` separately.
 *
 * History (2026-05-26):
 * The original session-2 commit shipped with five suppressed
 * substrate-race patterns:
 *   - race:lotus_bus_queue_drain
 *   - race:lotus_arena_new_chunk_for
 *   - race:lotus_arena_destroy
 *   - race:lotus_coop_pool_worker
 *   - race:lotus_chunk_pool_prefill_count
 * All five have since been fixed:
 *   1. `lotus_bus_queue_drain` — `g_bus_has_pinned` extended to
 *      fire when cooperative pool workers are spawned (atomic
 *      load/store).
 *   2. `lotus_arena_destroy` + `lotus_coop_pool_worker` —
 *      per-arena `pthread_mutex_t subregion_lock` protects the
 *      parent's child-slot freelist across concurrent
 *      create_subregion / destroy; codegen-side shutdown_all
 *      hoisted to run before main's arena_destroy.
 *   3. `lotus_arena_new_chunk_for` + `lotus_chunk_pool_prefill_count`
 *      — the env-var-driven lazy-init helpers all moved to
 *      `pthread_once`. No more `static int initialized` data
 *      races.
 *
 * The hook below returns an empty suppression list. If a new
 * race appears in code we choose not to fix immediately, add
 * its `race:<symbol>` line in the function body. Lockfree
 * hashmap entry points (lotus_hashmap_*_lockfree) are
 * intentionally NEVER suppressed — any race surfaced there is
 * a γ-v2 regression and must be fixed.
 */
#if defined(__has_feature)
#  if __has_feature(thread_sanitizer)
#    define LOTUS_TSAN_BUILD 1
#  endif
#endif
#ifdef LOTUS_TSAN_BUILD
const char *__tsan_default_suppressions(void) {
    /* All originally-suppressed substrate races have been fixed
     * as of 2026-05-26 (bus queue multi-thread flag, arena
     * subregion mutex, lazy-init env helpers via pthread_once).
     * Empty string keeps the hook in place for future use — if
     * a workload surfaces a new race in code we choose not to
     * fix immediately, add its `race:<symbol>` line here. */
    return "";
}
#endif
/* LOTUS_ARENA_LOG_BIG_CHUNKS (2026-05-21): backtrace-on-big-
 * alloc diagnostic for hunting unbounded chunk growth in a
 * long-running daemon. <execinfo.h> is glibc-only; the call
 * sites below #ifdef it out for platforms that lack backtrace(). */
#if defined(__GLIBC__)
#include <execinfo.h>
#endif
/* std::process::rss_bytes (2026-05-21): getrusage for the
 * process's resident-set high-water mark. Backs the
 * observability primitive used to verify
 * the Phase-4 method-scratch reclaim actually bounds memory. */
#include <sys/resource.h>
/* F.32-4a/4c (2026-05-24): mlockall (locking pages against
 * paging) and mmap with MAP_HUGETLB (huge-page-backed arena
 * chunks) for latency-critical workloads. Linux-only — the
 * relevant constants come from <sys/mman.h>. MAP_HUGE_2MB
 * lives in <linux/mman.h> which isn't always reachable from
 * <sys/mman.h>; define it locally when missing so we don't
 * have to depend on the kernel headers. The value comes from
 * the Linux ABI: MAP_HUGE_SHIFT = 26, MAP_HUGE_2MB = 21 << 26. */
#include <sys/mman.h>
#ifndef MAP_HUGE_SHIFT
#  define MAP_HUGE_SHIFT 26
#endif
#ifndef MAP_HUGE_2MB
#  define MAP_HUGE_2MB (21 << MAP_HUGE_SHIFT)
#endif
#ifndef MAP_HUGETLB
#  define MAP_HUGETLB 0x40000   /* Linux-specific; fallback for builds where it's missing */
#endif

/* Default chunk size: 64KB. Big enough that most loci fit in
 * one chunk, small enough that a leaf locus that allocates a
 * single ClosureViolation doesn't waste an entire MB. Tunable
 * via env (F.32-3, see lotus_arena_default_chunk_bytes below). */
#define LOTUS_ARENA_CHUNK_BYTES (64 * 1024)

/* F.32-3 (2026-05-25): operator-tunable chunk size override.
 * Set LOTUS_ARENA_CHUNK_BYTES_OVERRIDE=N (bytes, must be a
 * power of 2 in the [4096, 16M] range) to override the
 * default. Useful for multi-locus-per-pool deployments where
 * the default 64K chunks blow past L2-per-core; sized smaller,
 * each locus's hot chunk fits in cache across pool rotations.
 *
 * Reads the env once at first use; cached for the process
 * lifetime. The compile-time `LOTUS_ARENA_CHUNK_BYTES` macro
 * is still used by the chunk pool's discriminator (the pool
 * only retains chunks at the canonical default size); when
 * the override fires, chunks land on the malloc/free path,
 * not the pool. That's acceptable — overriding means the
 * operator picked a non-default and we treat those chunks as
 * "non-pooled" anyway. */
/* 2026-05-26 substrate-race fix: all of these env-var-driven
 * lazy-init helpers used a non-atomic `static int initialized`
 * gate that was a data race under concurrent first-callers.
 * Each one is now wrapped in `pthread_once`, which provides
 * the standard one-shot init primitive with proper happens-
 * before edges. The cost is one pthread_once_t per helper
 * (~8 B); first call pays the mutex pair, subsequent calls
 * are a single atomic load. */
static size_t g_default_chunk_bytes_cached = 0;
static void lotus_arena_default_chunk_bytes_init(void) {
    const char *env = getenv("LOTUS_ARENA_CHUNK_BYTES_OVERRIDE");
    if (env) {
        char *endp = NULL;
        unsigned long v = strtoul(env, &endp, 10);
        /* Validate: power of 2, in [4K, 16M]. */
        if (endp && *endp == '\0'
            && v >= 4096 && v <= (16ul * 1024 * 1024)
            && (v & (v - 1)) == 0)
        {
            g_default_chunk_bytes_cached = (size_t)v;
        }
    }
    if (g_default_chunk_bytes_cached == 0) {
        g_default_chunk_bytes_cached = LOTUS_ARENA_CHUNK_BYTES;
    }
}
static size_t lotus_arena_default_chunk_bytes(void) {
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_once(&once, lotus_arena_default_chunk_bytes_init);
    return g_default_chunk_bytes_cached;
}

typedef struct lotus_arena_chunk {
    struct lotus_arena_chunk *next;
    size_t                    used;
    size_t                    cap;
    /* F.32-4a (2026-05-24): when LOTUS_HUGE_PAGES=1 is set in
     * the environment AND the requested chunk size is >= the
     * huge-page threshold (default 2 MB), the chunk is mmap'd
     * with MAP_HUGETLB | MAP_HUGE_2MB rather than malloc'd.
     * The release path must munmap (not free) — track the
     * allocation method per chunk so destroy picks the right
     * deallocator. `mmap_size` is the full mmap'd region size
     * (header + data); 0 for malloc'd chunks. */
    int                       via_mmap;
    size_t                    mmap_size;
    /* `data` follows in the same allocation — accessed as
     * (char *)(chunk + 1). Inlined-trailing layout means each
     * chunk is one malloc, not two. */
} lotus_arena_chunk_t;

typedef struct lotus_arena {
    lotus_arena_chunk_t *head;
    size_t               default_chunk_size;
    /* m22: sub-region tracking. If `parent` is non-NULL, this
     * arena is a sub-region carved at one of its parent's slots;
     * destroy returns `slot` to the parent's free-list so the
     * next subregion_create can reuse it. Top-level arenas (the
     * program-wide @lotus.arena.global, plus any locus whose
     * parent is not chunked-class) have parent == NULL. */
    struct lotus_arena  *parent;
    int                  slot;
    /* m22: free-list of slot indices for sub-region children
     * (chunked-class). next_slot is the monotonic counter; freed
     * slots get pushed onto free_list and re-handed out before
     * the counter bumps again. free_list grows on demand. */
    int                 *free_list;
    size_t               free_count;
    size_t               free_cap;
    int                  next_slot;
    /* v1.x-3: when set, `lotus_arena_alloc` refuses to malloc a
     * fresh chunk on overflow — it returns NULL instead. Used by
     * recognition-class pools (fixed_cell + shared_slab) where
     * the capacity is a hard budget written down at the locus's
     * projection annotation. fixed_size also flags that the
     * arena struct + head chunk may live INLINE inside a recpool
     * cell (fixed_cell case), so `lotus_arena_destroy` becomes
     * a no-op and codegen routes teardown through the recpool's
     * release entry point instead. */
    int                  fixed_size;
    /* Phase-3 safety net (2026-05-19): if non-zero, the arena's
     * total chunk-byte allocation is capped. `chunk_byte_total`
     * tracks sum of chunk.cap across the live chunk list; the
     * fresh-chunk path in lotus_arena_alloc refuses to grow past
     * `chunk_byte_cap` and returns NULL instead. Used for
     * g_bus_payload_arena so a leaking long-running program
     * crashes loudly with the cap diagnostic on stderr instead
     * of an OOM kill. Zero means unbounded (the default for
     * locus-owned arenas, which are bounded by locus lifecycle
     * already). */
    size_t               chunk_byte_total;
    size_t               chunk_byte_cap;
    /* Human-readable name for the cap diagnostic. NULL means use
     * the generic message. */
    const char          *cap_diag_name;
    /* 2026-05-26 substrate-race fix: mutex protecting the
     * sub-region tracker (`free_list`, `free_count`, `free_cap`,
     * `next_slot`). When two threads concurrently create or
     * destroy children of the SAME parent — common under
     * cross-pool cooperative placement where the worker's
     * handler scratch is a sub-region of the main App arena —
     * unsynchronized reads + writes to the freelist would race
     * (concurrent realloc, double-pop, slot duplication, etc.).
     * The lock is held only across the (small, O(1)) freelist
     * mutation; chunk allocation/copy still runs lock-free on
     * the child's own state.
     *
     * Every arena carries the mutex because ANY arena can become
     * a parent the moment it's passed to
     * `lotus_arena_create_subregion`. Cost: one
     * `pthread_mutex_t` (~40 B) per arena struct + one init/
     * destroy pair. Sub-regions never acquire their OWN lock
     * (they only touch their PARENT's lock), but they carry
     * one anyway for the case they themselves become a parent
     * via a nested sub-region. */
    pthread_mutex_t      subregion_lock;
} lotus_arena_t;

/* Per-thread freelist of default-sized chunks (2026-05-21
 * follow-up to the Phase-4 per-method scratch reclaim). The
 * scratch open/destroy cycle does one malloc + one free per
 * method call; for hot paths (every locus method call) that's
 * ~100–400 ns of overhead even when the body itself does
 * almost nothing. The fix is to keep recently-freed
 * default-sized chunks in a thread-local LRU and hand them
 * back out on the next `lotus_arena_new_chunk` request.
 *
 * Thread-local because chunks must not migrate across
 * schedulers — a chunk freed on one thread that gets handed
 * to another would race with the freeing thread's `used`
 * cursor reset and (worse) hand out memory the donating
 * thread might still be touching during the same call frame.
 * Cap intentionally small: 16 × 64 KiB = 1 MiB per-thread
 * resident overhead. Bigger arenas (which we get when callers
 * pass a size larger than `default_chunk_size`) bypass the
 * pool — only the common-case 64 KiB chunks are recycled,
 * keeping the freelist policy obvious.
 *
 * Not synchronized: `__thread` provides per-thread storage,
 * so each scheduler thread has its own freelist with no
 * contention. The Bus-arena reclaim spec memo names this
 * primitive but defers it; this is that primitive landing
 * for real. */
/* 2026-05-21: bumped from 16 to 256. Per-method scratch reclaim
 * shipping in 7cc4439 made arena create/destroy a hot-path
 * operation (~6 kHz on a real-world recv loop), and a 16-slot
 * cache was missing 99.6% of the time — each miss returned the
 * chunk to glibc heap and contributed to per-thread arena
 * fragmentation. 256 slots × 64 KiB = 16 MiB per-thread
 * resident high-water, which is acceptable for the workloads
 * that benefit from the cache; smaller workloads use far less
 * because the pool grows on demand and only holds what's been
 * released. */
#define LOTUS_CHUNK_POOL_CAP 256

static __thread lotus_arena_chunk_t *
    g_chunk_pool[LOTUS_CHUNK_POOL_CAP];
static __thread int g_chunk_pool_count = 0;

/* LOTUS_CHUNK_POOL_STATS — when set, every chunk
 * acquire / release tallies into per-thread counters that
 * dump to stderr at process exit. Useful for diagnosing
 * "pool isn't recycling" symptoms — pairs hits vs misses,
 * stores vs overflows. The counters are __thread so each
 * scheduler thread reports its own numbers; the atexit
 * handler runs on the thread that registered it (main).
 * Set to 1 (or any non-empty value) to enable. */
static __thread uint64_t g_chunk_pool_hits = 0;
static __thread uint64_t g_chunk_pool_misses = 0;
static __thread uint64_t g_chunk_pool_stores = 0;
static __thread uint64_t g_chunk_pool_overflows = 0;

static int g_chunk_pool_stats_enabled_cached = 0;
static void lotus_chunk_pool_stats_init(void) {
    const char *env = getenv("LOTUS_CHUNK_POOL_STATS");
    g_chunk_pool_stats_enabled_cached =
        env && env[0] && env[0] != '0';
}
static int lotus_chunk_pool_stats_enabled(void) {
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_once(&once, lotus_chunk_pool_stats_init);
    return g_chunk_pool_stats_enabled_cached;
}

static void lotus_chunk_pool_stats_emit(const char *label) {
    fprintf(stderr,
            "[chunk_pool %s tid=%lu] hits=%llu misses=%llu "
            "stores=%llu overflows=%llu pool_size=%d\n",
            label,
            (unsigned long)pthread_self(),
            (unsigned long long)g_chunk_pool_hits,
            (unsigned long long)g_chunk_pool_misses,
            (unsigned long long)g_chunk_pool_stores,
            (unsigned long long)g_chunk_pool_overflows,
            g_chunk_pool_count);
    fflush(stderr);
}

static void lotus_chunk_pool_stats_dump_main(void) {
    if (!lotus_chunk_pool_stats_enabled()) return;
    lotus_chunk_pool_stats_emit("main-thread");
}

/* pthread-key destructor: fires when a non-main thread exits
 * (pthread_exit / return-from-start). Lets per-thread chunk
 * pool counters dump for every scheduler thread, not just the
 * main thread that runs the atexit hook. Triggered by the
 * `mark_thread_for_dtor` helper below, which sets a non-NULL
 * sentinel on first chunk pool touch — subsequent operations
 * skip the setspecific call via the `g_thread_marked` thread-
 * local flag. */
static pthread_key_t g_chunk_pool_stats_thread_key;
static int g_chunk_pool_stats_key_init = 0;
static __thread int g_thread_marked_for_pool_dtor = 0;

static void lotus_chunk_pool_stats_thread_dtor(void *unused) {
    (void)unused;
    if (!lotus_chunk_pool_stats_enabled()) return;
    /* Skip threads that never actually touched the pool — the
     * sentinel was set on first touch, so anything that gets
     * the destructor call had at least one event. The main
     * thread's atexit handler covers itself separately so we
     * don't double-dump. */
    lotus_chunk_pool_stats_emit("thread-exit");
}

static inline void lotus_mark_thread_for_pool_dtor(void) {
    if (g_thread_marked_for_pool_dtor) return;
    if (!g_chunk_pool_stats_key_init) return;
    pthread_setspecific(g_chunk_pool_stats_thread_key, (void *)1);
    g_thread_marked_for_pool_dtor = 1;
}

__attribute__((constructor))
static void lotus_chunk_pool_stats_install(void) {
    if (lotus_chunk_pool_stats_enabled()) {
        atexit(lotus_chunk_pool_stats_dump_main);
        if (pthread_key_create(&g_chunk_pool_stats_thread_key,
                               lotus_chunk_pool_stats_thread_dtor) == 0) {
            g_chunk_pool_stats_key_init = 1;
        }
    }
}

/* LOTUS_ARENA_RESIDENCY (2026-05-22 PM): per-long-lived-arena
 * byte counter, sampled at process exit. Answers the diagnostic
 * question "which long-lived arena's residency is growing under
 * sustained churn?" — LOTUS_ARENA_LOG_BIG_CHUNKS only catches
 * allocation events at the call site, not where the bytes
 * actually live.
 *
 * Enabled by setting `LOTUS_ARENA_RESIDENCY=1` (or any non-zero
 * value). When enabled, every top-level arena (created via
 * `lotus_arena_create` — locus arenas + g_bus_payload_arena,
 * NOT method-scratch subregions) is linked into a global registry
 * at create time with a backtrace captured for the construction
 * site. At process exit (atexit), the dumper walks the live set
 * and emits one line per arena with chunk count, total bytes,
 * parent pointer, and the construction backtrace.
 *
 * Subregions (`lotus_arena_create_subregion`) are intentionally
 * skipped — they're method scratch with method-bounded lifetimes,
 * not the long-lived residency we care about. Their parent's
 * total is what shows up in the dump.
 *
 * Programs that exit via `_exit(N)` skip atexit; callers wanting
 * a dump on signal / panic / explicit checkpoint can invoke
 * `lotus_arena_residency_dump_fd(int fd)` directly. */
/* Backtrace capture depth at arena birth. 24 covers typical Hale
 * call stacks; the first 2 frames (backtrace itself + register
 * helper) are stripped at dump time. Bumped from 8 after a
 * a long-burn produced backtraces that bottomed out in
 * libc-start before reaching any user-meaningful frame — the
 * shallow capture was eating the construction site under
 * inlining + recurse depth. */
#define LOTUS_ARENA_RESIDENCY_BACKTRACE_DEPTH 24

typedef struct lotus_arena_residency_entry {
    struct lotus_arena_residency_entry *next;
    struct lotus_arena *arena;
    int id;
    /* Optional human-readable tag set by lotus_arena_create_labeled
     * (codegen passes the locus name) or by hand for C-side
     * specials (g_bus_payload_arena). NULL when the arena was
     * created via plain lotus_arena_create with no label —
     * caller falls back to the backtrace for identification. */
    const char *label;
#if defined(__GLIBC__)
    void *birth_frames[LOTUS_ARENA_RESIDENCY_BACKTRACE_DEPTH];
    int   birth_frame_count;
#endif
} lotus_arena_residency_entry_t;

static lotus_arena_residency_entry_t *g_arena_residency_head = NULL;
static pthread_mutex_t g_arena_residency_lock = PTHREAD_MUTEX_INITIALIZER;
static _Atomic int g_arena_residency_seq = 0;

static int g_arena_residency_enabled_cached = 0;
static void lotus_arena_residency_enabled_init(void) {
    const char *env = getenv("LOTUS_ARENA_RESIDENCY");
    g_arena_residency_enabled_cached =
        env && env[0] && env[0] != '0';
}
static int lotus_arena_residency_enabled(void) {
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_once(&once, lotus_arena_residency_enabled_init);
    return g_arena_residency_enabled_cached;
}

static void lotus_arena_residency_register(struct lotus_arena *a,
                                            const char *label) {
    if (!lotus_arena_residency_enabled() || !a) return;
    lotus_arena_residency_entry_t *e = (lotus_arena_residency_entry_t *)
        malloc(sizeof(lotus_arena_residency_entry_t));
    if (!e) return;
    e->arena = a;
    e->label = label;
    e->id = atomic_fetch_add_explicit(
        &g_arena_residency_seq, 1, memory_order_relaxed);
#if defined(__GLIBC__)
    e->birth_frame_count = backtrace(
        e->birth_frames, LOTUS_ARENA_RESIDENCY_BACKTRACE_DEPTH);
#endif
    pthread_mutex_lock(&g_arena_residency_lock);
    e->next = g_arena_residency_head;
    g_arena_residency_head = e;
    pthread_mutex_unlock(&g_arena_residency_lock);
}

/* Late-binding label setter — codegen / C-side initializers can
 * call this after lotus_arena_create to attach a human-readable
 * tag to the arena's registry entry. Useful when the label isn't
 * known at create time, or for retrofitting the bus payload
 * arena lazy-init path. No-op when residency logging is disabled
 * or the arena isn't registered. */
void lotus_arena_set_label(struct lotus_arena *a, const char *label);

void lotus_arena_set_label(struct lotus_arena *a, const char *label) {
    if (!lotus_arena_residency_enabled() || !a) return;
    pthread_mutex_lock(&g_arena_residency_lock);
    for (lotus_arena_residency_entry_t *e = g_arena_residency_head;
         e; e = e->next) {
        if (e->arena == a) {
            e->label = label;
            break;
        }
    }
    pthread_mutex_unlock(&g_arena_residency_lock);
}

static void lotus_arena_residency_unregister(struct lotus_arena *a) {
    if (!lotus_arena_residency_enabled() || !a) return;
    pthread_mutex_lock(&g_arena_residency_lock);
    lotus_arena_residency_entry_t **link = &g_arena_residency_head;
    while (*link) {
        if ((*link)->arena == a) {
            lotus_arena_residency_entry_t *gone = *link;
            *link = gone->next;
            free(gone);
            break;
        }
        link = &(*link)->next;
    }
    pthread_mutex_unlock(&g_arena_residency_lock);
}

/* Public dump entry point. Callable from anywhere — atexit hook,
 * signal handler, downstream checkpoint, panic path. Writes one
 * line per still-alive registered arena to the given fd. The
 * dump is sorted descending by total bytes so the heaviest arena
 * surfaces at the top. */
void lotus_arena_residency_dump_fd(int fd);

void lotus_arena_residency_dump_fd(int fd) {
    if (!lotus_arena_residency_enabled()) return;
    pthread_mutex_lock(&g_arena_residency_lock);

    /* Snapshot to an array so we can sort + emit without holding
     * the lock through stdio. Backtrace symbol resolution can
     * touch malloc; holding the lock through it would risk
     * deadlock if a registration races during the dump. */
    size_t n = 0;
    for (lotus_arena_residency_entry_t *e = g_arena_residency_head;
         e; e = e->next) {
        n++;
    }
    lotus_arena_residency_entry_t **arr = (lotus_arena_residency_entry_t **)
        malloc(n * sizeof(lotus_arena_residency_entry_t *));
    if (!arr) {
        pthread_mutex_unlock(&g_arena_residency_lock);
        return;
    }
    size_t i = 0;
    for (lotus_arena_residency_entry_t *e = g_arena_residency_head;
         e; e = e->next) {
        arr[i++] = e;
    }
    pthread_mutex_unlock(&g_arena_residency_lock);

    /* Compute byte totals (sum of chunk caps) for each arena. */
    typedef struct {
        lotus_arena_residency_entry_t *entry;
        size_t bytes;
        size_t chunks;
    } row_t;
    row_t *rows = (row_t *)malloc(n * sizeof(row_t));
    if (!rows) {
        free(arr);
        return;
    }
    for (size_t j = 0; j < n; j++) {
        size_t bytes = 0, chunks = 0;
        for (const lotus_arena_chunk_t *c = arr[j]->arena->head;
             c; c = c->next) {
            chunks++;
            bytes += c->cap;
        }
        rows[j].entry = arr[j];
        rows[j].bytes = bytes;
        rows[j].chunks = chunks;
    }
    /* Insertion sort by bytes descending (n is small — handfuls
     * to tens of arenas in typical use). */
    for (size_t a = 1; a < n; a++) {
        row_t key = rows[a];
        size_t b = a;
        while (b > 0 && rows[b - 1].bytes < key.bytes) {
            rows[b] = rows[b - 1];
            b--;
        }
        rows[b] = key;
    }

    FILE *out = (fd == fileno(stderr)) ? stderr : fdopen(dup(fd), "w");
    if (!out) {
        free(rows);
        free(arr);
        return;
    }
    fprintf(out,
            "[arena_residency dump] %zu live arenas, "
            "sorted by bytes desc:\n", n);
    for (size_t j = 0; j < n; j++) {
        lotus_arena_residency_entry_t *e = rows[j].entry;
        fprintf(out,
                "  [#%d arena=%p label=%s] chunks=%zu bytes=%zu "
                "(%.2f MiB) parent=%p\n",
                e->id, (void *)e->arena,
                e->label ? e->label : "<unlabeled>",
                rows[j].chunks, rows[j].bytes,
                (double)rows[j].bytes / (1024.0 * 1024.0),
                (void *)e->arena->parent);
#if defined(__GLIBC__)
        if (e->birth_frame_count > 2) {
            fflush(out);
            backtrace_symbols_fd(
                e->birth_frames + 2,
                e->birth_frame_count - 2,
                fileno(out));
        }
#endif
    }
    fflush(out);
    if (out != stderr) {
        fclose(out);
    }
    free(rows);
    free(arr);
}

static void lotus_arena_residency_dump_atexit(void) {
    lotus_arena_residency_dump_fd(fileno(stderr));
}

__attribute__((constructor))
static void lotus_arena_residency_install(void) {
    if (lotus_arena_residency_enabled()) {
        atexit(lotus_arena_residency_dump_atexit);
    }
}

/* LOTUS_ARENA_LOG_BIG_CHUNKS (2026-05-21): on first call, parse
 * the env var to a byte threshold. Subsequent calls return the
 * cached threshold. 0 disables logging. The env var value is
 * parsed as decimal bytes; e.g. LOTUS_ARENA_LOG_BIG_CHUNKS=1048576
 * logs every chunk >= 1 MiB. A bare "1" / "on" / "true" defaults
 * to 1 MiB. */
static size_t g_arena_big_chunk_threshold_cached = 0;
static void lotus_arena_big_chunk_threshold_init(void) {
    const char *env = getenv("LOTUS_ARENA_LOG_BIG_CHUNKS");
    if (env && env[0]) {
        if (strcmp(env, "1") == 0
            || strcmp(env, "on") == 0
            || strcmp(env, "true") == 0
            || strcmp(env, "yes") == 0)
        {
            g_arena_big_chunk_threshold_cached = 1024 * 1024;
        } else {
            char *end = NULL;
            unsigned long long v = strtoull(env, &end, 10);
            if (end != env && v > 0) {
                g_arena_big_chunk_threshold_cached = (size_t)v;
            }
        }
    }
}
static size_t lotus_arena_big_chunk_threshold(void) {
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_once(&once, lotus_arena_big_chunk_threshold_init);
    return g_arena_big_chunk_threshold_cached;
}

/* Per-process event cap. Default 200 keeps stderr survivable on
 * a tight-loop workload; the pattern is usually obvious in the
 * first 200 events. Override via LOTUS_ARENA_LOG_BIG_MAX_EVENTS=N
 * (set 0 for unlimited). Parsed once on first use; cached. */
static int g_arena_big_max_events_cached = 200;
static void lotus_arena_big_max_events_init(void) {
    const char *env = getenv("LOTUS_ARENA_LOG_BIG_MAX_EVENTS");
    if (env && env[0]) {
        char *end = NULL;
        long v = strtol(env, &end, 10);
        if (end != env) {
            /* Negative or 0 → unlimited (INT_MAX); positive →
             * exact cap. The "0 = unlimited" convention matches
             * /dev/-style "no limit" sentinels and is what the
             * downstream consumer asked for. */
            g_arena_big_max_events_cached =
                (v <= 0) ? INT_MAX : (int)v;
        }
    }
}
static int lotus_arena_big_max_events(void) {
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_once(&once, lotus_arena_big_max_events_init);
    return g_arena_big_max_events_cached;
}

/* Emit a one-line diagnostic with size, monotonic seqno, and a
 * small backtrace. Capped per `lotus_arena_big_max_events` (200
 * by default; LOTUS_ARENA_LOG_BIG_MAX_EVENTS=0 lifts the cap). */
static void lotus_log_big_alloc_event(const char *label, size_t cap) {
    static _Atomic int seq = 0;
    int n = atomic_fetch_add_explicit(&seq, 1, memory_order_relaxed);
    if (n >= lotus_arena_big_max_events()) return;
    fprintf(stderr,
            "[%s #%d] cap=%zu bytes (%.2f MiB)\n",
            label, n, cap, (double)cap / (1024.0 * 1024.0));
#if defined(__GLIBC__)
    void *frames[12];
    int got = backtrace(frames, 12);
    /* backtrace_symbols_fd writes to a raw fd; bypasses libc
     * locks that might be held by the calling thread. Frame 0
     * is this fn; frame 1 is the wrapper / chunk-emitter;
     * useful call stack starts at frame 2. */
    if (got > 2) {
        backtrace_symbols_fd(frames + 2, got - 2, fileno(stderr));
    }
#endif
    fflush(stderr);
}

static void lotus_arena_log_big_chunk(size_t cap) {
    lotus_log_big_alloc_event("arena_big_chunk", cap);
}

/* LOTUS_ALLOC_LOG_BIG (2026-05-21 follow-up): linker --wrap
 * intercept for the libc allocator entry points. When the env
 * var threshold is set (shares the LOTUS_ARENA_LOG_BIG_CHUNKS
 * setting), every malloc / realloc / calloc / mmap call larger
 * than the threshold logs label + size + backtrace through the
 * shared `lotus_log_big_alloc_event` helper above. Distinct
 * labels per syscall so the report tells you which path fired
 * (e.g. mmap_big vs realloc_big), and so existing arena_big_chunk
 * entries stay easy to grep for.
 *
 * Gated by `-DLOTUS_ENABLE_WRAP_MALLOC` so the main `hale
 * build` clang invocation pulls in the wrappers (and pairs them
 * with `-Wl,--wrap=malloc` etc. so ld renames calls), while
 * sidecar test drivers that compile `lotus_arena.c` directly
 * without the wrap flags don't drag in the `__real_*` references
 * that would otherwise fail to link.
 *
 * The wrapping cost when the env var is unset is one int read +
 * one branch per allocation — rounding error compared to the
 * syscall / heap-walk cost the allocator itself pays. */
#ifdef LOTUS_ENABLE_WRAP_MALLOC
extern void *__real_malloc(size_t size);
extern void *__real_realloc(void *ptr, size_t size);
extern void *__real_calloc(size_t nmemb, size_t size);
extern void *__real_mmap(void *addr, size_t length, int prot,
                         int flags, int fd, off_t offset);

void *__wrap_malloc(size_t size) {
    size_t t = lotus_arena_big_chunk_threshold();
    if (t > 0 && size >= t) {
        lotus_log_big_alloc_event("malloc_big", size);
    }
    return __real_malloc(size);
}

void *__wrap_realloc(void *ptr, size_t size) {
    size_t t = lotus_arena_big_chunk_threshold();
    if (t > 0 && size >= t) {
        lotus_log_big_alloc_event("realloc_big", size);
    }
    return __real_realloc(ptr, size);
}

void *__wrap_calloc(size_t nmemb, size_t size) {
    size_t total = nmemb * size;
    size_t t = lotus_arena_big_chunk_threshold();
    if (t > 0 && total >= t) {
        lotus_log_big_alloc_event("calloc_big", total);
    }
    return __real_calloc(nmemb, size);
}

void *__wrap_mmap(void *addr, size_t length, int prot,
                  int flags, int fd, off_t offset) {
    size_t t = lotus_arena_big_chunk_threshold();
    if (t > 0 && length >= t) {
        lotus_log_big_alloc_event("mmap_big", length);
    }
    return __real_mmap(addr, length, prot, flags, fd, offset);
}
#endif /* LOTUS_ENABLE_WRAP_MALLOC */

/* 2026-05-21 follow-up: glibc malloc tuning hook. The default
 * glibc allocator can create up to 8 × ncpu per-thread arenas,
 * each of which mmaps 64 MiB "heap" segments on demand and
 * accumulates them when long-lived + short-lived allocations
 * interleave (heap fragmentation). On a long-running daemon
 * with stable thread count, this surfaces as continuously
 * growing virtual address space (100+ MB/sec) even though the
 * resident working set stays small (~5 MB per 64 MiB segment).
 *
 * mallopt(M_ARENA_MAX, N) caps the per-thread arena count
 * globally. N=1 forces a single arena (max contention, min
 * virtual bloat). The right N depends on thread count; for the
 * Hale model (cooperative scheduler + a small number of
 * pinned-locus threads) the default ncpu × 8 is overkill.
 *
 * Opt-in via env var so we don't surprise anyone whose
 * workload benefits from the multi-arena default. Set
 * LOTUS_GLIBC_ARENA_MAX=1 to force a single arena; any
 * positive integer N caps the count at N. Unset / 0 keeps
 * the glibc default. Runs at process startup before any user
 * allocation (mallopt for M_ARENA_MAX is only effective
 * before allocations land in extra arenas). */
__attribute__((constructor))
static void lotus_init_glibc_malloc_tuning(void) {
    const char *env = getenv("LOTUS_GLIBC_ARENA_MAX");
    if (!env || !env[0]) return;
    char *end = NULL;
    long v = strtol(env, &end, 10);
    if (end == env || v <= 0) return;
#ifdef M_ARENA_MAX
    mallopt(M_ARENA_MAX, (int)v);
#endif
}

/* 2026-05-21: pre-fill the per-thread pool on first touch with
 * `LOTUS_CHUNK_POOL_PREFILL` chunks. Downstream diagnostic
 * showed 71.5% pool hit rate on a real hot path with a pool
 * that oscillates near empty (pool_size=5 at exit) because
 * alloc-rate and release-rate are tightly coupled in time —
 * the pool's high-water depends on the lag between a release
 * and the next request. Pre-filling moves the steady-state
 * floor up by PREFILL; bursts that would otherwise drain the
 * pool to 0 now drain to (PREFILL - burst_depth), eliminating
 * the misses that surface as 64 KiB malloc churn.
 *
 * PREFILL = 32 → 2 MiB resident per thread that ever touches
 * the pool. Acceptable for the long-lived pinned-thread model
 * Hale targets; large thread-pool workloads would pay
 * 2 MiB × thread count, but those aren't the canonical use
 * case. Configurable via the env var if a workload disagrees. */
#define LOTUS_CHUNK_POOL_PREFILL_DEFAULT 32

static __thread int g_chunk_pool_prefilled = 0;

static int g_chunk_pool_prefill_count_cached =
    LOTUS_CHUNK_POOL_PREFILL_DEFAULT;
static void lotus_chunk_pool_prefill_count_init(void) {
    const char *env = getenv("LOTUS_CHUNK_POOL_PREFILL");
    if (env && env[0]) {
        char *end = NULL;
        long v = strtol(env, &end, 10);
        if (end != env && v >= 0) {
            /* 0 disables; otherwise clamped to pool cap. */
            int count = (int)v;
            if (count > LOTUS_CHUNK_POOL_CAP) {
                count = LOTUS_CHUNK_POOL_CAP;
            }
            g_chunk_pool_prefill_count_cached = count;
        }
    }
}
static int lotus_chunk_pool_prefill_count(void) {
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_once(&once, lotus_chunk_pool_prefill_count_init);
    return g_chunk_pool_prefill_count_cached;
}

static void lotus_chunk_pool_prefill_if_needed(void) {
    if (g_chunk_pool_prefilled) return;
    g_chunk_pool_prefilled = 1;
    int target = lotus_chunk_pool_prefill_count();
    while (g_chunk_pool_count < target) {
        lotus_arena_chunk_t *c = (lotus_arena_chunk_t *)
            malloc(sizeof(lotus_arena_chunk_t) + LOTUS_ARENA_CHUNK_BYTES);
        if (!c) break;
        c->next = NULL;
        c->used = 0;
        c->cap = LOTUS_ARENA_CHUNK_BYTES;
        g_chunk_pool[g_chunk_pool_count++] = c;
    }
}

/* LOTUS_ARENA_LOG_CHUNK_ATTACH (2026-05-22 PM follow-on): logs
 * EVERY chunk attachment to ANY arena — both fresh-malloc and
 * per-thread-pool-recycled paths. The existing
 * LOTUS_ARENA_LOG_BIG_CHUNKS path only fires on the malloc branch,
 * so chunks claimed from the pool (which is the dominant source
 * after the sret-pattern fix shifted method scratch destroys into
 * the recycling lane) are invisible. Enable this when investigating
 * "arena grew N chunks but the trace shows nothing" — set the value
 * to a byte threshold (e.g., 4096) and every chunk >= that size
 * gets a backtrace line. Shares the LOTUS_ARENA_LOG_BIG_MAX_EVENTS
 * cap with the big-chunk logger. */
static size_t g_arena_chunk_attach_threshold_cached = 0;
static void lotus_arena_chunk_attach_threshold_init(void) {
    const char *env = getenv("LOTUS_ARENA_LOG_CHUNK_ATTACH");
    if (env && env[0]) {
        if (strcmp(env, "1") == 0
            || strcmp(env, "on") == 0
            || strcmp(env, "true") == 0
            || strcmp(env, "yes") == 0)
        {
            /* log every chunk attachment */
            g_arena_chunk_attach_threshold_cached = 1;
        } else {
            char *end = NULL;
            unsigned long long v = strtoull(env, &end, 10);
            if (end != env && v > 0) {
                g_arena_chunk_attach_threshold_cached = (size_t)v;
            }
        }
    }
}
static size_t lotus_arena_chunk_attach_threshold(void) {
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_once(&once, lotus_arena_chunk_attach_threshold_init);
    return g_arena_chunk_attach_threshold_cached;
}

/* Resolve a human-readable label for an arena — walks the parent
 * chain to find the root, then looks up the root's residency
 * registry entry. Used by the chunk_attach logger so the trace
 * tells you "this chunk went to <SymbolBook>" instead of just
 * "a chunk was attached somewhere". Returns NULL when residency
 * isn't enabled, the arena's root has no label, or the lookup
 * misses (top-level arenas created via plain lotus_arena_create
 * with no label). The caller substitutes a fallback string. */
static const char *lotus_arena_resolve_label(struct lotus_arena *a) {
    if (!a) return NULL;
    /* cap_diag_name (set on g_bus_payload_arena + recpool slab)
     * wins on any arena in the chain — it's the most specific
     * label we have. Check from leaf up so a subregion's parent's
     * label propagates naturally. */
    struct lotus_arena *cur = a;
    while (cur) {
        if (cur->cap_diag_name) return cur->cap_diag_name;
        cur = cur->parent;
    }
    /* Walk the residency registry for the root arena's label.
     * The registry only holds top-level arenas, so resolve the
     * root first. */
    struct lotus_arena *root = a;
    while (root->parent) root = root->parent;
    if (!lotus_arena_residency_enabled()) return NULL;
    pthread_mutex_lock(&g_arena_residency_lock);
    const char *label = NULL;
    for (lotus_arena_residency_entry_t *e = g_arena_residency_head;
         e; e = e->next) {
        if (e->arena == root) {
            label = e->label;
            break;
        }
    }
    pthread_mutex_unlock(&g_arena_residency_lock);
    return label;
}

/* Like lotus_log_big_alloc_event but also prints the destination
 * arena's resolved label AND a kind=root|sub indicator, so the
 * trace consumer can attribute each chunk attach precisely:
 *
 *   kind=root  → chunk attached to the root (locus-lifetime) arena;
 *                stays attached until the locus dissolves.
 *                This is the leak class — every kind=root chunk_attach
 *                grows the residency dump.
 *   kind=sub   → chunk attached to a subregion (method scratch,
 *                free-fn body, recpool slab); will be recycled to
 *                the per-thread pool when the subregion destroys.
 *                These dominate the trace by volume but don't grow
 *                anything persistent.
 *
 * Filter `kind=root label=<your_arena>` to isolate the actual
 * arena-growing call sites. The label resolution walks the
 * subregion → root chain + the residency registry — only meaningful
 * when LOTUS_ARENA_RESIDENCY=1 (no label registry otherwise). When
 * the lookup misses, the line prints "label=<unknown>" so the
 * trace still parses uniformly. */
static void lotus_log_chunk_attach_event(struct lotus_arena *a,
                                          const char *label_tag,
                                          size_t cap) {
    static _Atomic int seq = 0;
    int n = atomic_fetch_add_explicit(&seq, 1, memory_order_relaxed);
    if (n >= lotus_arena_big_max_events()) return;
    const char *arena_label = lotus_arena_resolve_label(a);
    const char *kind = (a && a->parent) ? "sub" : "root";
    fprintf(stderr,
            "[%s #%d] arena=%p kind=%s label=%s cap=%zu bytes "
            "(%.2f MiB)\n",
            label_tag, n, (void *)a, kind,
            arena_label ? arena_label : "<unknown>",
            cap, (double)cap / (1024.0 * 1024.0));
#if defined(__GLIBC__)
    void *frames[12];
    int got = backtrace(frames, 12);
    if (got > 2) {
        backtrace_symbols_fd(frames + 2, got - 2, fileno(stderr));
    }
#endif
    fflush(stderr);
}

/* 2026-05-26 substrate-race fix: huge-pages env check used to
 * live inline inside `lotus_arena_new_chunk_for` with the same
 * static-int data-race pattern as the other env helpers. Lifted
 * to a pthread_once-wrapped helper here. */
static int g_hugepages_enabled_cached = 0;
static void lotus_hugepages_enabled_init(void) {
    const char *env = getenv("LOTUS_HUGE_PAGES");
    if (env && (env[0] == '1' || env[0] == 't' || env[0] == 'T')) {
        g_hugepages_enabled_cached = 1;
    }
}
static int lotus_hugepages_enabled(void) {
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_once(&once, lotus_hugepages_enabled_init);
    return g_hugepages_enabled_cached;
}

static lotus_arena_chunk_t *lotus_arena_new_chunk_for(
    struct lotus_arena *target, size_t cap)
{
    lotus_mark_thread_for_pool_dtor();
    lotus_chunk_pool_prefill_if_needed();
    /* Pool only the common-case default chunk size. Mixing
     * sizes in one freelist would force a scan per pop. */
    if (cap == LOTUS_ARENA_CHUNK_BYTES && g_chunk_pool_count > 0) {
        lotus_arena_chunk_t *c =
            g_chunk_pool[--g_chunk_pool_count];
        c->next = NULL;
        c->used = 0;
        /* c->cap already == LOTUS_ARENA_CHUNK_BYTES; the pool
         * is invariant on that. */
        g_chunk_pool_hits++;
        size_t attach_threshold = lotus_arena_chunk_attach_threshold();
        if (attach_threshold > 0 && cap >= attach_threshold) {
            lotus_log_chunk_attach_event(
                target, "chunk_attach_pool", cap);
        }
        return c;
    }
    if (cap == LOTUS_ARENA_CHUNK_BYTES) {
        g_chunk_pool_misses++;
    }
    size_t big_threshold = lotus_arena_big_chunk_threshold();
    if (big_threshold > 0 && cap >= big_threshold) {
        lotus_arena_log_big_chunk(cap);
    }
    size_t attach_threshold = lotus_arena_chunk_attach_threshold();
    if (attach_threshold > 0 && cap >= attach_threshold
        && (big_threshold == 0 || cap < big_threshold))
    {
        /* Only fire if the big-chunk logger didn't already
         * cover this allocation — avoid double-logging the
         * same chunk through both env vars. */
        lotus_log_chunk_attach_event(
            target, "chunk_attach_malloc", cap);
    }
    /* F.32-4a (2026-05-24): for chunks >= huge-page threshold
     * (2 MB), try mmap with MAP_HUGETLB | MAP_HUGE_2MB to back
     * the allocation with 2 MB pages. Reduces TLB pressure by
     * ~512x for big working sets — HFT-grade order books and
     * large @form(hashmap) registries see meaningful win. Falls
     * back to malloc cleanly when:
     *   - LOTUS_HUGE_PAGES is unset / false
     *   - chunk size is < 2 MB (waste of physical memory)
     *   - mmap fails (kernel huge-page pool exhausted or
     *     CAP_IPC_LOCK / sysctl vm.nr_hugepages not set up)
     *
     * Operator prereq: `sysctl -w vm.nr_hugepages=N` to reserve
     * N huge pages in the kernel pool. Without this, the mmap
     * call returns MAP_FAILED with errno=ENOMEM and we fall
     * back to regular malloc — the program still works, just
     * without the TLB-pressure win. */
    if (lotus_hugepages_enabled() && cap >= (2 * 1024 * 1024)) {
        size_t total = sizeof(lotus_arena_chunk_t) + cap;
        /* Round up to 2 MB for the mmap call — huge-page
         * allocations must be page-multiples. */
        size_t mmap_size = (total + (2 * 1024 * 1024 - 1))
                         & ~((size_t)(2 * 1024 * 1024 - 1));
        void *p = mmap(NULL, mmap_size,
                       PROT_READ | PROT_WRITE,
                       MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB | MAP_HUGE_2MB,
                       -1, 0);
        if (p != MAP_FAILED) {
            lotus_arena_chunk_t *c = (lotus_arena_chunk_t *)p;
            c->next = NULL;
            c->used = 0;
            c->cap  = cap;
            c->via_mmap = 1;
            c->mmap_size = mmap_size;
            return c;
        }
        /* mmap failed — fall through to malloc. Suppress the
         * diagnostic (would spam on every chunk grow); a single
         * warning at startup would be better, but the operator
         * can detect failures via `perf stat` (TLB-miss count
         * stays high) or by inspecting LOTUS_CHUNK_POOL_STATS. */
    }
    lotus_arena_chunk_t *c =
        (lotus_arena_chunk_t *)malloc(sizeof(lotus_arena_chunk_t) + cap);
    if (!c) return NULL;
    c->next = NULL;
    c->used = 0;
    c->cap  = cap;
    c->via_mmap = 0;
    c->mmap_size = 0;
    return c;
}

/* Backwards-compat shim for the slab-init path which doesn't have
 * an arena_t at the call site. Logs without a target arena (label
 * resolution will see the NULL and print "<unknown>"). */
static lotus_arena_chunk_t *lotus_arena_new_chunk(size_t cap) {
    return lotus_arena_new_chunk_for(NULL, cap);
}

/* Symmetric: return a chunk to the thread-local pool, or free
 * it via libc if the pool is full or the chunk isn't default-
 * sized. Called by `lotus_arena_destroy` for every chunk in
 * the dying arena's list. */
static void lotus_arena_release_chunk(lotus_arena_chunk_t *c) {
    if (!c) return;
    lotus_mark_thread_for_pool_dtor();
    /* F.32-4a (2026-05-24): huge-page-backed chunks bypass the
     * pool entirely (they're never default-sized; pool only
     * holds LOTUS_ARENA_CHUNK_BYTES chunks) and need munmap
     * not free. Check first so we don't accidentally free()
     * an mmap'd region. */
    if (c->via_mmap) {
        munmap(c, c->mmap_size);
        return;
    }
    if (c->cap == LOTUS_ARENA_CHUNK_BYTES
        && g_chunk_pool_count < LOTUS_CHUNK_POOL_CAP)
    {
        g_chunk_pool[g_chunk_pool_count++] = c;
        g_chunk_pool_stores++;
        return;
    }
    if (c->cap == LOTUS_ARENA_CHUNK_BYTES) {
        g_chunk_pool_overflows++;
    }
    free(c);
}

static inline size_t lotus_align_up(size_t n, size_t a) {
    return (n + a - 1) & ~(a - 1);
}

/* Const template for cheap, call-free initialization of a per-arena
 * subregion_lock (see lotus_arena_alloc_struct). A statically-
 * initialized default mutex is lockable and destroyable like one
 * from pthread_mutex_init(NULL). */
static const pthread_mutex_t LOTUS_SUBREGION_MUTEX_INIT =
    PTHREAD_MUTEX_INITIALIZER;

static lotus_arena_t *lotus_arena_alloc_struct(void) {
    lotus_arena_t *a = (lotus_arena_t *)malloc(sizeof(lotus_arena_t));
    if (!a) return NULL;
    a->default_chunk_size = lotus_arena_default_chunk_bytes();
    /* 2026-05-21: defer the initial chunk to the first
     * `lotus_arena_alloc` call. Per-method scratch reclaim
     * makes arena create/destroy a hot-path op; the dominant
     * shape (`DurationInt.to_ns`, `__json_find_field_raw` /
     * `peek_header` returning early, etc.) is "open scratch,
     * do non-allocating work, close scratch" — eagerly
     * mallocing a 64 KiB chunk for those is pure waste. The
     * head-NULL path in `lotus_arena_alloc` is the same code
     * the head-full path takes (allocate a fresh chunk, make
     * it head), so the runtime cost on actually-allocating
     * scratches is unchanged. */
    a->head = NULL;
    a->parent     = NULL;
    a->slot       = -1;
    a->free_list  = NULL;
    a->free_count = 0;
    a->free_cap   = 0;
    a->next_slot  = 0;
    a->fixed_size = 0;
    a->chunk_byte_total = 0;
    a->chunk_byte_cap   = 0;
    a->cap_diag_name    = NULL;
    /* 2026-05-26 substrate-race fix: subregion freelist mutex.
     * Used by create_subregion / destroy via the PARENT arena's
     * pointer; arenas with no children never acquire the lock.
     * Init via a const PTHREAD_MUTEX_INITIALIZER copy rather than
     * pthread_mutex_init() — a plain store of a constant, no libc
     * call — since arena create is a hot-path op (every method
     * scratch + every locus). The result is a usable default
     * mutex, lockable later if the program goes multithreaded.
     * (perf opt, 2026-05-29.) */
    a->subregion_lock = LOTUS_SUBREGION_MUTEX_INIT;
    return a;
}

/* Public ABI ---------------------------------------------------- */

lotus_arena_t *lotus_arena_create(void) {
    lotus_arena_t *a = lotus_arena_alloc_struct();
    /* Top-level arenas (no parent) are the long-lived residency
     * targets — locus __arenas, g_bus_payload_arena, anything
     * not bounded by a method's exit. Register them so the
     * LOTUS_ARENA_RESIDENCY dump can attribute bytes back to
     * the construction site. Subregions are skipped in
     * `lotus_arena_create_subregion`. */
    lotus_arena_residency_register(a, NULL);
    return a;
}

/* Codegen entry point for locus arena creation — same as
 * lotus_arena_create but stashes `label` on the residency entry
 * so the dump emits a human-readable tag (typically the locus
 * name) instead of just an address + backtrace. The label
 * pointer must outlive the arena; codegen passes a global string
 * literal so this is automatic. */
lotus_arena_t *lotus_arena_create_labeled(const char *label) {
    lotus_arena_t *a = lotus_arena_alloc_struct();
    lotus_arena_residency_register(a, label);
    return a;
}

/* F.32-3 (2026-05-25): codegen-aware sized arena creation.
 * Same shape as lotus_arena_create_labeled but consults the
 * caller's chunk-size hint. Codegen emits this variant for
 * loci instantiated on a non-`main` cooperative pool, passing
 * a hint computed at compile time from loci-per-pool count so
 * each locus's hot chunk fits within its share of the worker
 * thread's L2 slice.
 *
 * The hint is advisory:
 *
 *  - Out-of-range values (< 4096, larger than env-default,
 *    non-power-of-2) silently fall back to the env-default
 *    that `lotus_arena_alloc_struct` already stamped. The
 *    codegen side emits clean values; this is defense.
 *
 *  - The env override (LOTUS_ARENA_CHUNK_BYTES_OVERRIDE) wins
 *    via the upper bound: if the operator picked a default
 *    smaller than the codegen hint, the hint is rejected and
 *    the env value stands. The codegen hint can only shrink
 *    chunks below the env default, never enlarge them.
 *
 * Net effect: for small N (loci-per-pool <= 8) the codegen
 * hint equals the default and this is a no-op call. For
 * N >= 16 the hint is materially smaller (32K / 16K / 8K)
 * and the per-locus working set drops into the L2-per-core
 * envelope. */
lotus_arena_t *lotus_arena_create_labeled_sized(
    const char *label, size_t initial_chunk_bytes)
{
    lotus_arena_t *a = lotus_arena_alloc_struct();
    if (!a) return NULL;
    size_t def = a->default_chunk_size;  /* env-resolved */
    size_t hint = initial_chunk_bytes;
    if (hint >= 4096 && hint <= def && (hint & (hint - 1)) == 0) {
        a->default_chunk_size = hint;
    }
    lotus_arena_residency_register(a, label);
    return a;
}

/* Single-thread fast-path latch for the subregion freelist lock
 * (perf opt, 2026-05-29). The `subregion_lock` mutex exists only
 * to serialize concurrent subregion create/destroy on the SAME
 * parent across threads (the 2026-05-26 race fix). A program that
 * never spawns a second thread can never hit that race, so the
 * uncontended lock/unlock is pure overhead — measurable on hot
 * instantiation loops (locus_instantiation regressed ~91% when
 * the mutex landed). This latch lets the lock sites skip the
 * mutex until the program goes multithreaded.
 *
 * Correctness: the flag only ever transitions 0 -> 1, and
 * `lotus_mark_multithreaded` is called BEFORE every `pthread_create`
 * (coop-pool workers, transport reader threads, pinned-locus
 * spawns). The FIRST such transition therefore happens while only
 * the main thread exists and is inside spawn code (not an arena
 * op) — so no in-flight arena op on any thread observes the
 * transition mid-op. Every subsequent spawn finds the flag already
 * latched, so all threads are already taking the lock. Each lock
 * site reads the flag once into a local and uses that one value
 * for both the lock and the unlock, so a create/destroy op is
 * internally consistent. */
static int g_runtime_multithreaded = 0;

void lotus_mark_multithreaded(void) {
    __atomic_store_n(&g_runtime_multithreaded, 1, __ATOMIC_RELEASE);
}

static inline int lotus_runtime_multithreaded(void) {
    return __atomic_load_n(&g_runtime_multithreaded, __ATOMIC_ACQUIRE);
}

/* Carve a sub-region of `parent`. The sub-region holds its own
 * chunk list (independent allocation lifetime is *bounded* by
 * the parent's, but the chunks themselves are separate from the
 * parent's chunks — m22 doesn't yet pool memory across siblings).
 *
 * The point of this entry point vs. plain `lotus_arena_create()`
 * is the bookkeeping: we get a slot number from the parent's
 * free-list / counter, and `lotus_arena_destroy` returns that
 * slot when this sub-region dies. The free-list keeps the
 * parent's slot space O(peak children alive), not O(total
 * children ever accepted). */
lotus_arena_t *lotus_arena_create_subregion(lotus_arena_t *parent) {
    if (!parent) return lotus_arena_create();
    lotus_arena_t *a = lotus_arena_alloc_struct();
    if (!a) return NULL;
    a->parent = parent;
    /* 2026-05-26 substrate-race fix: lock the parent's
     * subregion tracker. Without this, two threads
     * concurrently creating sub-regions of the same parent
     * would race on the free_list pop OR on next_slot++,
     * potentially handing the same slot index to both
     * children. The lock window is O(1) — a freelist pop or
     * an int increment — so contention is bounded even with
     * many concurrent producers. */
    int mt = lotus_runtime_multithreaded();
    if (mt) pthread_mutex_lock(&parent->subregion_lock);
    if (parent->free_count > 0) {
        a->slot = parent->free_list[--parent->free_count];
    } else {
        a->slot = parent->next_slot++;
    }
    if (mt) pthread_mutex_unlock(&parent->subregion_lock);
    return a;
}

/* Compute the offset within `c` that yields a pointer aligned to
 * `align` for the request at `c->used`. The chunk's data region
 * starts immediately after the lotus_arena_chunk_t header
 * (3 * sizeof(void*) = 24 bytes on x86_64 LP64), so the data
 * base is 8-byte aligned but NOT 16-byte aligned. Aligning the
 * raw offset (lotus_align_up(used, align)) IS NOT enough — the
 * returned pointer is `base + off`, and `base` carries its own
 * misalignment that the bare offset-alignment ignores. Align the
 * actual pointer address instead. Bug 2026-05-20: original
 * Decimal-in-struct segfault root cause — Decimal (i128) stores
 * in struct fields used `movaps`, which traps on 8-byte-aligned
 * destinations. */
static inline size_t lotus_arena_off_for(
    const lotus_arena_chunk_t *c, size_t align)
{
    uintptr_t base = (uintptr_t)(c + 1);
    uintptr_t cursor = base + c->used;
    uintptr_t aligned = lotus_align_up(cursor, align);
    return (size_t)(aligned - base);
}

void *lotus_arena_alloc(lotus_arena_t *a, size_t size, size_t align) {
    if (!a) return NULL;
    if (size == 0) size = 1;        /* every alloc gets a unique addr */
    if (align == 0) align = 8;      /* default 8-byte alignment */

    lotus_arena_chunk_t *c = a->head;
    /* 2026-05-21: lazy initial chunk. `lotus_arena_alloc_struct`
     * leaves head == NULL so per-method scratches that never
     * allocate cost zero mallocs end-to-end. The first call
     * here falls through to the fresh-chunk path below by
     * forcing the "doesn't fit" branch. Subsequent reads of
     * head are guaranteed non-null because the path either
     * filled head from the pool or just mallocd a new chunk. */
    size_t off;
    int needs_fresh;
    if (!c) {
        needs_fresh = 1;
        off = 0;  /* unused; silences may-be-uninitialized */
    } else {
        off = lotus_arena_off_for(c, align);
        needs_fresh = (off + size > c->cap);
    }
    if (needs_fresh) {
        /* v1.x-3: recognition-class pools mark the arena
         * `fixed_size` — the cell's capacity is the budget
         * spelled at the locus's projection annotation, and
         * silently mallocing a fresh chunk would defeat that.
         * Return NULL; the caller (codegen-emitted body code in
         * v1.x-3 PR4+) routes this into the closure-violation
         * channel via lotus_root_panic. */
        if (a->fixed_size) return NULL;
        /* Need a fresh chunk. Size it to cover this single
         * request if the request itself is larger than the
         * default; otherwise use the default. The new chunk
         * becomes the head, so subsequent small allocs land
         * in it (and we don't bother trying to fit them into
         * older chunks — keeps the bump fast and the lookup
         * O(1)). */
        size_t need = size + align;
        size_t cap  = need > a->default_chunk_size
                          ? need
                          : a->default_chunk_size;
        /* Phase-3 safety net: if a byte-cap is set, refuse to
         * grow the arena past it. Returns NULL; the caller is
         * responsible for surfacing the failure (the existing
         * lotus_bytes_alloc_fail_sentinel / empty_global paths
         * already do this for most stdlib callers; m70
         * deserialize fns will surface a corrupted payload that
         * the F.27 routing should pick up). dprintf a one-line
         * diagnostic on first hit so the cap event is visible
         * in production logs. */
        if (a->chunk_byte_cap > 0 &&
            a->chunk_byte_total + cap > a->chunk_byte_cap) {
            static int diag_emitted = 0;
            if (!diag_emitted) {
                diag_emitted = 1;
                const char *name = a->cap_diag_name
                    ? a->cap_diag_name : "(unnamed arena)";
                dprintf(2,
                    "lotus: arena cap hit (%s): %zu / %zu bytes; "
                    "subsequent allocations will return NULL. "
                    "This is a hard cap meant to surface leaks "
                    "early; investigate which producer is "
                    "depositing without an owner reclaiming. "
                    "See spec/memory.md \xc2\xa7 m20 / Phase-2 (4).\n",
                    name, a->chunk_byte_total, a->chunk_byte_cap);
            }
            return NULL;
        }
        lotus_arena_chunk_t *fresh = lotus_arena_new_chunk_for(a, cap);
        if (!fresh) return NULL;
        a->chunk_byte_total += fresh->cap;
        fresh->next = c;
        a->head = fresh;
        c = fresh;
        off = lotus_arena_off_for(c, align);
    }

    char *base = (char *)(c + 1);
    void *p    = base + off;
    c->used    = off + size;
    return p;
}

void lotus_arena_destroy(lotus_arena_t *a) {
    if (!a) return;

    /* m22: if this is a sub-region, return its slot to the
     * parent's free-list so a future create_subregion can reuse
     * it. Grow the free_list capacity as needed (doubling).
     * The parent itself stays alive — only the SUB-region's
     * chunks + struct go away here.
     *
     * 2026-05-26 substrate-race fix: lock the parent's
     * subregion tracker around the freelist push + realloc.
     * Without this, two threads concurrently destroying
     * sub-regions of the same parent would race on the
     * realloc (one's free might run on the same pointer the
     * other's realloc returned), on the cap doubling, and
     * on the freelist write. The lock window is O(1) (the
     * realloc happens at most every 2^N grows). */
    if (a->parent) {
        lotus_arena_t *p = a->parent;
        int mt = lotus_runtime_multithreaded();
        if (mt) pthread_mutex_lock(&p->subregion_lock);
        if (p->free_count == p->free_cap) {
            size_t new_cap = p->free_cap == 0 ? 8 : p->free_cap * 2;
            int *new_list  = (int *)realloc(p->free_list,
                                            new_cap * sizeof(int));
            if (new_list) {
                p->free_list = new_list;
                p->free_cap  = new_cap;
            }
            /* If realloc failed, we silently drop the slot —
             * functionally correct (slot never gets reused) but
             * causes parent's slot space to grow. Acceptable
             * graceful-degradation for v0. */
        }
        if (p->free_count < p->free_cap) {
            p->free_list[p->free_count++] = a->slot;
        }
        if (mt) pthread_mutex_unlock(&p->subregion_lock);
    }

    /* LOTUS_ARENA_RESIDENCY: drop the registry entry (if any)
     * before freeing the arena struct. Subregions never
     * registered so the unregister walk no-ops on them; the
     * registry's linear scan is fine here since arena destroys
     * are rare and unregister is only meaningful for the env-var
     * enabled diagnostic path anyway. */
    lotus_arena_residency_unregister(a);

    lotus_arena_chunk_t *c = a->head;
    while (c) {
        lotus_arena_chunk_t *next = c->next;
        lotus_arena_release_chunk(c);
        c = next;
    }
    /* 2026-05-26 substrate-race fix: a->free_list may have been
     * realloc'd by a concurrent child destroy on another thread
     * just before we got here. Take the subregion lock once to
     * synchronize-with that write before we free the buffer.
     * Holding/releasing the lock immediately is a "happens-before
     * barrier" — it doesn't keep new children from arriving (the
     * caller is responsible for that ordering via
     * lotus_coop_pool_shutdown_all), but it ensures we see the
     * final committed value of free_list. */
    int mt_barrier = lotus_runtime_multithreaded();
    if (mt_barrier) pthread_mutex_lock(&a->subregion_lock);
    int *fl = a->free_list;
    a->free_list = NULL;
    if (mt_barrier) pthread_mutex_unlock(&a->subregion_lock);
    if (fl) free(fl);
    /* Tear down the per-arena subregion lock. By the time we
     * reach here the parent's lock (if any) has already been
     * released and no future caller can reach this arena's
     * lock — the destroying thread is exclusive on its own
     * arena struct. Skip when single-threaded: the mutex was
     * statically initialized and never locked, so a default
     * mutex holds no resources to release (perf opt, 2026-05-29;
     * reuses mt_barrier read above). */
    if (mt_barrier) pthread_mutex_destroy(&a->subregion_lock);
    free(a);
}

/*
 * v1.x-3 — Recognition projection class pools.
 *
 * Recognition is the projection class for "I expect many siblings,
 * each shaped the same, with bounded per-child state." The locus
 * annotation
 *     : projection recognition(cap=N, <sub-mode>)
 * commits to a storage discipline at the declaration site, and the
 * sub-mode picks the allocator strategy at codegen time. v1 ships
 * two sub-modes; the other two parse + typecheck but reject at
 * codegen (mirrors the v1.x-4 / v1.x-4b surface-then-runtime split).
 *
 * fixed_cell(bytes=K): cap_count cells of K payload bytes each,
 *   pre-allocated as one contiguous block; bitmap-tracked. Each
 *   cell carries an INLINE lotus_arena_t + chunk header at its
 *   front, so the cell IS the child's arena — child body code
 *   treats the returned pointer as a regular lotus_arena_t* and
 *   the existing arena_alloc path bumps the in-cell bump pointer.
 *   Overflow returns NULL from arena_alloc (caller routes to the
 *   closure-violation channel). Release clears the bit; the slot
 *   is reusable. Whole block frees at parent dissolve.
 *
 * shared_slab(bytes=K): one fixed_size lotus_arena_t whose initial
 *   chunk is K bytes. Every acquire returns the SAME arena pointer
 *   — children share a bump space, so per-child release is a no-op
 *   and child structs + arena allocations interleave in the slab.
 *   Whole slab frees at parent dissolve. cap_count is recorded but
 *   not enforced at the C layer (codegen's birth-cap check is what
 *   limits concurrent children — the slab is a memory budget, not
 *   a child-count budget).
 *
 * In both cases the arena returned by acquire has `fixed_size=1`,
 * so `lotus_arena_alloc` refuses to grow on overflow. The codegen
 * dispatch (PR4) is responsible for emitting the matching
 * recpool_release at child dissolve and recpool_destroy at parent
 * dissolve instead of the regular lotus_arena_destroy — the cell
 * memory is owned by the recpool, not by the child's arena handle.
 *
 * Spec: spec/recognition.md (v1.x-3 PR6 ships the canonical doc).
 */

#include <assert.h>

typedef struct lotus_recpool_fixed {
    size_t    cap_count;     /* number of cells */
    size_t    cell_bytes;    /* user-facing payload bytes per cell */
    size_t    cell_stride;   /* total per-cell stride incl. inline header */
    size_t    bitmap_words;  /* number of uint64_t words in `bitmap` */
    uint64_t *bitmap;        /* 1 bit per cell; 1 = occupied */
    char     *cells;         /* cap_count * cell_stride bytes */
} lotus_recpool_fixed_t;

typedef struct lotus_recpool_slab {
    size_t         cap_count;   /* recorded, not enforced here (see codegen) */
    size_t         slab_bytes;
    lotus_arena_t *slab_arena;  /* fixed_size=1; never grows */
} lotus_recpool_slab_t;

/* Per-cell stride: inline lotus_arena_t + inline chunk header +
 * payload, rounded up to 16 bytes so the next cell is also 16-byte
 * aligned (the arena_alloc default align is 8; bumping to 16 covers
 * SSE/struct alignment without effort). */
static size_t lotus_recpool_compute_stride(size_t cell_bytes) {
    size_t raw = sizeof(lotus_arena_t)
               + sizeof(lotus_arena_chunk_t)
               + cell_bytes;
    return lotus_align_up(raw, 16);
}

/* Initialize the inline arena+chunk at the head of a cell so that
 * arena_alloc treats the rest of the cell as the bump space. The
 * cell layout is:
 *     [ lotus_arena_t | lotus_arena_chunk_t | cell_bytes payload ]
 * The arena's `head` points at the inline chunk; the chunk's data
 * lives at (chunk+1), which lands on the payload region. */
static void lotus_recpool_init_cell_arena(char *cell_base, size_t cell_bytes) {
    lotus_arena_t *a = (lotus_arena_t *)cell_base;
    lotus_arena_chunk_t *c =
        (lotus_arena_chunk_t *)(cell_base + sizeof(lotus_arena_t));
    c->next  = NULL;
    c->used  = 0;
    c->cap   = cell_bytes;

    a->head               = c;
    a->default_chunk_size = cell_bytes;  /* irrelevant when fixed_size=1 */
    a->parent             = NULL;
    a->slot               = -1;
    a->free_list          = NULL;
    a->free_count         = 0;
    a->free_cap           = 0;
    a->next_slot          = 0;
    a->fixed_size         = 1;
}

static size_t lotus_recpool_bitmap_words_for(size_t cap_count) {
    return (cap_count + 63) / 64;
}

/* Forward scan: find the lowest-index zero bit, or -1 if all set
 * up to cap_count. Uses ctzll on the inverted word for O(1) per
 * word; the bitmap is small enough (cap ~ 100s) that the loop is
 * fine without SIMD. */
static int lotus_recpool_bitmap_first_zero(uint64_t *bm,
                                           size_t words,
                                           size_t cap_count) {
    for (size_t w = 0; w < words; w++) {
        uint64_t inv = ~bm[w];
        if (inv == 0) continue;
        int b = __builtin_ctzll(inv);
        size_t slot = w * 64 + (size_t)b;
        if (slot >= cap_count) return -1;
        return (int)slot;
    }
    return -1;
}

/* fixed_cell ---------------------------------------------------- */

lotus_recpool_fixed_t *lotus_recpool_fixed_create(size_t cap_count,
                                                  size_t cell_bytes) {
    if (cap_count == 0 || cell_bytes == 0) return NULL;
    lotus_recpool_fixed_t *p =
        (lotus_recpool_fixed_t *)malloc(sizeof(lotus_recpool_fixed_t));
    if (!p) return NULL;
    p->cap_count    = cap_count;
    p->cell_bytes   = cell_bytes;
    p->cell_stride  = lotus_recpool_compute_stride(cell_bytes);
    p->bitmap_words = lotus_recpool_bitmap_words_for(cap_count);
    p->bitmap       = (uint64_t *)calloc(p->bitmap_words, sizeof(uint64_t));
    if (!p->bitmap) { free(p); return NULL; }
    p->cells = (char *)malloc(cap_count * p->cell_stride);
    if (!p->cells) { free(p->bitmap); free(p); return NULL; }
    return p;
}

lotus_arena_t *lotus_recpool_fixed_acquire(lotus_recpool_fixed_t *p) {
    if (!p) return NULL;
    int slot = lotus_recpool_bitmap_first_zero(p->bitmap,
                                               p->bitmap_words,
                                               p->cap_count);
    if (slot < 0) return NULL;
    p->bitmap[slot / 64] |= ((uint64_t)1 << (slot % 64));
    char *cell_base = p->cells + (size_t)slot * p->cell_stride;
    lotus_recpool_init_cell_arena(cell_base, p->cell_bytes);
    return (lotus_arena_t *)cell_base;
}

void lotus_recpool_fixed_release(lotus_recpool_fixed_t *p,
                                 lotus_arena_t *arena) {
    if (!p || !arena) return;
    char *base = (char *)arena;
    if (base < p->cells) return;
    size_t off = (size_t)(base - p->cells);
    if (off % p->cell_stride != 0) return;
    size_t slot = off / p->cell_stride;
    if (slot >= p->cap_count) return;
    p->bitmap[slot / 64] &= ~((uint64_t)1 << (slot % 64));
    /* Cell content stays valid-looking until the next acquire
     * re-initializes the inline arena. No memset — matches the
     * existing Pool free-list contract (caller of acquire is
     * responsible for treating the cell as freshly-allocated). */
}

void lotus_recpool_fixed_destroy(lotus_recpool_fixed_t *p) {
    if (!p) return;
    free(p->cells);
    free(p->bitmap);
    free(p);
}

/* shared_slab --------------------------------------------------- */

lotus_recpool_slab_t *lotus_recpool_slab_create(size_t cap_count,
                                                size_t slab_bytes) {
    if (slab_bytes == 0) return NULL;
    lotus_recpool_slab_t *p =
        (lotus_recpool_slab_t *)malloc(sizeof(lotus_recpool_slab_t));
    if (!p) return NULL;
    p->cap_count  = cap_count;
    p->slab_bytes = slab_bytes;
    /* Build the slab arena with one initial chunk sized to the
     * user-spelled budget, then mark it fixed_size=1 so arena_alloc
     * never mallocs a fresh chunk on overflow. */
    lotus_arena_t *a =
        (lotus_arena_t *)malloc(sizeof(lotus_arena_t));
    if (!a) { free(p); return NULL; }
    a->head = lotus_arena_new_chunk(slab_bytes);
    if (!a->head) { free(a); free(p); return NULL; }
    a->default_chunk_size = slab_bytes;
    a->parent             = NULL;
    a->slot               = -1;
    a->free_list          = NULL;
    a->free_count         = 0;
    a->free_cap           = 0;
    a->next_slot          = 0;
    a->fixed_size         = 1;
    p->slab_arena = a;
    return p;
}

lotus_arena_t *lotus_recpool_slab_acquire(lotus_recpool_slab_t *p) {
    if (!p) return NULL;
    /* Every child shares the same slab arena. Sibling allocations
     * interleave; per-child release is a no-op. The cap_count from
     * the locus annotation bounds the number of concurrent children
     * via codegen's accept-side check; the C layer doesn't track it. */
    return p->slab_arena;
}

void lotus_recpool_slab_release(lotus_recpool_slab_t *p,
                                lotus_arena_t *arena) {
    /* No-op by design — the slab is freed wholesale at parent
     * dissolve via lotus_recpool_slab_destroy. */
    (void)p;
    (void)arena;
}

void lotus_recpool_slab_destroy(lotus_recpool_slab_t *p) {
    if (!p) return;
    if (p->slab_arena) {
        /* arena_destroy walks the chunk list and frees each chunk
         * + the arena struct itself. The slab arena has one chunk
         * (it never grew, because fixed_size=1), so this frees the
         * slab cleanly. */
        lotus_arena_destroy(p->slab_arena);
    }
    free(p);
}

/*
 * F.22 capacity slot — Pool of T (fixed-size cell recycling).
 *
 * A pool holds a singly-linked list of chunks; each chunk is one
 * malloc holding N contiguous cells. Live cells are handed out
 * via acquire(); released cells get pushed onto an embedded
 * free-list (each free cell stores the next-free pointer at its
 * own base). When acquire() finds an empty free-list, it grows
 * by malloc'ing a fresh chunk and threading its cells onto the
 * list.
 *
 * Lifetime: wholesale teardown at slot destroy; individual
 * acquire/release rolls memory through the population without
 * touching the OS. The locus's parent arena is irrelevant — Pool
 * owns its own chunks and frees them in destroy.
 *
 * Cell stride = max(cell_size, sizeof(void*)) aligned to
 * cell_align. The sizeof(void*) floor ensures the embedded
 * free-list pointer fits inside any free cell, even if T's
 * own size is smaller than a pointer (e.g. Int8 in a future
 * narrow-int extension).
 *
 * Chunks grow geometrically (16, 32, 64, ...) capped at 4096
 * cells so peak-cells-alive populations don't all malloc on the
 * same boundary. The cap is tunable; the geometric ramp matches
 * the arena's "one big chunk amortizes many small allocs"
 * principle adapted to fixed-stride cells.
 *
 * v1.x-17: initial chunk size adapts to the host page size at
 * runtime — when one full page fits more than 16 cells of T,
 * the initial chunk holds page_size / cell_stride cells (so
 * the chunk is approximately one page including the chunk
 * header) instead of a hardcoded 16. Tiny T (single-byte cells
 * etc.) get a tighter initial chunk than the static 16 would
 * produce; large T (cell_stride > page/16) keep the static 16.
 * Falls back to LOTUS_POOL_INITIAL_CELLS when sysconf is
 * unavailable or returns nonsense.
 *
 * Spec: spec/design-rationale.md §F.22 — "Pool of T — *I hold
 * a bounded shape of recyclable state.*"
 */

#define LOTUS_POOL_INITIAL_CELLS 16
#define LOTUS_POOL_MAX_CHUNK_CELLS 4096

/* v1.x-17: page-size-aware initial chunk sizing. Cached after
 * first sysconf — page size doesn't change during program
 * lifetime, so a one-shot global is fine without locking
 * (the only race window writes the same value).
 */
static size_t lotus_host_page_size(void) {
    static size_t cached = 0;
    if (cached) return cached;
    long ps = sysconf(_SC_PAGESIZE);
    if (ps <= 0 || ps > (1L << 20)) {
        /* Implausible — fall back to the canonical 4 KiB. */
        cached = 4096;
    } else {
        cached = (size_t)ps;
    }
    return cached;
}

static size_t lotus_pool_initial_cells_for(size_t cell_stride) {
    if (cell_stride == 0) return LOTUS_POOL_INITIAL_CELLS;
    size_t page = lotus_host_page_size();
    if (page < cell_stride) return LOTUS_POOL_INITIAL_CELLS;
    size_t n = page / cell_stride;
    if (n < LOTUS_POOL_INITIAL_CELLS) n = LOTUS_POOL_INITIAL_CELLS;
    if (n > LOTUS_POOL_MAX_CHUNK_CELLS) n = LOTUS_POOL_MAX_CHUNK_CELLS;
    return n;
}

typedef struct lotus_pool_chunk {
    struct lotus_pool_chunk *next;
    size_t                   cells;
    /* cell data follows in the same allocation — first cell
     * starts at (char *)(chunk) + header_stride. */
} lotus_pool_chunk_t;

typedef struct lotus_pool {
    size_t              cell_stride;
    size_t              cell_align;
    size_t              header_stride;     /* aligned sizeof(chunk header) */
    size_t              next_chunk_cells;
    lotus_pool_chunk_t *chunks;
    void               *free_head;
} lotus_pool_t;

lotus_pool_t *lotus_pool_create(size_t cell_size, size_t cell_align) {
    if (cell_align == 0) cell_align = 8;
    size_t min_size = cell_size > sizeof(void *) ? cell_size : sizeof(void *);
    size_t stride   = lotus_align_up(min_size, cell_align);
    size_t hdr      = lotus_align_up(sizeof(lotus_pool_chunk_t), cell_align);
    lotus_pool_t *p = (lotus_pool_t *)malloc(sizeof(lotus_pool_t));
    if (!p) return NULL;
    p->cell_stride       = stride;
    p->cell_align        = cell_align;
    p->header_stride     = hdr;
    /* v1.x-17: initial chunk sized to host page size when that
     * fits more cells than the static-16 floor. */
    p->next_chunk_cells  = lotus_pool_initial_cells_for(stride);
    p->chunks            = NULL;
    p->free_head         = NULL;
    return p;
}

static int lotus_pool_grow(lotus_pool_t *p) {
    size_t n          = p->next_chunk_cells;
    size_t data_bytes = n * p->cell_stride;
    void  *raw        = malloc(p->header_stride + data_bytes);
    if (!raw) return 0;
    lotus_pool_chunk_t *c = (lotus_pool_chunk_t *)raw;
    c->next   = p->chunks;
    c->cells  = n;
    p->chunks = c;
    /* Thread the new cells onto the free-list in reverse so the
     * lowest-address cell ends up at the head — gives acquire-
     * order locality (first acquire after grow lands on the
     * lowest address, next acquire lands one stride above, etc.). */
    char *base = (char *)raw + p->header_stride;
    for (size_t i = n; i > 0; i--) {
        char *cell       = base + (i - 1) * p->cell_stride;
        *(void **)cell   = p->free_head;
        p->free_head     = cell;
    }
    if (p->next_chunk_cells < LOTUS_POOL_MAX_CHUNK_CELLS) {
        size_t doubled = p->next_chunk_cells * 2;
        p->next_chunk_cells = doubled > LOTUS_POOL_MAX_CHUNK_CELLS
                                  ? LOTUS_POOL_MAX_CHUNK_CELLS
                                  : doubled;
    }
    return 1;
}

void *lotus_pool_acquire(lotus_pool_t *p) {
    if (!p) return NULL;
    if (!p->free_head) {
        if (!lotus_pool_grow(p)) return NULL;
    }
    void *cell    = p->free_head;
    p->free_head  = *(void **)cell;
    /* Caller treats the cell as uninitialized — we don't memset.
     * Hale's let-binding rule says every binding is the type's
     * initial declaration; the caller writes fields before any
     * read can observe the stale free-list pointer that still
     * sits in the cell's first sizeof(void*) bytes. */
    return cell;
}

void lotus_pool_release(lotus_pool_t *p, void *cell) {
    if (!p || !cell) return;
    *(void **)cell = p->free_head;
    p->free_head   = cell;
}

void lotus_pool_destroy(lotus_pool_t *p) {
    if (!p) return;
    lotus_pool_chunk_t *c = p->chunks;
    while (c) {
        lotus_pool_chunk_t *next = c->next;
        free(c);
        c = next;
    }
    free(p);
}

/*
 * F.22 capacity slot — Heap of T (individually-freed cells with
 * locus-bounded lifetime).
 *
 * Each alloc is one malloc; the heap struct holds a doubly-linked
 * list of every live cell so free() is O(1) (unlink the cell)
 * and destroy() can free every still-live cell wholesale.
 *
 * The list lives in a per-cell header sitting just before the
 * returned pointer in the same allocation. Cell payload starts
 * at base + header_stride, where header_stride is the aligned-up
 * size of the header. On free(), the header is recovered by
 * subtracting header_stride from the user pointer.
 *
 * Alignment: malloc returns alignof(max_align_t) (typically 16)
 * regardless of request. Hale v1 types have alignment ≤ 8
 * (Int/Float = 8; user structs default to 8 or 16). For
 * cell_align > alignof(max_align_t) the substrate would need
 * posix_memalign; v1 doesn't generate such types so we don't
 * implement the fallback. If a cell_align larger than 16 ever
 * lands, the assertion path is to extend create() to record an
 * "oversized align" flag and route alloc through posix_memalign.
 *
 * Spec: spec/design-rationale.md §F.22 — "Heap of T — *I hold
 * growable state bounded by my own lifetime.*"
 */

typedef struct lotus_heap_cell {
    struct lotus_heap_cell *prev;
    struct lotus_heap_cell *next;
    /* cell payload follows at (char *)(cell) + header_stride. */
} lotus_heap_cell_t;

typedef struct lotus_heap {
    size_t              cell_size;
    size_t              cell_align;
    size_t              header_stride;
    lotus_heap_cell_t  *live_head;
} lotus_heap_t;

lotus_heap_t *lotus_heap_create(size_t cell_size, size_t cell_align) {
    if (cell_align == 0) cell_align = 8;
    size_t hdr = lotus_align_up(sizeof(lotus_heap_cell_t), cell_align);
    lotus_heap_t *h = (lotus_heap_t *)malloc(sizeof(lotus_heap_t));
    if (!h) return NULL;
    h->cell_size     = cell_size;
    h->cell_align    = cell_align;
    h->header_stride = hdr;
    h->live_head     = NULL;
    return h;
}

void *lotus_heap_alloc(lotus_heap_t *h) {
    if (!h) return NULL;
    void *raw = malloc(h->header_stride + h->cell_size);
    if (!raw) return NULL;
    lotus_heap_cell_t *cell = (lotus_heap_cell_t *)raw;
    cell->prev = NULL;
    cell->next = h->live_head;
    if (h->live_head) h->live_head->prev = cell;
    h->live_head = cell;
    return (char *)raw + h->header_stride;
}

void lotus_heap_free(lotus_heap_t *h, void *cell) {
    if (!h || !cell) return;
    lotus_heap_cell_t *hdr =
        (lotus_heap_cell_t *)((char *)cell - h->header_stride);
    if (hdr->prev) hdr->prev->next = hdr->next;
    else            h->live_head    = hdr->next;
    if (hdr->next) hdr->next->prev = hdr->prev;
    free(hdr);
}

void lotus_heap_destroy(lotus_heap_t *h) {
    if (!h) return;
    lotus_heap_cell_t *c = h->live_head;
    while (c) {
        lotus_heap_cell_t *next = c->next;
        free(c);
        c = next;
    }
    free(h);
}

/*
 * @form(vec) substrate (v1.x-FORM-1 PR4).
 *
 * A contiguous, growable buffer of elements of a single fixed
 * size. Inline in the locus's struct layout — codegen emits the
 * three-field struct `{ cap, len, buf }` for each `heap items of T`
 * slot under `@form(vec)`, and the functions below operate on
 * that struct generically by taking `elem_size` (= sizeof(T))
 * as an explicit parameter at each call site.
 *
 * The functions read/write the struct through a `void *` pointer
 * to the vec's start. All `lotus_vec_<T>_t` layouts share the
 * `{ size_t cap, size_t len, char *buf }` prefix — codegen
 * monomorphizes the typedef per T, but the runtime sees only the
 * common prefix. Element storage is contiguous in `buf`; the i-th
 * element lives at `buf + i * elem_size`.
 *
 * Growth policy: capacity starts at 0 (no allocation at locus
 * birth). The first `push` allocates a 4-element buffer. Each
 * overflow doubles cap and `realloc`s. Shrink is not implemented
 * in v1; `lotus_vec_destroy` releases the buffer at locus
 * dissolution.
 *
 * Fallible operations (`get`, `pop`) return `int` (1 = success,
 * 0 = error). Codegen in PR5/6 lifts that bool into the
 * `Ty::Fallible { success: T, payload: IndexError }` surface the
 * type system sees.
 */

typedef struct {
    size_t cap;
    size_t len;
    char *buf;
} lotus_vec_t;

/* Initial buffer size on first push, in elements. Chosen as a
 * small constant that avoids per-element malloc on tiny vecs
 * without wasting space for short-lived ones. */
#define LOTUS_VEC_INITIAL_CAP 4

void lotus_vec_init(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    v->cap = 0;
    v->len = 0;
    v->buf = NULL;
}

void lotus_vec_push(void *vec_ptr, size_t elem_size, const void *elem) {
    if (!vec_ptr || !elem) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len == v->cap) {
        size_t new_cap = v->cap == 0 ? LOTUS_VEC_INITIAL_CAP : v->cap * 2;
        char *new_buf = (char *)realloc(v->buf, new_cap * elem_size);
        if (!new_buf) {
            /* OOM. Per the design's failure surface, hardware
             * traps re-raise as closure violations; PR5/6 wires
             * that. For now, drop the push and signal via
             * unchanged v (best-effort; codegen integration will
             * add proper trap handling). */
            return;
        }
        v->buf = new_buf;
        v->cap = new_cap;
    }
    memcpy(v->buf + v->len * elem_size, elem, elem_size);
    v->len += 1;
}

int lotus_vec_get(void *vec_ptr, size_t elem_size, int64_t i, void *out) {
    if (!vec_ptr || !out) return 0;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (i < 0 || (size_t)i >= v->len) return 0;
    memcpy(out, v->buf + (size_t)i * elem_size, elem_size);
    return 1;
}

/* In-place mutation. Mirrors lotus_vec_get: bounds-checked at
 * [0, len). Returns 1 on success, 0 on out-of-bounds. Codegen
 * lifts that bool into `Ty::Fallible { success: (), payload:
 * IndexError }` at the call site. */
int lotus_vec_set(void *vec_ptr, size_t elem_size, int64_t i, const void *elem) {
    if (!vec_ptr || !elem) return 0;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (i < 0 || (size_t)i >= v->len) return 0;
    memcpy(v->buf + (size_t)i * elem_size, elem, elem_size);
    return 1;
}

int lotus_vec_pop(void *vec_ptr, size_t elem_size, void *out) {
    if (!vec_ptr || !out) return 0;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len == 0) return 0;
    v->len -= 1;
    memcpy(out, v->buf + v->len * elem_size, elem_size);
    return 1;
}

int64_t lotus_vec_len(void *vec_ptr) {
    if (!vec_ptr) return 0;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    return (int64_t)v->len;
}

int lotus_vec_is_empty(void *vec_ptr) {
    if (!vec_ptr) return 1;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    return v->len == 0 ? 1 : 0;
}

/* Typed comparators for the primitive `sort()` variants. qsort
 * is happy with these directly — no cookie / no trampoline. */
static int cmp_i64(const void *a, const void *b) {
    int64_t av = *(const int64_t *)a;
    int64_t bv = *(const int64_t *)b;
    return (av > bv) - (av < bv);
}
static int cmp_f64(const void *a, const void *b) {
    double av = *(const double *)a;
    double bv = *(const double *)b;
    if (av < bv) return -1;
    if (av > bv) return  1;
    return 0;
}
static int cmp_str(const void *a, const void *b) {
    const char *av = *(const char *const *)a;
    const char *bv = *(const char *const *)b;
    return strcmp(av, bv);
}

void lotus_vec_sort_int(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len < 2 || !v->buf) return;
    qsort(v->buf, v->len, sizeof(int64_t), cmp_i64);
}
void lotus_vec_sort_float(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len < 2 || !v->buf) return;
    qsort(v->buf, v->len, sizeof(double), cmp_f64);
}
void lotus_vec_sort_string(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len < 2 || !v->buf) return;
    qsort(v->buf, v->len, sizeof(const char *), cmp_str);
}

/* sort_by / sort_desc_by infrastructure. The trampoline pattern:
 * codegen emits a per-cell-type wrapper that loads (a, b) from
 * the buffer, calls the user's `fn(T, T) -> Bool` comparator,
 * and returns -1/0/+1 the way qsort expects. The cookie carries
 * (arena, user_cmp_fn, reverse_flag) — reverse_flag flips the
 * result so sort_desc_by reuses the same trampoline with a true
 * flag. */
typedef int (*lotus_vec_trampoline_t)(const void *a, const void *b, void *cookie);

void lotus_vec_sort_by(void *vec_ptr,
                       size_t elem_size,
                       lotus_vec_trampoline_t cmp,
                       void *cookie) {
    if (!vec_ptr || !cmp) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    if (v->len < 2 || !v->buf) return;
    /* qsort_r is GNU-extension; the arg order matches glibc's
     * `(base, nmemb, size, compar, arg)` form. */
    qsort_r(v->buf, v->len, elem_size,
            (int (*)(const void *, const void *, void *))cmp,
            cookie);
}

void lotus_vec_destroy(void *vec_ptr) {
    if (!vec_ptr) return;
    lotus_vec_t *v = (lotus_vec_t *)vec_ptr;
    free(v->buf);
    v->buf = NULL;
    v->cap = 0;
    v->len = 0;
}

/*
 * v1.x-FORM-4 — `@form(hashmap)` storage primitives.
 *
 * Intrusive open-addressing hash table with linear probing. The
 * value type S carries its own key as one of its fields
 * (`indexed_by <fieldname>`); codegen extracts the key by GEP'ing
 * the field offset before each call, so the C ABI takes key and
 * value as separate pointers and never has to know about the
 * struct's internal layout.
 *
 * Slot layout: each slot is `1 + key_size + value_size` bytes:
 *
 *   [occupied: 1 byte] [key: key_size bytes] [value: value_size bytes]
 *
 * `occupied = 0` means empty; we use backward-shift deletion
 * (no tombstones) so probes terminate as soon as an empty slot
 * is seen. Cap is always a power of two so the hash-to-index
 * fold is a single `& mask`. Initial cap = 8; doubles when load
 * factor exceeds 0.7.
 *
 * Key types at v1: 0 = Int (64-bit, Knuth multiplicative hash),
 * 1 = String (C-string pointer, FNV-1a over the bytes). The
 * key_type_tag is set at init and frozen for the hashmap's life.
 *
 * Fallible operations (`get`, `remove`) return `int` (1 =
 * success, 0 = not_found). Codegen in PR5/6 lifts that bool
 * into the `Ty::Fallible { success: S, payload: KeyError }`
 * surface the type system sees.
 */

#define LOTUS_HASHMAP_KEY_INT    0
#define LOTUS_HASHMAP_KEY_STRING 1

/* Initial slot count. Power of two so `& mask` folds the hash;
 * 8 covers small-population hashmaps (config tables, small
 * registries) without an early grow. */
#define LOTUS_HASHMAP_INITIAL_CAP 8

/* Load-factor threshold = LOAD_NUM / LOAD_DEN = 7/10. Grow
 * before insertion when `(len + 1) * LOAD_DEN > cap * LOAD_NUM`. */
#define LOTUS_HASHMAP_LOAD_NUM 7
#define LOTUS_HASHMAP_LOAD_DEN 10

/* F.32-1γ-v2 (session 3, 2026-05-26): lockfree's load-factor
 * threshold is tighter than the other disciplines (6/10 vs
 * 7/10) because tombstones accumulate between grows — keeping
 * the probe distance bounded under churn requires triggering
 * grow sooner. The check is `(live + tombstones) * LOAD_DEN
 * > cap * LF_LOAD_NUM`. */
#define LOTUS_HASHMAP_LF_LOAD_NUM 6

/* F.32-1β2 (2026-05-25) — cell-level CAS striped discipline:
 * sync_mode + occupancy state encoding + cache-line constant. */
#define LOTUS_HASHMAP_SYNC_NONE       0  /* plain @form(hashmap); single-pool */
#define LOTUS_HASHMAP_SYNC_SERIALIZED 1  /* α: per-map pthread_mutex_t */
#define LOTUS_HASHMAP_SYNC_STRIPED    2  /* β2: cell CAS + rwlock-on-grow */
#define LOTUS_HASHMAP_SYNC_LOCKFREE   3  /* γ-v1: fixed-cap, pure CAS, no rwlock, no remove
                                          * γ-v2 session 1: + tombstones + remove */

/* 3-state occupancy byte for striped cells. Plain / serialized
 * cells use just 0 (empty) / 1 (occupied) since serial access
 * doesn't need the CLAIMED intermediate state. */
#define LOTUS_CELL_EMPTY     0
#define LOTUS_CELL_CLAIMED   1  /* striped: writer holds slot, not yet published */
#define LOTUS_CELL_COMMITTED 2  /* striped: writer released; key + value valid */
/* F.32-1γ-v2 (session 1): tombstone marker for removed lockfree
 * entries. Probes continue past TOMBSTONE (an entry with this key
 * may live further along the probe chain, placed before the
 * tombstone was created); only EMPTY terminates the probe. EMPTY
 * is the probe boundary. Striped / serialized / none never produce
 * TOMBSTONE — they use backward-shift compaction in
 * remove_unlocked, so the TOMBSTONE path is dead code for those
 * disciplines but the shared probe helpers still handle the state
 * gracefully. */
#define LOTUS_CELL_TOMBSTONE 3

/* Cache line for the striped cell-stride padding (matches the
 * value used in experiments/f32-false-sharing/bench.c). The
 * C twin's measurement shows ~5x speedup ceiling from padding
 * on this hardware; striped cells round their stride up to
 * this multiple to land each cell on its own line. Tunable
 * via build-time #define for non-x86_64 targets. */
#ifndef LOTUS_CACHE_LINE
#  define LOTUS_CACHE_LINE 64
#endif

typedef struct {
    size_t cap;
    size_t len;
    size_t key_size;
    size_t value_size;
    int key_type_tag;
    char *slots;
    /* F.32-1α (2026-05-24): sync = serialized opt-in. When the
     * locus is declared `@form(hashmap, sync = serialized)`,
     * `lotus_hashmap_init_serialized` sets `has_sync = 1` and
     * heap-allocates `mu`. All hashmap entry points then take
     * the mutex around the existing single-threaded body. Plain
     * `@form(hashmap)` keeps `has_sync = 0` and `mu = NULL`;
     * the lock-check is a single load + branch — no atomic, no
     * contention.
     *
     * Why a pointer rather than inline storage: `pthread_mutex_t`
     * has platform-dependent size (40 B on Linux glibc x86_64,
     * 64 B on macOS, etc.). The inline-LLVM-struct codegen
     * needs a portable, fixed-width slot — a pointer is 8 B
     * everywhere. The one-time malloc at init is amortized
     * across the locus's lifetime. */
    /* F.32-1α/β2 (2026-05-24 / 2026-05-25): cross-pool sync
     * discipline. Renamed from `has_sync` (bool) to widen for
     * `striped` (β2). Values: LOTUS_HASHMAP_SYNC_{NONE,
     * SERIALIZED, STRIPED}. Read at every entry point to
     * dispatch the right locking strategy. */
    int sync_mode;
    /* SERIALIZED only: per-map mutex. Heap-allocated by
     * lotus_hashmap_init_serialized so the inline struct
     * layout stays platform-portable (pthread_mutex_t has
     * platform-dependent size; the pointer doesn't). */
    pthread_mutex_t *mu;
    /* STRIPED only: per-map RW-lock for grow exclusion. set /
     * get / has / key_at / value_at / len take rdlock so
     * multiple ops run concurrently; grow / remove take wrlock
     * so they're exclusive against everything. Heap-allocated
     * for the same portability reason as `mu`. NULL for
     * non-STRIPED disciplines. */
    pthread_rwlock_t *mu_grow;
    /* Cell stride in bytes — replaces the runtime-computed
     * `1 + key_size + value_size` for layout-aware paths. For
     * NONE / SERIALIZED, set to the packed value at init. For
     * STRIPED, set to `round_up(1 + key_size + value_size,
     * LOTUS_CACHE_LINE)` so each cell lands on its own cache
     * line. All slot indexing uses this stride; `entry_size()`
     * returns it. */
    size_t cell_stride;
    /* 2026-05-25: monotonic-iteration cursor. `key_at(i)` and
     * `value_at(i)` are each O(cap) per call — they scan the
     * slots array counting occupied entries until they hit the
     * i-th one. A loop calling them with i = 0, 1, ..., N-1 is
     * O(N * cap) ≈ O(N²) since cap grows with N. The cursor
     * collapses this to O(N) total for monotonic iteration:
     *
     *   - After `key_at(i)` / `value_at(i)` succeeds at slot S,
     *     set `cursor_i = i` and `cursor_slot = S`.
     *   - Next call with the same `i`: hit slot S directly, O(1).
     *   - Next call with `i == cursor_i + 1`: scan from
     *     `cursor_slot + 1`, O(slots-walked) amortized O(1)/call.
     *   - Random-access `i`: cursor is stale → fall back to
     *     full scan, behavior unchanged from before.
     *
     * Cursor is invalidated by any mutation that changes the
     * slot layout: `set` (may append + may grow), `remove`
     * (shifts entries to fill the gap), `grow` (whole table
     * rebuilt). The invalidation is a single store to
     * `cursor_i = -1`. Safe for the common bench / canonical
     * iteration shape:
     *
     *   let n = m.len();
     *   let mut i = 0;
     *   while i < n {
     *       let k = m.key_at(i) or raise;
     *       let e = m.entry_at(i) or raise;     // same i, O(1)
     *       i = i + 1;                          // next call i+1
     *   }
     *
     * Stricter invariant: between two cursor-using calls there
     * are no `set` / `remove` / mutating method invocations.
     * Held by every spec-compliant iteration loop.
     *
     * Bench impact (form_hashmap_walk_large @ 100k): 6.88 s →
     * single-digit ms. Pre-fix: Hale was 17,000× behind Go on
     * iteration. */
    int64_t cursor_i;
    size_t  cursor_slot;
    /* F.32-1γ-v2 (session 1): tombstone counter, lockfree only.
     * `m->len` continues to mean live entries; `tombstone_count`
     * tracks slots in TOMBSTONE state. NONE/SERIALIZED/STRIPED
     * use backward-shift remove (no tombstones) so this counter
     * stays 0 for them. Session 3 grow now uses
     * `live + tombstone_count` for the load-factor check, and
     * the migration rebuilds the table WITHOUT tombstones
     * (lazy compaction) — `tombstone_count` resets to 0 on
     * every grow. */
    size_t tombstone_count;
    /* F.32-1γ-v2 (session 3, 2026-05-26): lockfree grow state.
     *
     * Design choice (deviates from NBHM in the handoff doc): use
     * a single grower with a brief writers/readers stall instead
     * of cooperative migration via SENTINEL. Steady-state ops
     * stay fully lockfree (one atomic load on the fast path);
     * during the rare grow event (~ms for typical caps) all
     * ops yield-spin. Trade-off: simpler implementation; tail
     * latency on the op that triggered grow is bounded by the
     * migration time, not amortized across helpers.
     *
     * Protocol:
     *   - Fast path: ALL lockfree ops load `lf_grow_phase`. If 0
     *     (idle), increment `lf_writers_in_flight` and proceed.
     *     If non-0, sched_yield and retry.
     *   - Grow path: ONE writer CAS-claims grow_phase 0→1, then
     *     spin-waits for `lf_writers_in_flight` to drain. Once
     *     drained, it owns the table exclusively: allocates NEW,
     *     copies COMMITTED entries (tombstones drop), swaps
     *     m->slots/cap, frees OLD eagerly (session 4 — the
     *     drain-wait guarantees no in-flight op references OLD),
     *     stores grow_phase = 0.
     *
     * Fields:
     *   lf_grow_phase: 0 = idle (hot path lockfree), 1 = grow
     *                  in progress (all lockfree ops stall).
     *   lf_writers_in_flight: count of in-flight lockfree ops
     *                  that hold a stale m->slots / m->cap
     *                  snapshot. Grower waits for this to reach
     *                  zero before swapping.
     *
     * NONE/SERIALIZED/STRIPED disciplines never touch these
     * fields — they use their own grow paths
     * (`lotus_hashmap_grow` via the lock pair). */
    int lf_grow_phase;
    int64_t lf_writers_in_flight;
} lotus_hashmap_t;

static size_t lotus_hashmap_entry_size(const lotus_hashmap_t *m) {
    /* F.32-1β2 (2026-05-25): returns the cached cell stride, which
     * is set at init time. For plain / serialized maps this is
     * the packed `1 + key + value`; for striped maps this is
     * rounded up to LOTUS_CACHE_LINE so adjacent cells land on
     * separate cache lines. */
    return m->cell_stride;
}

static size_t lotus_hashmap_hash(const lotus_hashmap_t *m, const void *key) {
    if (m->key_type_tag == LOTUS_HASHMAP_KEY_INT) {
        /* 64-bit Knuth multiplicative — distributes Int keys
         * including dense sequences (handles common workloads
         * like consecutive IDs without all colliding on slot 0). */
        uint64_t k = *(const uint64_t *)key;
        return (size_t)(k * 0x9E3779B97F4A7C15ULL);
    }
    /* String — the key is a C-string pointer; hash the bytes. */
    const char *s = *(const char *const *)key;
    if (!s) return 0;
    uint64_t h = 0xcbf29ce484222325ULL;
    for (const char *p = s; *p; ++p) {
        h ^= (uint8_t)*p;
        h *= 0x100000001b3ULL;
    }
    return (size_t)h;
}

static int lotus_hashmap_key_eq(const lotus_hashmap_t *m,
                                 const void *a,
                                 const void *b) {
    if (m->key_type_tag == LOTUS_HASHMAP_KEY_INT) {
        return *(const int64_t *)a == *(const int64_t *)b;
    }
    const char *sa = *(const char *const *)a;
    const char *sb = *(const char *const *)b;
    if (sa == sb) return 1;
    if (!sa || !sb) return 0;
    return strcmp(sa, sb) == 0;
}

/* Find the slot index for `key`. Returns either:
 *   - an existing entry with the matching key (slot occupied,
 *     key equal), or
 *   - the first empty slot encountered along the probe chain.
 * Caller inspects the occupied byte to disambiguate. */
static size_t lotus_hashmap_find_slot(const lotus_hashmap_t *m,
                                       const void *key) {
    size_t es = lotus_hashmap_entry_size(m);
    size_t mask = m->cap - 1;
    size_t i = lotus_hashmap_hash(m, key) & mask;
    for (;;) {
        char *slot = m->slots + i * es;
        if (!slot[0]) return i;
        if (lotus_hashmap_key_eq(m, slot + 1, key)) return i;
        i = (i + 1) & mask;
    }
}

void lotus_hashmap_init(void *map_ptr,
                         size_t key_size,
                         size_t value_size,
                         int key_type_tag) {
    if (!map_ptr) return;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    m->cap = LOTUS_HASHMAP_INITIAL_CAP;
    m->len = 0;
    m->key_size = key_size;
    m->value_size = value_size;
    m->key_type_tag = key_type_tag;
    m->cell_stride = 1 + key_size + value_size;
    m->slots = (char *)calloc(m->cap, m->cell_stride);
    /* F.32-1α: default discipline = none. Methods inspect
     * `sync_mode` and skip locking. */
    m->sync_mode = LOTUS_HASHMAP_SYNC_NONE;
    m->mu = NULL;
    m->mu_grow = NULL;
    /* 2026-05-25: cursor starts invalid. First key_at / value_at
     * pays the full scan; subsequent monotonic accesses ride
     * the cursor. */
    m->cursor_i = -1;
    m->cursor_slot = 0;
    /* F.32-1γ-v2 (session 1): tombstone counter, only mutated on
     * the lockfree remove / set-on-tombstone paths but
     * initialized here so it has a defined value for any
     * discipline. */
    m->tombstone_count = 0;
    /* F.32-1γ-v2 (session 3): lockfree grow state. Idle by
     * default; only the lockfree discipline mutates these. */
    m->lf_grow_phase = 0;
    m->lf_writers_in_flight = 0;
}

/* F.32-1α (2026-05-24): sync = serialized variant. Same init
 * payload as `lotus_hashmap_init` PLUS allocates + initializes
 * a `pthread_mutex_t`. The locus codegen emits a call to this
 * variant (in place of `lotus_hashmap_init`) when the form
 * annotation carries `sync = serialized`. */
void lotus_hashmap_init_serialized(void *map_ptr,
                                    size_t key_size,
                                    size_t value_size,
                                    int key_type_tag) {
    if (!map_ptr) return;
    lotus_hashmap_init(map_ptr, key_size, value_size, key_type_tag);
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    m->mu = (pthread_mutex_t *)malloc(sizeof(pthread_mutex_t));
    pthread_mutex_init(m->mu, NULL);
    m->sync_mode = LOTUS_HASHMAP_SYNC_SERIALIZED;
}

/* F.32-1γ-v1 (2026-05-25): sync = lockfree variant.
 * Cache-padded cells like β2. Pure CAS on slot[0] for the
 * 4-state occupancy machine (EMPTY → CLAIMED → COMMITTED →
 * TOMBSTONE after γ-v2 session 1); no rwlock, no mutex on
 * the steady-state hot path.
 *
 * Per-op cost on the fast path: 1 atomic load of
 * `lf_grow_phase` (branch-not-taken) + load_acquire + CAS +
 * memcpy + release_store. No kernel-mediated synchronization
 * primitives in steady state. Suited for:
 *   - Workloads where the entry count is bounded + known at
 *     deploy time (Prometheus registries with a fixed metric
 *     list, route tables, config caches).
 *   - High-core-count writes (8+ cores) where the rwlock
 *     overhead in β2 dominates.
 *
 * F.32-1γ-v2 session 3 (2026-05-26): `cap = N` is now an
 * initial-size hint rather than a hard ceiling. When the
 * load factor (live + tombstones / cap) exceeds the
 * threshold, the table grows by doubling. Grow briefly
 * stalls in-flight lockfree ops (~ms typical) but steady
 * state remains lockfree. Tombstones are dropped during
 * migration (lazy compaction). */
void lotus_hashmap_init_lockfree(void *map_ptr,
                                  size_t key_size,
                                  size_t value_size,
                                  int key_type_tag,
                                  size_t fixed_cap) {
    if (!map_ptr) return;
    /* Round cap up to next power of 2 for the `& mask` probe;
     * the user's `cap = N` is treated as a floor, not an
     * exact count. */
    size_t cap = 1;
    while (cap < fixed_cap) cap <<= 1;
    if (cap < LOTUS_HASHMAP_INITIAL_CAP) cap = LOTUS_HASHMAP_INITIAL_CAP;
    lotus_hashmap_init(map_ptr, key_size, value_size, key_type_tag);
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    /* Pad cell stride + reallocate slots to the user's cap. */
    size_t packed = 1 + key_size + value_size;
    size_t padded = (packed + (LOTUS_CACHE_LINE - 1))
                  & ~((size_t)(LOTUS_CACHE_LINE - 1));
    free(m->slots);
    m->cell_stride = padded;
    m->cap = cap;
    m->slots = (char *)calloc(cap, m->cell_stride);
    m->sync_mode = LOTUS_HASHMAP_SYNC_LOCKFREE;
}

/* F.32-1β2 (2026-05-25): sync = striped variant. Cells are
 * cache-line-padded so disjoint-key writers on different
 * cores don't false-share. Per-map RW-lock guards the grow
 * path: set / get / etc take rdlock (concurrent), grow /
 * remove take wrlock (exclusive). Slot-level claim via the
 * 3-state occupancy byte (EMPTY → CLAIMED → COMMITTED) lets
 * concurrent writers race on different slots without
 * blocking on a global mutex.
 *
 * Reallocates the slots array because cell_stride differs
 * from the packed default. Init order: regular init first
 * (sets packed stride), then we replace cell_stride with the
 * padded value and recalloc. */
void lotus_hashmap_init_striped(void *map_ptr,
                                 size_t key_size,
                                 size_t value_size,
                                 int key_type_tag) {
    if (!map_ptr) return;
    lotus_hashmap_init(map_ptr, key_size, value_size, key_type_tag);
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    /* Pad cell stride up to the next cache-line multiple. */
    size_t packed = 1 + key_size + value_size;
    size_t padded = (packed + (LOTUS_CACHE_LINE - 1))
                  & ~((size_t)(LOTUS_CACHE_LINE - 1));
    if (padded > m->cell_stride) {
        free(m->slots);
        m->cell_stride = padded;
        m->slots = (char *)calloc(m->cap, m->cell_stride);
    }
    m->mu_grow = (pthread_rwlock_t *)malloc(sizeof(pthread_rwlock_t));
    pthread_rwlock_init(m->mu_grow, NULL);
    m->sync_mode = LOTUS_HASHMAP_SYNC_STRIPED;
}

/* F.32-1α helpers — single load + branch each. Inlined in the
 * compiled C; the branch is well-predicted on either side
 * (always-taken for serialized maps, never-taken for plain).
 *
 * F.32-1β2 (2026-05-25): dispatch on sync_mode for the lock
 * pair. NONE = no-op. SERIALIZED = full pthread_mutex around
 * the body (writers serialize, reads block writers). STRIPED
 * = rdlock around the body (concurrent reads / writes; the
 * cell-level CAS in `set_striped` provides slot-level
 * mutual exclusion). The striped wrlock paths (grow / remove)
 * use their own dedicated locking — `lotus_hashmap_wrlock`. */
static inline void lotus_hashmap_lock(lotus_hashmap_t *m) {
    switch (m->sync_mode) {
        case LOTUS_HASHMAP_SYNC_SERIALIZED:
            pthread_mutex_lock(m->mu);
            break;
        case LOTUS_HASHMAP_SYNC_STRIPED:
            pthread_rwlock_rdlock(m->mu_grow);
            break;
        default: /* NONE */
            break;
    }
}
static inline void lotus_hashmap_unlock(lotus_hashmap_t *m) {
    switch (m->sync_mode) {
        case LOTUS_HASHMAP_SYNC_SERIALIZED:
            pthread_mutex_unlock(m->mu);
            break;
        case LOTUS_HASHMAP_SYNC_STRIPED:
            pthread_rwlock_unlock(m->mu_grow);
            break;
        default:
            break;
    }
}
/* F.32-1β2 wrlock pair — exclusive against everything. Used by
 * grow (called from set when load factor exceeded) and remove
 * (Robin-Hood shifts can't run concurrent with reads). For
 * NONE / SERIALIZED, falls back to the same lock pair as
 * lotus_hashmap_lock (mutex is already exclusive). */
static inline void lotus_hashmap_wrlock(lotus_hashmap_t *m) {
    switch (m->sync_mode) {
        case LOTUS_HASHMAP_SYNC_SERIALIZED:
            pthread_mutex_lock(m->mu);
            break;
        case LOTUS_HASHMAP_SYNC_STRIPED:
            pthread_rwlock_wrlock(m->mu_grow);
            break;
        default:
            break;
    }
}
static inline void lotus_hashmap_wrunlock(lotus_hashmap_t *m) {
    switch (m->sync_mode) {
        case LOTUS_HASHMAP_SYNC_SERIALIZED:
            pthread_mutex_unlock(m->mu);
            break;
        case LOTUS_HASHMAP_SYNC_STRIPED:
            pthread_rwlock_unlock(m->mu_grow);
            break;
        default:
            break;
    }
}

/* F.32-1α (2026-05-24): each public entry point comes in two
 * pieces: an `_unlocked` body that does the actual work, and
 * the public wrapper that optionally takes the per-map mutex
 * around the body. `grow` and the cross-method recursion paths
 * use only `_unlocked` so the lock isn't re-acquired. */
static void lotus_hashmap_set_unlocked(lotus_hashmap_t *m,
                                        const void *key,
                                        const void *value);

static void lotus_hashmap_grow(lotus_hashmap_t *m) {
    size_t old_cap = m->cap;
    char *old_slots = m->slots;
    size_t es = lotus_hashmap_entry_size(m);
    size_t new_cap = old_cap * 2;
    m->cap = new_cap;
    m->slots = (char *)calloc(new_cap, es);
    m->len = 0;
    /* Reinsert every live entry into the new table. Route through
     * `_unlocked` — the outer wrapper already holds the lock if
     * sync_mode != NONE (or the wrlock for striped), and the grow
     * path itself is single-threaded by invariant. F.32-1β2:
     * for striped, occupancy byte CLAIMED-or-COMMITTED both
     * count as live entries to migrate; we copy the slot bytes
     * regardless of whether the previous occupant was mid-write
     * (a CLAIMED-during-grow scenario would mean someone is
     * holding rdlock while we hold wrlock, which the rwlock
     * primitive itself rules out). */
    for (size_t i = 0; i < old_cap; i++) {
        char *slot = old_slots + i * es;
        if (slot[0]) {
            lotus_hashmap_set_unlocked(m, slot + 1, slot + 1 + m->key_size);
        }
    }
    free(old_slots);
}

static void lotus_hashmap_set_unlocked(lotus_hashmap_t *m,
                                        const void *key,
                                        const void *value) {
    /* Grow before insertion when adding one more entry would
     * cross the load-factor threshold. The check uses unsigned
     * arithmetic so it stays correct as len/cap grow. */
    if ((m->len + 1) * LOTUS_HASHMAP_LOAD_DEN >
        m->cap * LOTUS_HASHMAP_LOAD_NUM) {
        lotus_hashmap_grow(m);
    }
    size_t es = lotus_hashmap_entry_size(m);
    size_t i = lotus_hashmap_find_slot(m, key);
    char *slot = m->slots + i * es;
    int was_empty = !slot[0];
    /* 2026-05-25: write LOTUS_CELL_COMMITTED (=2) rather than
     * a bare `1` here. v1 NONE/SERIALIZED only sees non-zero;
     * either value works for them. v2 STRIPED reserves state=1
     * (LOTUS_CELL_CLAIMED) for the transient "writer mid-publish"
     * marker, so the grow path (which also goes through this
     * function during rebuild) must NOT leave entries in
     * CLAIMED state — otherwise a striped probe scanning the
     * rebuilt table would spin forever on every cell. */
    slot[0] = LOTUS_CELL_COMMITTED;
    memcpy(slot + 1, key, m->key_size);
    memcpy(slot + 1 + m->key_size, value, m->value_size);
    if (was_empty) m->len++;
    /* 2026-05-25: iteration cursor is now stale (slot occupancy
     * pattern changed). Next key_at / value_at falls back to a
     * full scan; subsequent monotonic calls re-establish the
     * cursor. */
    m->cursor_i = -1;
}

/* F.32-1β2 (2026-05-25): striped set. Cell-level CAS for slot
 * claim + 3-state occupancy for safe concurrent reads. Grow
 * exclusion via rwlock_wrlock.
 *
 * Protocol per slot:
 *   EMPTY (0)     — free; writers CAS to CLAIMED (1)
 *   CLAIMED (1)   — writer owns slot; key+value being written
 *   COMMITTED (2) — key+value published; readers can dereference
 *
 * The acquire-release pair on slot[0] ensures readers that
 * observe COMMITTED also see the key+value writes.
 *
 * Spin on CLAIMED: rare — only when two writers probe to the
 * same slot index in tight succession, and the loser briefly
 * waits for the winner's ~100 ns memcpy. No yield needed in
 * practice; the loop is bounded by the writer's release. */
/* F.32-1β2-v2 (2026-05-25): true cell-level CAS striped set.
 * Multiple writers run in parallel — each contends only on
 * the cells their probe sequences visit, not on a global
 * mutex.
 *
 * Protocol per slot, with state transitions on slot[0]:
 *
 *   EMPTY (0) ──CAS──> CLAIMED (1) ── memcpy + release-store ──> COMMITTED (2)
 *                                                                     │
 *   COMMITTED (2) ──CAS (update)──> CLAIMED (1) ──memcpy──> COMMITTED (2)
 *                                                                     │
 *   COMMITTED (2) ──remove (under wrlock)──> EMPTY (0)
 *
 * The acquire-release pair on slot[0] ensures readers that
 * observe COMMITTED also see the key/value writes.
 *
 * Probe safety: if the table becomes too full to find an
 * empty slot before we wrap (probes >= m->cap), force a
 * grow. Triggered when concurrent writers race past the
 * load-factor check and collectively fill the remaining
 * empty slots before our probe completes. */
static void lotus_hashmap_set_striped(lotus_hashmap_t *m,
                                       const void *key,
                                       const void *value) {
    pthread_rwlock_rdlock(m->mu_grow);

    /* Grow check + re-check pattern. If we need to grow, we
     * have to drop rdlock to take wrlock. Another thread may
     * grow first; we re-check under wrlock to avoid double-
     * grow, then drop wrlock + retake rdlock for the probe. */
    size_t cur_len = __atomic_load_n(&m->len, __ATOMIC_RELAXED);
    if ((cur_len + 1) * LOTUS_HASHMAP_LOAD_DEN
        > m->cap * LOTUS_HASHMAP_LOAD_NUM)
    {
        pthread_rwlock_unlock(m->mu_grow);
        pthread_rwlock_wrlock(m->mu_grow);
        cur_len = __atomic_load_n(&m->len, __ATOMIC_RELAXED);
        if ((cur_len + 1) * LOTUS_HASHMAP_LOAD_DEN
            > m->cap * LOTUS_HASHMAP_LOAD_NUM)
        {
            lotus_hashmap_grow(m);
        }
        pthread_rwlock_unlock(m->mu_grow);
        pthread_rwlock_rdlock(m->mu_grow);
    }

    /* Probe with wrap-around detection. mask + i recomputed
     * after each potential grow (cap may have doubled). */
probe_restart:;
    size_t es = lotus_hashmap_entry_size(m);
    size_t mask = m->cap - 1;
    size_t i = lotus_hashmap_hash(m, key) & mask;
    size_t probes = 0;

    for (;;) {
        if (probes >= m->cap) {
            /* Wrapped without finding a slot. Concurrent
             * writers raced past us; force a grow and
             * restart the probe with the new cap. */
            pthread_rwlock_unlock(m->mu_grow);
            pthread_rwlock_wrlock(m->mu_grow);
            cur_len = __atomic_load_n(&m->len, __ATOMIC_RELAXED);
            if ((cur_len + 1) * LOTUS_HASHMAP_LOAD_DEN
                > m->cap * LOTUS_HASHMAP_LOAD_NUM)
            {
                lotus_hashmap_grow(m);
            }
            pthread_rwlock_unlock(m->mu_grow);
            pthread_rwlock_rdlock(m->mu_grow);
            goto probe_restart;
        }

        char *slot = m->slots + i * es;
        uint8_t state =
            __atomic_load_n((uint8_t *)&slot[0], __ATOMIC_ACQUIRE);

        if (state == LOTUS_CELL_EMPTY) {
            uint8_t expected = LOTUS_CELL_EMPTY;
            if (__atomic_compare_exchange_n(
                    (uint8_t *)&slot[0],
                    &expected, LOTUS_CELL_CLAIMED,
                    0, /* strong CAS */
                    __ATOMIC_ACQUIRE,
                    __ATOMIC_RELAXED))
            {
                /* Won the slot. Publish key + value, then
                 * release-store COMMITTED. */
                memcpy(slot + 1, key, m->key_size);
                memcpy(slot + 1 + m->key_size, value, m->value_size);
                __atomic_store_n((uint8_t *)&slot[0],
                                 LOTUS_CELL_COMMITTED,
                                 __ATOMIC_RELEASE);
                __atomic_fetch_add(&m->len, 1, __ATOMIC_RELAXED);
                m->cursor_i = -1;
                break;
            }
            /* CAS lost — re-read state and re-decide. */
            continue;
        }

        if (state == LOTUS_CELL_CLAIMED) {
            /* Writer mid-publish on this slot. Spin briefly. */
            continue;
        }

        /* COMMITTED — compare keys. */
        if (lotus_hashmap_key_eq(m, slot + 1, key)) {
            /* Update path. CAS COMMITTED → CLAIMED, rewrite
             * value, release-store COMMITTED. */
            uint8_t expected = LOTUS_CELL_COMMITTED;
            if (__atomic_compare_exchange_n(
                    (uint8_t *)&slot[0],
                    &expected, LOTUS_CELL_CLAIMED,
                    0,
                    __ATOMIC_ACQUIRE,
                    __ATOMIC_RELAXED))
            {
                memcpy(slot + 1 + m->key_size, value, m->value_size);
                __atomic_store_n((uint8_t *)&slot[0],
                                 LOTUS_CELL_COMMITTED,
                                 __ATOMIC_RELEASE);
                /* len unchanged for update */
                break;
            }
            /* Lost update race — retry same slot. */
            continue;
        }

        /* Different key — probe next. */
        i = (i + 1) & mask;
        probes++;
    }

    pthread_rwlock_unlock(m->mu_grow);
}

/* F.32-1γ-v2 session 3 (2026-05-26): enter/exit protocol for
 * lockfree ops. Fast path: 1 atomic load. Slow path (when grow
 * is in progress): yield-spin until grow completes, then take
 * the writer-in-flight counter.
 *
 * The pair must bracket every access to m->slots / m->cap on
 * the lockfree path — set, get, has, remove, iteration. */
static inline void lotus_hashmap_lf_enter(lotus_hashmap_t *m) {
    for (;;) {
        int phase = __atomic_load_n(&m->lf_grow_phase, __ATOMIC_ACQUIRE);
        if (phase != 0) {
            sched_yield();
            continue;
        }
        __atomic_fetch_add(&m->lf_writers_in_flight, 1, __ATOMIC_ACQUIRE);
        /* Re-check: a grower could have CAS'd phase 0→1 between
         * our load and our fetch_add. If so, back out and retry
         * — the grower's `wait for writers_in_flight == 0`
         * spin would otherwise deadlock with our presence. */
        phase = __atomic_load_n(&m->lf_grow_phase, __ATOMIC_ACQUIRE);
        if (phase != 0) {
            __atomic_fetch_sub(&m->lf_writers_in_flight, 1, __ATOMIC_RELEASE);
            sched_yield();
            continue;
        }
        return;
    }
}

static inline void lotus_hashmap_lf_exit(lotus_hashmap_t *m) {
    __atomic_fetch_sub(&m->lf_writers_in_flight, 1, __ATOMIC_RELEASE);
}

/* Single-threaded migration. Caller (lotus_hashmap_grow_lockfree)
 * holds lf_grow_phase == 1 and has spin-waited for
 * lf_writers_in_flight to drain. No concurrent ops touch
 * m->slots / m->cap. Tombstones are silently dropped (the lazy
 * compaction the handoff doc anticipated). */
static void lotus_hashmap_lf_migrate(lotus_hashmap_t *m,
                                      char *old_slots,
                                      size_t old_cap,
                                      char *new_slots,
                                      size_t new_cap) {
    size_t es = m->cell_stride;
    size_t mask = new_cap - 1;
    size_t live = 0;
    for (size_t s = 0; s < old_cap; s++) {
        char *slot = old_slots + s * es;
        /* Migration sees only COMMITTED + EMPTY + TOMBSTONE in
         * steady state; CLAIMED is transient (writer
         * mid-publish) and our drain-wait guaranteed no
         * writers are in flight. */
        if (slot[0] != LOTUS_CELL_COMMITTED) continue;
        const char *key = slot + 1;
        const char *value = slot + 1 + m->key_size;
        /* Insert into NEW. NEW is fresh (calloc) so all slots
         * are EMPTY; probing terminates at the first EMPTY. */
        size_t i = lotus_hashmap_hash(m, key) & mask;
        for (;;) {
            char *nslot = new_slots + i * es;
            if (nslot[0] == LOTUS_CELL_EMPTY) {
                memcpy(nslot + 1, key, m->key_size);
                memcpy(nslot + 1 + m->key_size, value, m->value_size);
                nslot[0] = LOTUS_CELL_COMMITTED;
                live++;
                break;
            }
            i = (i + 1) & mask;
        }
    }
    /* Update accounting under the single-threaded migration
     * lock. m->len is rebuilt precisely; tombstones drop to
     * zero. */
    __atomic_store_n(&m->len, live, __ATOMIC_RELAXED);
    __atomic_store_n(&m->tombstone_count, 0, __ATOMIC_RELAXED);
}

/* F.32-1γ-v2 session 3: grow path. ONE writer wins the CAS to
 * lf_grow_phase 0→1; spin-waits for in-flight ops to drain;
 * allocates the doubled NEW table; copies live entries (drops
 * tombstones); installs NEW as m->slots/cap; stashes OLD on
 * lf_old_slots (one-generation hold) and frees the previous
 * stash if any. Session 4 replaces the stash-then-free pattern
 * with QSBR epoch reclamation. */
static void lotus_hashmap_grow_lockfree(lotus_hashmap_t *m) {
    int expected = 0;
    if (!__atomic_compare_exchange_n(
            &m->lf_grow_phase, &expected, 1,
            0, __ATOMIC_ACQ_REL, __ATOMIC_RELAXED))
    {
        /* Another writer already started the grow. Don't
         * wait here — our caller will retry the op via the
         * lf_enter spin on the next iteration. */
        return;
    }
    /* Spin until every in-flight lockfree op has exited.
     * The 0→1 store on lf_grow_phase makes new ops back off
     * via the lf_enter re-check; existing ops drain quickly
     * (they hold the counter across a few CAS / memcpy). */
    while (__atomic_load_n(&m->lf_writers_in_flight, __ATOMIC_ACQUIRE) > 0) {
        sched_yield();
    }
    size_t old_cap = m->cap;
    char *old_slots = m->slots;
    size_t new_cap = old_cap * 2;
    if (new_cap < LOTUS_HASHMAP_INITIAL_CAP) {
        new_cap = LOTUS_HASHMAP_INITIAL_CAP;
    }
    size_t es = m->cell_stride;
    char *new_slots = (char *)calloc(new_cap, es);
    if (!new_slots) {
        /* Allocation failure: bail out without growing. Set
         * paths will continue to insert into OLD until the
         * next grow attempt. The probe will eventually
         * saturate, but the caller's program has bigger
         * problems if calloc returned NULL. */
        __atomic_store_n(&m->lf_grow_phase, 0, __ATOMIC_RELEASE);
        return;
    }
    lotus_hashmap_lf_migrate(m, old_slots, old_cap, new_slots, new_cap);
    /* Atomic-store the new slots/cap so any future reader
     * loading via __atomic_load_n sees a consistent pair.
     * (No in-flight reader exists right now; the stores are
     * just for memory-ordering correctness on later loads —
     * lf_enter's grow_phase re-check guarantees subsequent
     * ops see grow_phase=0 only AFTER they'd see the new
     * slots/cap via the release-store sequencing.) */
    __atomic_store_n(&m->slots, new_slots, __ATOMIC_RELEASE);
    __atomic_store_n(&m->cap, new_cap, __ATOMIC_RELEASE);
    /* Iterator cursor is invalidated by the rebuild — its
     * cached slot index no longer maps onto m->slots. */
    m->cursor_i = -1;
    m->cursor_slot = 0;
    /* F.32-1γ-v2 session 4 (2026-05-26): free OLD eagerly.
     *
     * Initially session 3 stashed OLD on `lf_old_slots` and
     * freed it at the NEXT grow (one-generation hold). The
     * stash was defensive: if any in-flight op could still
     * hold a stale pointer to OLD, freeing immediately would
     * be use-after-free. But the writers_in_flight drain above
     * already guarantees no such op exists — every lockfree
     * entry point brackets itself with lf_enter/lf_exit, and
     * the drain spun until the counter reached zero. After
     * the drain, OLD has zero live references.
     *
     * Removing the stash means a sustained-write workload
     * that triggers multiple grows holds only the CURRENT
     * table (no extra cap/2 bytes carried per generation).
     * RSS stays bounded by the current table size + the
     * brief peak during migration (OLD + NEW both alive for
     * the duration of `lf_migrate`). This is the result the
     * handoff doc's QSBR design was reaching for — our
     * simpler single-grower protocol gives it directly,
     * without epoch tracking through the cooperative
     * scheduler. */
    free(old_slots);
    __atomic_store_n(&m->lf_grow_phase, 0, __ATOMIC_RELEASE);
}

/* F.32-1γ-v1 (2026-05-25): lockfree set. Same 3-state CAS
 * machine as β2's set_striped, minus the rwlock and the
 * grow path. Probe bounded by m->cap.
 *
 * F.32-1γ-v2 (session 1): TOMBSTONE-aware. Probes advance
 * past TOMBSTONE slots without reuse — same-key updates land
 * in the new EMPTY slot ahead of the tombstone, fresh inserts
 * find the next EMPTY past the tombstone chain. Reuse-on-
 * tombstone is intentionally deferred to session 3, where the
 * grow path's natural compaction (NEW table built without
 * tombstones) removes the need.
 *
 * F.32-1γ-v2 (session 3): bracketed by lf_enter / lf_exit so
 * grow can safely swap m->slots / m->cap. Triggers grow after
 * a successful insert when load factor exceeds
 * LF_LOAD_NUM/LOAD_DEN. */
static void lotus_hashmap_set_lockfree(lotus_hashmap_t *m,
                                        const void *key,
                                        const void *value) {
    int did_grow_check = 0;
set_retry:
    lotus_hashmap_lf_enter(m);
    size_t es = lotus_hashmap_entry_size(m);
    size_t mask = m->cap - 1;
    size_t i = lotus_hashmap_hash(m, key) & mask;
    size_t probes = 0;
    for (;;) {
        if (probes >= m->cap) {
            /* Probe exhausted under the current cap. Exit the
             * critical region, trigger grow (the load-factor
             * check below will succeed), and retry the set
             * against the doubled table. */
            lotus_hashmap_lf_exit(m);
            lotus_hashmap_grow_lockfree(m);
            did_grow_check = 1;
            goto set_retry;
        }
        char *slot = m->slots + i * es;
        uint8_t state =
            __atomic_load_n((uint8_t *)&slot[0], __ATOMIC_ACQUIRE);
        if (state == LOTUS_CELL_EMPTY) {
            uint8_t expected = LOTUS_CELL_EMPTY;
            if (__atomic_compare_exchange_n(
                    (uint8_t *)&slot[0],
                    &expected, LOTUS_CELL_CLAIMED,
                    0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED))
            {
                memcpy(slot + 1, key, m->key_size);
                memcpy(slot + 1 + m->key_size, value, m->value_size);
                __atomic_store_n((uint8_t *)&slot[0],
                                 LOTUS_CELL_COMMITTED,
                                 __ATOMIC_RELEASE);
                __atomic_fetch_add(&m->len, 1, __ATOMIC_RELAXED);
                goto set_done_insert;
            }
            continue;  /* CAS lost; re-read */
        }
        if (state == LOTUS_CELL_CLAIMED) continue;  /* spin */
        /* F.32-1γ-v2 (session 1): TOMBSTONE — slot's previous
         * key was removed. Advance probe; the residual key
         * bytes are NOT valid for comparison (they may match
         * coincidentally, which would silently corrupt the
         * update path via a TOMBSTONE → CLAIMED CAS that
         * shouldn't fire). */
        if (state == LOTUS_CELL_TOMBSTONE) {
            i = (i + 1) & mask;
            probes++;
            continue;
        }
        /* COMMITTED — compare keys. */
        if (lotus_hashmap_key_eq(m, slot + 1, key)) {
            /* Update path. Same CAS COMMITTED → CLAIMED →
             * write → release-store COMMITTED. */
            uint8_t expected = LOTUS_CELL_COMMITTED;
            if (__atomic_compare_exchange_n(
                    (uint8_t *)&slot[0],
                    &expected, LOTUS_CELL_CLAIMED,
                    0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED))
            {
                memcpy(slot + 1 + m->key_size, value, m->value_size);
                __atomic_store_n((uint8_t *)&slot[0],
                                 LOTUS_CELL_COMMITTED,
                                 __ATOMIC_RELEASE);
                /* Update path: no len change, no grow trigger. */
                lotus_hashmap_lf_exit(m);
                return;
            }
            continue;  /* lost update race; retry */
        }
        i = (i + 1) & mask;
        probes++;
    }
set_done_insert:
    /* Successful insert. Exit the critical region, then check
     * the load factor and trigger grow if exceeded. Grow runs
     * its own enter-spin so racing growers serialize on the
     * grow_phase CAS.
     *
     * Cap snapshot is taken AFTER exit so we don't hold the
     * writer-in-flight token while spinning on grow. The race
     * is benign: a concurrent writer may grow first; we'll
     * find lf_grow_phase != 0 on the CAS and skip our grow. */
    {
        size_t cap_now = m->cap;
        size_t live = __atomic_load_n(&m->len, __ATOMIC_RELAXED);
        size_t tomb = __atomic_load_n(&m->tombstone_count, __ATOMIC_RELAXED);
        lotus_hashmap_lf_exit(m);
        if (!did_grow_check
            && (live + tomb) * LOTUS_HASHMAP_LOAD_DEN
                > cap_now * LOTUS_HASHMAP_LF_LOAD_NUM)
        {
            lotus_hashmap_grow_lockfree(m);
        }
    }
}

void lotus_hashmap_set(void *map_ptr,
                        const void *key,
                        const void *value) {
    if (!map_ptr || !key || !value) return;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_LOCKFREE) {
        lotus_hashmap_set_lockfree(m, key, value);
        return;
    }
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_STRIPED) {
        lotus_hashmap_set_striped(m, key, value);
        return;
    }
    lotus_hashmap_lock(m);
    lotus_hashmap_set_unlocked(m, key, value);
    lotus_hashmap_unlock(m);
}

/* F.32-1β2-v2 (2026-05-25): striped probe-and-read. Reads slot[0]
 * atomically; spins past CLAIMED slots (writer mid-publish)
 * until COMMITTED or EMPTY. Probe boundary is EMPTY; mismatched-
 * key COMMITTED entries advance to the next slot. Returns slot
 * index when the key's COMMITTED entry is found, or m->cap when
 * not found.
 *
 * The CLAIMED-spin is bounded by the writer's CAS-publish window
 * (~tens of ns: memcpy key+value + release-store). No kernel
 * wait. */
static size_t lotus_hashmap_find_slot_striped(lotus_hashmap_t *m,
                                               const void *key) {
    size_t es = lotus_hashmap_entry_size(m);
    size_t mask = m->cap - 1;
    size_t i = lotus_hashmap_hash(m, key) & mask;
    /* Probe-bound: a fully tombstoned-or-committed table would
     * never expose an EMPTY terminator. Walk at most `cap`
     * positions before declaring "not present". For non-lockfree
     * disciplines this is unreachable (tombstones don't exist
     * there, and the load-factor invariant guarantees at least
     * one EMPTY); for lockfree v2 it's a safety net against
     * adversarial probe chains. */
    size_t probes = 0;
    for (;;) {
        if (probes >= m->cap) return m->cap;  /* not present */
        char *slot = m->slots + i * es;
        uint8_t state;
        for (;;) {
            state =
                __atomic_load_n((uint8_t *)&slot[0], __ATOMIC_ACQUIRE);
            if (state != LOTUS_CELL_CLAIMED) break;
            /* Spin: writer mid-publish. */
        }
        if (state == LOTUS_CELL_EMPTY) {
            return m->cap;  /* probe boundary: not found */
        }
        /* F.32-1γ-v2 (session 1): TOMBSTONE — slot's key was
         * removed but the key bytes may still match
         * coincidentally. Advance the probe; the key (if still
         * present) lives further down the chain since session-1
         * doesn't reuse tombstoned slots on insert. */
        if (state == LOTUS_CELL_TOMBSTONE) {
            i = (i + 1) & mask;
            probes++;
            continue;
        }
        /* COMMITTED — key bytes valid (release-store paired
         * with our acquire-load above). */
        if (lotus_hashmap_key_eq(m, slot + 1, key)) {
            return i;
        }
        i = (i + 1) & mask;
        probes++;
    }
}

int lotus_hashmap_get(void *map_ptr, const void *key, void *out_value) {
    if (!map_ptr || !key || !out_value) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_LOCKFREE) {
        /* Lockfree get: no rwlock; bracket with lf_enter/exit so
         * grow can swap m->slots / m->cap underneath us safely. */
        lotus_hashmap_lf_enter(m);
        int r = 0;
        if (__atomic_load_n(&m->len, __ATOMIC_RELAXED) > 0) {
            size_t i = lotus_hashmap_find_slot_striped(m, key);
            if (i < m->cap) {
                char *slot = m->slots + i * lotus_hashmap_entry_size(m);
                memcpy(out_value, slot + 1 + m->key_size, m->value_size);
                r = 1;
            }
        }
        lotus_hashmap_lf_exit(m);
        return r;
    }
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_STRIPED) {
        pthread_rwlock_rdlock(m->mu_grow);
        int r = 0;
        if (__atomic_load_n(&m->len, __ATOMIC_RELAXED) > 0) {
            size_t i = lotus_hashmap_find_slot_striped(m, key);
            if (i < m->cap) {
                char *slot = m->slots + i * lotus_hashmap_entry_size(m);
                memcpy(out_value, slot + 1 + m->key_size, m->value_size);
                r = 1;
            }
        }
        pthread_rwlock_unlock(m->mu_grow);
        return r;
    }
    lotus_hashmap_lock(m);
    int r;
    if (m->len == 0) {
        r = 0;
    } else {
        size_t es = lotus_hashmap_entry_size(m);
        size_t i = lotus_hashmap_find_slot(m, key);
        char *slot = m->slots + i * es;
        if (!slot[0]) {
            r = 0;
        } else {
            memcpy(out_value, slot + 1 + m->key_size, m->value_size);
            r = 1;
        }
    }
    lotus_hashmap_unlock(m);
    return r;
}

int lotus_hashmap_has(void *map_ptr, const void *key) {
    if (!map_ptr || !key) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_LOCKFREE) {
        lotus_hashmap_lf_enter(m);
        int r = 0;
        if (__atomic_load_n(&m->len, __ATOMIC_RELAXED) > 0) {
            size_t i = lotus_hashmap_find_slot_striped(m, key);
            r = (i < m->cap) ? 1 : 0;
        }
        lotus_hashmap_lf_exit(m);
        return r;
    }
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_STRIPED) {
        pthread_rwlock_rdlock(m->mu_grow);
        int r = 0;
        if (__atomic_load_n(&m->len, __ATOMIC_RELAXED) > 0) {
            size_t i = lotus_hashmap_find_slot_striped(m, key);
            r = (i < m->cap) ? 1 : 0;
        }
        pthread_rwlock_unlock(m->mu_grow);
        return r;
    }
    lotus_hashmap_lock(m);
    int r;
    if (m->len == 0) {
        r = 0;
    } else {
        size_t es = lotus_hashmap_entry_size(m);
        size_t i = lotus_hashmap_find_slot(m, key);
        r = m->slots[i * es] ? 1 : 0;
    }
    lotus_hashmap_unlock(m);
    return r;
}

/* Backward-shift deletion. After clearing the target slot,
 * walk forward and shift any entry whose natural position is
 * "before" the freed slot in the probe sequence — that's what
 * keeps `find_slot` correct without tombstones.
 *
 * F.32-1β2 (2026-05-25): factored body for striped remove (the
 * shifts are not safe to run concurrent with reads; striped
 * remove takes wrlock for exclusivity). For NONE/SERIALIZED
 * the caller is already inside `lotus_hashmap_lock`. */
static int lotus_hashmap_remove_unlocked(lotus_hashmap_t *m,
                                          const void *key) {
    if (m->len == 0) return 0;
    size_t es = lotus_hashmap_entry_size(m);
    size_t mask = m->cap - 1;
    size_t i = lotus_hashmap_find_slot(m, key);
    if (!m->slots[i * es]) return 0;
    m->slots[i * es] = 0;
    m->len--;
    size_t j = (i + 1) & mask;
    while (m->slots[j * es]) {
        size_t natural =
            lotus_hashmap_hash(m, m->slots + j * es + 1) & mask;
        size_t dist_to_j = (j - natural) & mask;
        size_t dist_to_i = (i - natural) & mask;
        if (dist_to_i < dist_to_j) {
            memmove(m->slots + i * es, m->slots + j * es, es);
            m->slots[j * es] = 0;
            i = j;
        }
        j = (j + 1) & mask;
    }
    /* 2026-05-25: shifts above invalidate the iteration cursor. */
    m->cursor_i = -1;
    return 1;
}

/* F.32-1γ-v2 (session 1): lockfree remove. CAS the slot's
 * occupancy byte COMMITTED → TOMBSTONE; concurrent readers
 * that observed COMMITTED before the CAS continue to read the
 * (now-stale) value, which is the documented lockfree
 * consistency model ("key was present at the moment we read").
 *
 * Returns 1 on a successful remove, 0 on miss (key not present,
 * or concurrent double-remove won the race). A racing concurrent
 * update may push the slot to CLAIMED briefly; find_slot_striped
 * already spins past CLAIMED so the retry loop converges. */
static int lotus_hashmap_remove_lockfree(lotus_hashmap_t *m,
                                          const void *key) {
    lotus_hashmap_lf_enter(m);
    int r = 0;
    if (__atomic_load_n(&m->len, __ATOMIC_RELAXED) > 0) {
        size_t es = lotus_hashmap_entry_size(m);
        for (;;) {
            size_t i = lotus_hashmap_find_slot_striped(m, key);
            if (i >= m->cap) { r = 0; break; }  /* not present */
            char *slot = m->slots + i * es;
            uint8_t expected = LOTUS_CELL_COMMITTED;
            if (__atomic_compare_exchange_n(
                    (uint8_t *)&slot[0],
                    &expected, LOTUS_CELL_TOMBSTONE,
                    0, /* strong CAS */
                    __ATOMIC_ACQ_REL,
                    __ATOMIC_RELAXED))
            {
                __atomic_fetch_add(&m->tombstone_count, 1, __ATOMIC_RELAXED);
                __atomic_fetch_sub(&m->len, 1, __ATOMIC_RELAXED);
                r = 1;
                break;
            }
            /* CAS failed. The slot's state moved out from
             * under us:
             *   - CLAIMED: a concurrent update is in flight;
             *     loop and find_slot's CLAIMED-spin will
             *     re-stabilize.
             *   - TOMBSTONE: a concurrent remove already won;
             *     the next find_slot iteration won't see the
             *     key → r = 0 via "not present" branch.
             * The loop terminates because each iteration
             * either converges (CAS succeeds) or progresses
             * the state machine toward a terminal state. */
        }
    }
    lotus_hashmap_lf_exit(m);
    return r;
}

int lotus_hashmap_remove(void *map_ptr, const void *key) {
    if (!map_ptr || !key) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_LOCKFREE) {
        /* F.32-1γ-v2 (session 1): tombstone-based remove.
         * No rwlock — pure CAS on the slot's occupancy byte.
         * Grow + compaction still pending (session 3). */
        return lotus_hashmap_remove_lockfree(m, key);
    }
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_STRIPED) {
        /* Striped: take wrlock so shifts can't race with rdlock'd
         * readers. find_slot uses non-atomic occupancy reads in
         * the unlocked body — safe because wrlock is exclusive. */
        pthread_rwlock_wrlock(m->mu_grow);
        int r = lotus_hashmap_remove_unlocked(m, key);
        pthread_rwlock_unlock(m->mu_grow);
        return r;
    }
    lotus_hashmap_lock(m);
    int r = lotus_hashmap_remove_unlocked(m, key);
    lotus_hashmap_unlock(m);
    return r;
}

int64_t lotus_hashmap_len(void *map_ptr) {
    if (!map_ptr) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_STRIPED
        || m->sync_mode == LOTUS_HASHMAP_SYNC_LOCKFREE) {
        /* Atomic load is sufficient; no lock needed — len
         * updates are atomic increments and a snapshot is
         * inherently approximate under concurrent writers. */
        return (int64_t)__atomic_load_n(&m->len, __ATOMIC_RELAXED);
    }
    lotus_hashmap_lock(m);
    int64_t n = (int64_t)m->len;
    lotus_hashmap_unlock(m);
    return n;
}

int lotus_hashmap_is_empty(void *map_ptr) {
    if (!map_ptr) return 1;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_STRIPED
        || m->sync_mode == LOTUS_HASHMAP_SYNC_LOCKFREE) {
        return __atomic_load_n(&m->len, __ATOMIC_RELAXED) == 0 ? 1 : 0;
    }
    lotus_hashmap_lock(m);
    int r = m->len == 0 ? 1 : 0;
    lotus_hashmap_unlock(m);
    return r;
}

/* Hash-table-order iteration. Walk the slots array counting
 * occupied entries; on the i-th occupied slot copy out the key
 * (or value/entry) and return 1. Returns 0 if i is out of range
 * (i < 0 || i >= len).
 *
 * Order is hash-table order (insertion-affected but stable for
 * a given table state). For "populate then iterate" patterns
 * the snapshot order is reproducible; mixing iteration with
 * mutation will see shifting order after a rehash. Per-call
 * cost is O(cap), so a full sweep is O(cap²) — fine at small/
 * medium scale, watch out at 100k+ entries.
 */
/* 2026-05-16: word-tokenize a C-string into a @form(vec) of
 * String. A "word" is a maximal run of bytes for which
 * is_word_char (alpha + digit + underscore + apostrophe) is
 * true; whitespace and punctuation are delimiters. Each token is
 * lower-cased (canonical agent intent for wordfreq-style work)
 * and arena-allocated as a NUL-terminated C string; the pointer
 * is pushed into the target vec via lotus_vec_push.
 *
 * Caller is responsible for passing an empty (or otherwise
 * reusable) target vec; the primitive does NOT clear it.
 */
static int lotus_text_is_word_byte(unsigned char c) {
    return (c >= 'a' && c <= 'z')
        || (c >= 'A' && c <= 'Z')
        || (c >= '0' && c <= '9')
        || c == '_'
        || c == '\'';
}

void lotus_text_tokenize_words_into(
    void *target_vec,
    const char *src,
    void *arena_ptr,
    int lowercase
) {
    if (!target_vec || !src) return;
    lotus_arena_t *arena = (lotus_arena_t *)arena_ptr;
    size_t i = 0;
    while (src[i]) {
        /* Skip non-word bytes. */
        while (src[i] && !lotus_text_is_word_byte((unsigned char)src[i])) {
            i++;
        }
        if (!src[i]) break;
        size_t start = i;
        while (src[i] && lotus_text_is_word_byte((unsigned char)src[i])) {
            i++;
        }
        size_t tok_len = i - start;
        /* Arena-allocate tok_len + 1 bytes for NUL termination. */
        char *tok = (char *)lotus_arena_alloc(arena, tok_len + 1, 1);
        if (!tok) return;
        memcpy(tok, src + start, tok_len);
        if (lowercase) {
            for (size_t j = 0; j < tok_len; j++) {
                if (tok[j] >= 'A' && tok[j] <= 'Z') tok[j] += 32;
            }
        }
        tok[tok_len] = '\0';
        /* Push the pointer (sizeof(char*) = sizeof(void*) = 8 on
         * 64-bit). lotus_vec_push memcpys `es` bytes from the
         * source; we point at &tok which is a stack temporary
         * whose address is fine across the call. */
        char *tok_ptr_for_push = tok;
        lotus_vec_push(target_vec, sizeof(char *), &tok_ptr_for_push);
    }
}

/* 2026-05-25: resolve the slot index for the i-th occupied
 * entry, using the iteration cursor when monotonic access is
 * detected. Returns the slot index (< m->cap) or m->cap when
 * not found / i out of range. Updates m->cursor_{i,slot} on
 * success so subsequent monotonic calls accelerate.
 *
 * Cursor states handled:
 *   - `cursor_i == i`            → return cached slot directly.
 *   - `cursor_i + 1 == i`        → start scan from cursor_slot + 1.
 *   - any other (incl. invalid)  → full scan from slot 0.
 *
 * Bounds check (i < len) is the caller's responsibility — both
 * key_at and value_at guard before invoking this. */
static size_t lotus_hashmap_resolve_index_slot(lotus_hashmap_t *m,
                                                int64_t i) {
    size_t es = lotus_hashmap_entry_size(m);
    if (m->cursor_i == i) {
        /* Same i twice in a row (canonical case: key_at(j) then
         * entry_at(j)). O(1) hit. */
        return m->cursor_slot;
    }
    size_t start_slot;
    size_t seen;
    if (m->cursor_i >= 0 && m->cursor_i + 1 == i) {
        /* Monotonic next step — pick up where we left off. */
        start_slot = m->cursor_slot + 1;
        seen = (size_t)(m->cursor_i + 1);
    } else {
        /* Random access or stale cursor — full scan from 0. */
        start_slot = 0;
        seen = 0;
    }
    for (size_t s = start_slot; s < m->cap; s++) {
        char *slot = m->slots + s * es;
        if (!slot[0]) continue;
        if (seen == (size_t)i) {
            m->cursor_i = i;
            m->cursor_slot = s;
            return s;
        }
        seen++;
    }
    return m->cap;  /* not found (i out of range despite < len check) */
}

/* F.32-1β2 (2026-05-25): striped iterator slot resolver. Mirror
 * of lotus_hashmap_resolve_index_slot, but reads occupancy
 * atomically and spins past CLAIMED slots (writer mid-publish).
 * Spin is bounded by the writer's CAS-publish window (tens of
 * ns) — no kernel wait. Cursor updates may race under rdlock;
 * the only consequence is a cold-path scan on the next call. */
static size_t lotus_hashmap_resolve_index_slot_striped(lotus_hashmap_t *m,
                                                        int64_t i) {
    size_t es = lotus_hashmap_entry_size(m);
    if (m->cursor_i == i) {
        return m->cursor_slot;
    }
    size_t start_slot;
    size_t seen;
    if (m->cursor_i >= 0 && m->cursor_i + 1 == i) {
        start_slot = m->cursor_slot + 1;
        seen = (size_t)(m->cursor_i + 1);
    } else {
        start_slot = 0;
        seen = 0;
    }
    for (size_t s = start_slot; s < m->cap; s++) {
        char *slot = m->slots + s * es;
        uint8_t state;
        for (;;) {
            state =
                __atomic_load_n((uint8_t *)&slot[0], __ATOMIC_ACQUIRE);
            if (state != LOTUS_CELL_CLAIMED) break;
            /* Spin: writer mid-publish on this slot. */
        }
        if (state == LOTUS_CELL_EMPTY) continue;
        /* F.32-1γ-v2 (session 1): tombstones don't count toward
         * iteration order — they're removed entries. m->len
         * tracks live entries only, so the i-th live entry skips
         * past TOMBSTONE slots the same way it skips EMPTY. */
        if (state == LOTUS_CELL_TOMBSTONE) continue;
        if (seen == (size_t)i) {
            m->cursor_i = i;
            m->cursor_slot = s;
            return s;
        }
        seen++;
    }
    return m->cap;
}

int lotus_hashmap_key_at(void *map_ptr, int64_t i, void *out_key) {
    if (!map_ptr || !out_key || i < 0) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_LOCKFREE) {
        lotus_hashmap_lf_enter(m);
        int r = 0;
        size_t cur_len = __atomic_load_n(&m->len, __ATOMIC_RELAXED);
        if ((size_t)i < cur_len) {
            size_t s = lotus_hashmap_resolve_index_slot_striped(m, i);
            if (s < m->cap) {
                size_t es = lotus_hashmap_entry_size(m);
                char *slot = m->slots + s * es;
                memcpy(out_key, slot + 1, m->key_size);
                r = 1;
            }
        }
        lotus_hashmap_lf_exit(m);
        return r;
    }
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_STRIPED) {
        pthread_rwlock_rdlock(m->mu_grow);
        int r = 0;
        size_t cur_len = __atomic_load_n(&m->len, __ATOMIC_RELAXED);
        if ((size_t)i < cur_len) {
            size_t s = lotus_hashmap_resolve_index_slot_striped(m, i);
            if (s < m->cap) {
                size_t es = lotus_hashmap_entry_size(m);
                char *slot = m->slots + s * es;
                memcpy(out_key, slot + 1, m->key_size);
                r = 1;
            }
        }
        pthread_rwlock_unlock(m->mu_grow);
        return r;
    }
    lotus_hashmap_lock(m);
    int r = 0;
    if ((size_t)i < m->len) {
        size_t s = lotus_hashmap_resolve_index_slot(m, i);
        if (s < m->cap) {
            size_t es = lotus_hashmap_entry_size(m);
            char *slot = m->slots + s * es;
            memcpy(out_key, slot + 1, m->key_size);
            r = 1;
        }
    }
    lotus_hashmap_unlock(m);
    return r;
}

int lotus_hashmap_value_at(void *map_ptr, int64_t i, void *out_value) {
    if (!map_ptr || !out_value || i < 0) return 0;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_LOCKFREE) {
        lotus_hashmap_lf_enter(m);
        int r = 0;
        size_t cur_len = __atomic_load_n(&m->len, __ATOMIC_RELAXED);
        if ((size_t)i < cur_len) {
            size_t s = lotus_hashmap_resolve_index_slot_striped(m, i);
            if (s < m->cap) {
                size_t es = lotus_hashmap_entry_size(m);
                char *slot = m->slots + s * es;
                memcpy(out_value, slot + 1 + m->key_size, m->value_size);
                r = 1;
            }
        }
        lotus_hashmap_lf_exit(m);
        return r;
    }
    if (m->sync_mode == LOTUS_HASHMAP_SYNC_STRIPED) {
        pthread_rwlock_rdlock(m->mu_grow);
        int r = 0;
        size_t cur_len = __atomic_load_n(&m->len, __ATOMIC_RELAXED);
        if ((size_t)i < cur_len) {
            size_t s = lotus_hashmap_resolve_index_slot_striped(m, i);
            if (s < m->cap) {
                size_t es = lotus_hashmap_entry_size(m);
                char *slot = m->slots + s * es;
                memcpy(out_value, slot + 1 + m->key_size, m->value_size);
                r = 1;
            }
        }
        pthread_rwlock_unlock(m->mu_grow);
        return r;
    }
    lotus_hashmap_lock(m);
    int r = 0;
    if ((size_t)i < m->len) {
        size_t s = lotus_hashmap_resolve_index_slot(m, i);
        if (s < m->cap) {
            size_t es = lotus_hashmap_entry_size(m);
            char *slot = m->slots + s * es;
            memcpy(out_value, slot + 1 + m->key_size, m->value_size);
            r = 1;
        }
    }
    lotus_hashmap_unlock(m);
    return r;
}

void lotus_hashmap_destroy(void *map_ptr) {
    if (!map_ptr) return;
    lotus_hashmap_t *m = (lotus_hashmap_t *)map_ptr;
    /* F.32-1α / β2 (2026-05-24 / 2026-05-25): release the
     * sync-discipline resources before freeing the storage.
     * Safe to call on NONE-mode maps (the switch falls through). */
    switch (m->sync_mode) {
        case LOTUS_HASHMAP_SYNC_SERIALIZED:
            pthread_mutex_destroy(m->mu);
            free(m->mu);
            m->mu = NULL;
            break;
        case LOTUS_HASHMAP_SYNC_STRIPED:
            pthread_rwlock_destroy(m->mu_grow);
            free(m->mu_grow);
            m->mu_grow = NULL;
            break;
        default:
            break;
    }
    m->sync_mode = LOTUS_HASHMAP_SYNC_NONE;
    free(m->slots);
    m->slots = NULL;
    m->cap = 0;
    m->len = 0;
}

/*
 * @form(ring_buffer) — fixed-capacity FIFO with push-back / pop-front.
 *
 * Pre-allocated at locus birth (single malloc of `cap × elem_size`
 * bytes); never grows. `push` returns 0 when the buffer is full
 * (caller decides drop vs. backpressure). `pop` returns 0 when
 * empty. Head/tail indices wrap modulo cap; the "ring" lives in
 * a flat contiguous buffer, no per-element link overhead.
 *
 * Layout matches the inline LLVM struct codegen emits:
 *
 *     struct lotus_ring_buffer {
 *         size_t cap;        // fixed at init; never changes
 *         size_t head;       // index of oldest element (next pop)
 *         size_t len;        // current element count (0..cap)
 *         size_t elem_size;  // bytes per element
 *         char  *buf;        // cap * elem_size bytes
 *     }
 *
 * The 5-field shape mirrors @form(vec)'s 3-field
 * { cap, len, buf } and @form(hashmap)'s 6-field
 * { cap, len, key_size, value_size, key_type_tag, slots } — same
 * "inline header + heap-malloc'd backing buffer" pattern, same
 * codegen-emits-inline-struct discipline. Fixed cap means no
 * doubling realloc; the entire buffer lives until the locus
 * dissolves.
 */

typedef struct lotus_ring_buffer {
    size_t cap;
    size_t head;
    size_t len;
    size_t elem_size;
    char  *buf;
} lotus_ring_buffer_t;

void lotus_ring_buffer_init(void *rb_ptr, size_t cap, size_t elem_size) {
    if (!rb_ptr) return;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    rb->cap = cap;
    rb->head = 0;
    rb->len = 0;
    rb->elem_size = elem_size;
    /* malloc rather than calloc — push always writes before any
     * read sees the slot. */
    rb->buf = (char *)malloc(cap * elem_size);
}

/* Returns 1 on success, 0 when full. v1 contract: "push returns
 * false when full"; the spec preview names the synthesized method
 * `push(x: T) -> Bool` (infallible — full is a Bool result, not
 * a fallible error). */
int lotus_ring_buffer_push(void *rb_ptr, const void *src) {
    if (!rb_ptr || !src) return 0;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    if (rb->len == rb->cap) return 0;
    size_t tail = (rb->head + rb->len) % rb->cap;
    memcpy(rb->buf + tail * rb->elem_size, src, rb->elem_size);
    rb->len++;
    return 1;
}

/* Returns 1 on success (and writes `elem_size` bytes into `out`),
 * 0 when empty. The synthesized `pop()` codegen converts the
 * Bool into the fallible(EmptyError) shape. */
int lotus_ring_buffer_pop(void *rb_ptr, void *out) {
    if (!rb_ptr || !out) return 0;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    if (rb->len == 0) return 0;
    memcpy(out, rb->buf + rb->head * rb->elem_size, rb->elem_size);
    rb->head = (rb->head + 1) % rb->cap;
    rb->len--;
    return 1;
}

size_t lotus_ring_buffer_len(void *rb_ptr) {
    if (!rb_ptr) return 0;
    return ((lotus_ring_buffer_t *)rb_ptr)->len;
}

int lotus_ring_buffer_is_full(void *rb_ptr) {
    if (!rb_ptr) return 0;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    return rb->len == rb->cap ? 1 : 0;
}

void lotus_ring_buffer_destroy(void *rb_ptr) {
    if (!rb_ptr) return;
    lotus_ring_buffer_t *rb = (lotus_ring_buffer_t *)rb_ptr;
    free(rb->buf);
    rb->buf = NULL;
    rb->cap = 0;
    rb->head = 0;
    rb->len = 0;
}

/*
 * Cooperative scheduler — bus dispatch queue (m26 + m28b stage 1).
 *
 * Per The Design / lotus, every bus dispatch is a substrate
 * cell. The cooperative scheduler enqueues these cells at
 * publish time and pops them one at a time at drain time.
 * Each pop runs the handler to completion (handler-atomic;
 * cooperative yields BETWEEN cells, not within). Handlers may
 * publish more events, which enqueue more cells; drain
 * continues until the queue is empty.
 *
 * m28b stage 1 changed cell shape: cells now carry an INLINE
 * payload buffer instead of a pointer to subscriber-arena
 * memory. This is the prerequisite for cross-thread bus: the
 * publisher can be on a different thread than the subscriber,
 * so the payload can't live in either arena (each arena is
 * single-threaded territory). The boundary IS the queue —
 * inline payload makes the queue the single point of cross-
 * thread synchronization. Drain copies inline → subscriber's
 * arena before invoking the handler, so the per-spec/memory.md
 * "every locus boundary copies the payload" rule still holds:
 * the subscriber gets its own arena-resident copy that outlives
 * the publisher.
 *
 * Cost vs m26: every cell does TWO memcpy's (publisher → cell
 * inline + cell inline → subscriber arena) instead of one
 * (publisher → subscriber arena). For the small typed messages
 * lotus carries this is negligible; cross-thread correctness
 * is worth more than one memcpy.
 *
 * Mutex protects the cell array so pinned threads can enqueue
 * concurrently with the cooperative drain (m28b stage 2). v0
 * uses a single mutex around enqueue + each pop. Drain releases
 * the lock around handler invocation so handlers can re-enqueue
 * without self-deadlock (and so cooperative handlers don't
 * block pinned producers for their entire run-time).
 *
 * Two-tier payload storage: small payloads stay inline (zero
 * malloc on the hot path); large payloads spill to a per-cell
 * `malloc`. LOTUS_PAYLOAD_INLINE sets the inline threshold —
 * payloads at or below this size use `payload_inline`,
 * payloads above it route through `payload_heap`. The drain
 * paths free the heap buffer after the handler returns.
 *
 * Pre-2026-05-27 this was a hard `LOTUS_PAYLOAD_MAX` cap and
 * over-cap payloads were dropped silently on enqueue —
 * surfaced by an L2 market-data feed that wanted to ship
 * 3.3 KB book snapshots over the bus. The two-tier shape
 * keeps the inline fast path's cost (one memcpy, no malloc)
 * while letting large payloads through. The UDP reader
 * thread's recv buffer is sized off `lotus_bus_udp_bufsize`
 * (env-configurable, default 64 KB) to admit jumbo-frame
 * datagrams end-to-end.
 */

#define LOTUS_PAYLOAD_INLINE 512

/* Maximum wire-format payload size the substrate handles per
 * message. Reader threads and dispatch sites allocate
 * buffers of this size; payloads larger than this truncate at
 * deserialize. Chosen at the UDP datagram max (65507 rounded
 * up) — bigger payloads would have to fragment, which the bus
 * doesn't do today (an app-layer framing protocol is the
 * right shape for >64 KB). Stack buffers of this size are
 * fine on default 8 MB pthread stacks even at 4-8 levels of
 * recursive bus dispatch; cooperative-pool workers and
 * reader threads inherit the default.
 *
 * Decoupled from LOTUS_PAYLOAD_INLINE so the in-cell hot path
 * keeps its tight inline size (zero malloc, one memcpy) while
 * the wire-side keeps headroom for jumbo / spilled payloads. */
#define LOTUS_PAYLOAD_MAX 65536

typedef struct lotus_bus_cell {
    void  *handler;                       /* void (*)(void *self, void *payload) */
    void  *self_ptr;                      /* subscriber's locus ptr */
    size_t payload_size;                  /* bytes used (inline or heap) */
    /* When NULL: payload lives in `payload_inline` (fits in
     * LOTUS_PAYLOAD_INLINE bytes). When non-NULL: payload lives
     * in a per-cell malloc'd buffer of `payload_size` bytes;
     * drain paths free it after handler returns. */
    void  *payload_heap;
    char   payload_inline[LOTUS_PAYLOAD_INLINE];
} lotus_bus_cell_t;

typedef struct lotus_bus_queue {
    lotus_bus_cell_t *cells;
    size_t            head;     /* next slot to pop */
    size_t            tail;     /* next slot to fill */
    size_t            cap;
    pthread_mutex_t   lock;
} lotus_bus_queue_t;

#define LOTUS_BUS_QUEUE_INITIAL_CAP 64

/* Single-thread fast path. Set to non-zero before any thread
 * beyond main can touch the bus queue. Set by:
 *   - `lotus_bus_mark_pinned` — codegen emits a call at every
 *     pinned-locus instantiation (sync, before pthread_create,
 *     so the new thread can never observe the flag unset on
 *     its publish path).
 *   - `lotus_coop_pool_start_all` — cooperative pool workers
 *     are their own threads too; main + ≥1 worker = multi-
 *     threaded bus access. (Originally this case was missed,
 *     producing a TSAN race in `lotus_bus_queue_drain` whenever
 *     two cooperative pools both drained the shared queue.
 *     2026-05-26 fix.)
 *
 * When zero, every enqueue and pop happens on a single thread
 * so the queue mutex is dead overhead — ~20-40ns/event on
 * uncontended lock+unlock pair. The flag is monotonic 0→1;
 * once set, contention is possible and we lock normally for
 * the rest of the program.
 *
 * The flag is atomic — readers on the bus hot path use
 * `__atomic_load_n` with acquire ordering; writers
 * (mark_pinned / pool_start_all) use release ordering. */
static int g_bus_has_pinned = 0;

void lotus_bus_mark_pinned(void) {
    __atomic_store_n(&g_bus_has_pinned, 1, __ATOMIC_RELEASE);
    /* Going multithreaded for the subregion-freelist lock too: a
     * pinned locus runs its lifecycle on its own thread, so it can
     * concurrently create/destroy subregions of a shared parent
     * arena. (perf opt, 2026-05-29 — codegen already emits this
     * call at pinned spawns, so the subregion latch comes free.) */
    lotus_mark_multithreaded();
}

lotus_bus_queue_t *lotus_bus_queue_create(void) {
    lotus_bus_queue_t *q =
        (lotus_bus_queue_t *)malloc(sizeof(lotus_bus_queue_t));
    if (!q) return NULL;
    q->cap   = LOTUS_BUS_QUEUE_INITIAL_CAP;
    q->cells = (lotus_bus_cell_t *)
        malloc(q->cap * sizeof(lotus_bus_cell_t));
    if (!q->cells) {
        free(q);
        return NULL;
    }
    q->head = 0;
    q->tail = 0;
    pthread_mutex_init(&q->lock, NULL);
    return q;
}

/* Enqueue (handler, self, payload_src + payload_size). The
 * publisher's payload is memcpy'd into the cell's inline
 * buffer; the cell does NOT carry a pointer back to publisher
 * memory. After enqueue returns, the publisher is free to
 * dissolve / reuse / overwrite the payload source — the queue
 * holds the canonical copy until drain re-copies it into the
 * subscriber's arena.
 *
 * Holds the queue's mutex for the duration so concurrent pinned
 * publishers don't corrupt each other's writes. */
void lotus_bus_queue_enqueue(lotus_bus_queue_t *q,
                             void *handler,
                             void *self_ptr,
                             const void *payload_src,
                             size_t payload_size) {
    if (!q) return;
    /* Two-tier payload storage. Small payloads land in the
     * cell's inline buffer (no malloc on the hot path); large
     * ones spill to a per-cell malloc that the drain path
     * frees after the handler returns. */
    void *heap_buf = NULL;
    if (payload_size > LOTUS_PAYLOAD_INLINE) {
        heap_buf = malloc(payload_size);
        if (!heap_buf) return;        /* drop on OOM */
        if (payload_src) {
            memcpy(heap_buf, payload_src, payload_size);
        }
    }
    int locked = __atomic_load_n(&g_bus_has_pinned, __ATOMIC_ACQUIRE);
    if (locked) pthread_mutex_lock(&q->lock);
    if (q->tail == q->cap) {
        /* Compact first: slide live cells to the front. */
        size_t live = q->tail - q->head;
        if (q->head > 0) {
            memmove(q->cells, q->cells + q->head,
                    live * sizeof(lotus_bus_cell_t));
            q->head = 0;
            q->tail = live;
        }
        if (q->tail == q->cap) {
            /* Truly full — double the capacity. */
            size_t new_cap = q->cap * 2;
            lotus_bus_cell_t *new_cells = (lotus_bus_cell_t *)
                realloc(q->cells, new_cap * sizeof(lotus_bus_cell_t));
            if (!new_cells) {
                if (locked) pthread_mutex_unlock(&q->lock);
                if (heap_buf) free(heap_buf);
                return;     /* drop on OOM */
            }
            q->cells = new_cells;
            q->cap   = new_cap;
        }
    }
    lotus_bus_cell_t *slot = &q->cells[q->tail++];
    slot->handler      = handler;
    slot->self_ptr     = self_ptr;
    slot->payload_size = payload_size;
    slot->payload_heap = heap_buf;
    if (!heap_buf && payload_size > 0 && payload_src) {
        memcpy(slot->payload_inline, payload_src, payload_size);
    }
    if (locked) pthread_mutex_unlock(&q->lock);
}

/* Drain the queue: pop cells one at a time and invoke
 * handler(self, payload). Handlers may enqueue more cells
 * (cooperative-cooperative bus dispatch is the natural
 * interleaving — see The Design / lotus, substrate cells).
 * Loops until the queue is empty AT POP TIME, including any
 * cells enqueued during the drain itself.
 *
 * Payload-pointer lifetime: the pointer handed to the handler
 * is valid for the duration of that handler invocation only.
 * The handler reads field values out of it (typical pattern:
 * `self.total = self.total + payload.value`); field copies
 * land in self, the pointer itself does not escape. Hale
 * doesn't allow taking explicit addresses in user code, so
 * this invariant is structurally enforced. Per spec/memory.md
 * § "Bus dispatch: copy-not-pointer semantic", the *value*
 * crosses the locus boundary (via the cell's inline buffer);
 * what changes here is that the value no longer bounces
 * through the subscriber's arena before the handler reads it
 * — a per-event `lotus_arena_alloc` + second `memcpy` that
 * dominated the cost for the small-payload event-flood case
 * (`bus_dispatch`/`stream_aggregator`/`pipeline_3stage`-style).
 *
 * Lock discipline (locked path): take the mutex to pop one cell
 * INTO a stack-local snapshot; release before invoking the
 * handler. The snapshot's `payload_inline` field IS the
 * canonical copy for this dispatch — handler reads through
 * `&snapshot.payload_inline`. Holding the lock across handler
 * invocation would (a) block pinned producers for the entire
 * handler runtime and (b) deadlock if the handler re-enqueues.
 *
 * Single-threaded path: a single stack buffer outside the loop
 * receives the cell's payload before each handler invocation.
 * Required because the handler may publish, which may realloc
 * `q->cells`, which would dangle a direct pointer into the
 * cell. Recursive drain calls (via the handler's trailing
 * bus_drain) get their own stack frame and their own buffer. */
typedef void (*lotus_handler_fn)(void *self, void *payload);

void lotus_bus_queue_drain(lotus_bus_queue_t *q) {
    if (!q) return;
    int locked = __atomic_load_n(&g_bus_has_pinned, __ATOMIC_ACQUIRE);
    if (locked) {
        /* Concurrent producers possible — must snapshot each cell
         * under the lock so the cells array can't be realloc'd out
         * from under the in-flight pop. The snapshot's inline
         * buffer is what the handler reads. */
        for (;;) {
            pthread_mutex_lock(&q->lock);
            if (q->head >= q->tail) {
                q->head = 0;
                q->tail = 0;
                pthread_mutex_unlock(&q->lock);
                return;
            }
            lotus_bus_cell_t cell_copy = q->cells[q->head++];
            pthread_mutex_unlock(&q->lock);

            void *payload_ptr = NULL;
            if (cell_copy.payload_size > 0) {
                payload_ptr = cell_copy.payload_heap
                    ? cell_copy.payload_heap
                    : (void *)cell_copy.payload_inline;
            }
            ((lotus_handler_fn)cell_copy.handler)(
                cell_copy.self_ptr, payload_ptr);
            if (cell_copy.payload_heap) free(cell_copy.payload_heap);
        }
    } else {
        /* Single-threaded cooperative path: no concurrent producer
         * exists. One stack-allocated payload buffer, reused
         * across iterations and stable across recursive drain
         * calls (the recursive call has its own frame). */
        unsigned char stack_payload[LOTUS_PAYLOAD_INLINE]
            __attribute__((aligned(16)));
        for (;;) {
            if (q->head >= q->tail) {
                q->head = 0;
                q->tail = 0;
                return;
            }
            lotus_bus_cell_t *cell = &q->cells[q->head++];
            void *handler_fn = cell->handler;
            void *handler_self = cell->self_ptr;
            size_t psize = cell->payload_size;
            void *heap_ptr = cell->payload_heap;
            void *payload_ptr = NULL;
            if (heap_ptr) {
                /* Heap pointer is stable across handler-driven
                 * q->cells realloc — hand it through directly,
                 * free after handler returns. */
                payload_ptr = heap_ptr;
            } else if (psize > 0) {
                /* Last cell-dereference before invoking the
                 * handler. After this memcpy, any handler-side
                 * realloc of q->cells is harmless — we're done
                 * reading from `cell`. */
                memcpy(stack_payload, cell->payload_inline, psize);
                payload_ptr = stack_payload;
            }
            ((lotus_handler_fn)handler_fn)(handler_self, payload_ptr);
            if (heap_ptr) free(heap_ptr);
        }
    }
}

void lotus_bus_queue_destroy(lotus_bus_queue_t *q) {
    if (!q) return;
    pthread_mutex_destroy(&q->lock);
    if (q->cells) free(q->cells);
    free(q);
}

/*
 * Per-pinned-locus mailbox (m28b stage 2).
 *
 * Each pinned locus that declares `bus subscribe` gets its own
 * mailbox: same cell shape as the global queue, plus a condvar
 * + shutdown flag. Cross-thread publishers (cooperative or
 * pinned) call lotus_mailbox_post to drop a cell into the
 * subscriber's mailbox; the pinned thread's main loop calls
 * lotus_mailbox_drain_one to pull one cell at a time, copy
 * its inline payload into the locus's arena, and invoke the
 * handler — handler-atomic per substrate cell, just like the
 * cooperative drain.
 *
 * post → broadcasts the not_empty condvar so a thread waiting
 * in drain_one wakes up.
 *
 * drain_one blocks on the condvar until either:
 *   - a cell arrives (returns 1 after invoking the handler), or
 *   - shutdown is signaled and the queue is empty (returns 0).
 *
 * shutdown sets the flag + broadcasts so all waiters return.
 * The pinned thread observes return=0, breaks its loop, runs
 * its drain/dissolve, and exits — main thread then joins.
 *
 * Per The Design / lotus, this is the canonical "any → pinned"
 * bus path: publisher and subscriber sit in different layers
 * of the lotus, the cost lives at the boundary (the mailbox
 * lock + the inline payload's two memcpy's), and each arena
 * stays single-threaded territory.
 */

typedef struct lotus_mailbox {
    lotus_bus_cell_t *cells;
    size_t            head;
    size_t            tail;
    size_t            cap;
    int               shutdown;
    pthread_mutex_t   lock;
    pthread_cond_t    not_empty;
} lotus_mailbox_t;

#define LOTUS_MAILBOX_INITIAL_CAP 64

lotus_mailbox_t *lotus_mailbox_create(void) {
    lotus_mailbox_t *mb =
        (lotus_mailbox_t *)malloc(sizeof(lotus_mailbox_t));
    if (!mb) return NULL;
    mb->cap   = LOTUS_MAILBOX_INITIAL_CAP;
    mb->cells = (lotus_bus_cell_t *)
        malloc(mb->cap * sizeof(lotus_bus_cell_t));
    if (!mb->cells) {
        free(mb);
        return NULL;
    }
    mb->head     = 0;
    mb->tail     = 0;
    mb->shutdown = 0;
    pthread_mutex_init(&mb->lock, NULL);
    pthread_cond_init(&mb->not_empty, NULL);
    return mb;
}

void lotus_mailbox_post(lotus_mailbox_t *mb,
                        void *handler,
                        void *self_ptr,
                        const void *payload_src,
                        size_t payload_size) {
    if (!mb) return;
    /* Two-tier payload storage; see queue_enqueue for the
     * design rationale. */
    void *heap_buf = NULL;
    if (payload_size > LOTUS_PAYLOAD_INLINE) {
        heap_buf = malloc(payload_size);
        if (!heap_buf) return;
        if (payload_src) {
            memcpy(heap_buf, payload_src, payload_size);
        }
    }
    pthread_mutex_lock(&mb->lock);
    if (mb->tail == mb->cap) {
        size_t live = mb->tail - mb->head;
        if (mb->head > 0) {
            memmove(mb->cells, mb->cells + mb->head,
                    live * sizeof(lotus_bus_cell_t));
            mb->head = 0;
            mb->tail = live;
        }
        if (mb->tail == mb->cap) {
            size_t new_cap = mb->cap * 2;
            lotus_bus_cell_t *new_cells = (lotus_bus_cell_t *)
                realloc(mb->cells, new_cap * sizeof(lotus_bus_cell_t));
            if (!new_cells) {
                pthread_mutex_unlock(&mb->lock);
                if (heap_buf) free(heap_buf);
                return;
            }
            mb->cells = new_cells;
            mb->cap   = new_cap;
        }
    }
    lotus_bus_cell_t *slot = &mb->cells[mb->tail++];
    slot->handler      = handler;
    slot->self_ptr     = self_ptr;
    slot->payload_size = payload_size;
    slot->payload_heap = heap_buf;
    if (!heap_buf && payload_size > 0 && payload_src) {
        memcpy(slot->payload_inline, payload_src, payload_size);
    }
    pthread_cond_broadcast(&mb->not_empty);
    pthread_mutex_unlock(&mb->lock);
}

int lotus_mailbox_drain_one(lotus_mailbox_t *mb) {
    if (!mb) return 0;
    pthread_mutex_lock(&mb->lock);
    while (mb->head >= mb->tail && !mb->shutdown) {
        pthread_cond_wait(&mb->not_empty, &mb->lock);
    }
    if (mb->head >= mb->tail) {
        /* shutdown with empty queue */
        mb->head = 0;
        mb->tail = 0;
        pthread_mutex_unlock(&mb->lock);
        return 0;
    }
    lotus_bus_cell_t cell_copy = mb->cells[mb->head++];
    if (mb->head >= mb->tail) {
        mb->head = 0;
        mb->tail = 0;
    }
    pthread_mutex_unlock(&mb->lock);

    /* Hand the dequeued cell's payload to the handler.
     * `cell_copy` is a stack-local snapshot of the dequeued
     * cell; its inline buffer (or its heap pointer) is the
     * canonical payload copy for this dispatch. Skipping the
     * prior `lotus_arena_alloc` + extra memcpy into the
     * locus's arena drops the per-event overhead on the
     * pinned-subscriber path. See the matching note in
     * lotus_bus_queue_drain — same lifetime invariant. The
     * heap-spill case (payload > LOTUS_PAYLOAD_INLINE) hands
     * the malloc'd buffer through directly and frees it after
     * the handler returns. */
    void *payload_ptr = NULL;
    if (cell_copy.payload_size > 0) {
        payload_ptr = cell_copy.payload_heap
            ? cell_copy.payload_heap
            : (void *)cell_copy.payload_inline;
    }
    ((lotus_handler_fn)cell_copy.handler)(
        cell_copy.self_ptr, payload_ptr);
    if (cell_copy.payload_heap) free(cell_copy.payload_heap);
    return 1;
}

void lotus_mailbox_shutdown(lotus_mailbox_t *mb) {
    if (!mb) return;
    pthread_mutex_lock(&mb->lock);
    mb->shutdown = 1;
    pthread_cond_broadcast(&mb->not_empty);
    pthread_mutex_unlock(&mb->lock);
}

void lotus_mailbox_destroy(lotus_mailbox_t *mb) {
    if (!mb) return;
    pthread_cond_destroy(&mb->not_empty);
    pthread_mutex_destroy(&mb->lock);
    if (mb->cells) free(mb->cells);
    free(mb);
}

/*
 * the coop→pinned drain friction (2026-05-23) — coop→pinned mid-program drain.
 *
 * Pre-fix: a pinned locus's mailbox loop only enters AFTER its
 * run() returns. Long-running pinned servers (typical mdgw
 * shape) never return from run(), so cooperative publishers
 * could enqueue cells but the handler never fired until
 * dissolve-time drain — far too late for any kind of CnC or
 * boot-config message flow.
 *
 * Post-fix: the pinned thread stashes its mailbox pointer in
 * TLS at __pinned_main entry. `time::sleep` and explicit
 * `yield;` (anywhere on that thread) call
 * `lotus_mailbox_drain_pending` against the TLS-cached
 * pointer, draining every currently-queued cell without
 * blocking. Cells posted during the sleep window become
 * mid-program-visible.
 *
 * The non-blocking variant (vs `lotus_mailbox_drain_one`'s
 * blocking wait) is the load-bearing distinction. On a
 * cooperative thread it's a no-op (TLS is null); on a pinned
 * thread without subscriptions it's a no-op (mailbox is
 * null).
 */

static __thread lotus_mailbox_t *g_current_pinned_mailbox = NULL;

void lotus_mailbox_set_current(lotus_mailbox_t *mb) {
    g_current_pinned_mailbox = mb;
}

lotus_mailbox_t *lotus_mailbox_get_current(void) {
    return g_current_pinned_mailbox;
}

/* Drain every cell currently in the mailbox, then return.
 * Unlike drain_one, this does NOT block on the condvar — if
 * the queue is empty we return immediately. Designed to be
 * called from yield/sleep points on the pinned locus's own
 * thread; cells posted DURING this call land in the queue
 * and are seen by the next iteration. Handlers run with the
 * lock released so they can publish further cells (the post
 * path re-acquires the lock).
 */
void lotus_mailbox_drain_pending(lotus_mailbox_t *mb) {
    if (!mb) return;
    for (;;) {
        pthread_mutex_lock(&mb->lock);
        if (mb->head >= mb->tail) {
            /* Empty — done. Don't reset head/tail when shutdown
             * is signaled mid-sleep; the regular drain_one in
             * __pinned_main handles that path. */
            pthread_mutex_unlock(&mb->lock);
            return;
        }
        lotus_bus_cell_t cell_copy = mb->cells[mb->head++];
        if (mb->head >= mb->tail) {
            mb->head = 0;
            mb->tail = 0;
        }
        pthread_mutex_unlock(&mb->lock);
        void *payload_ptr = NULL;
        if (cell_copy.payload_size > 0) {
            payload_ptr = cell_copy.payload_heap
                ? cell_copy.payload_heap
                : (void *)cell_copy.payload_inline;
        }
        ((lotus_handler_fn)cell_copy.handler)(
            cell_copy.self_ptr, payload_ptr);
        if (cell_copy.payload_heap) free(cell_copy.payload_heap);
    }
}

/*
 * F.31 Phase 4: cooperative-pool worker threads (M:N substrate).
 *
 * The placement block on `main locus` partitions cooperative loci
 * across named pools. Each pool gets its own OS thread that runs
 * a cooperative drain loop — handler-atomic per substrate cell,
 * just like the original cooperative thread, but multiple pools
 * can be running concurrently. Pool "main" is implicit and runs
 * on the binary's primary thread (drained by the existing
 * flush_dissolve_frame / explicit yield / time::sleep machinery);
 * pools named anything else are spawned at startup.
 *
 * The pool queue uses the same `lotus_bus_cell_t` shape as
 * mailbox/queue so bus dispatch can route cells uniformly:
 *   - mailbox (non-null) → pinned subscriber → mailbox_post
 *   - coop_pool (non-null) → non-main cooperative pool → coop_pool_post
 *   - else → main-thread cooperative queue → bus_queue_enqueue
 *
 * Registry is a small linear array keyed by name. Phase 4 v1
 * does not support dynamic registration mid-run; all pools are
 * registered at main's prelude and torn down at exit. Lookup is
 * O(N) on the pool count, which is typically 1-4 — name-based
 * lookup is rare anyway (most call sites cache the pool ptr at
 * registration time).
 *
 * Single-threaded-method invariant (spec/types.md F.31): each
 * locus's methods run only on its pool's thread. The
 * typechecker rejects cross-pool method calls; bus delivery is
 * the legal cross-pool path. This C runtime is the
 * substrate-side enforcement: cells routed to a pool's queue
 * only ever fire on that pool's thread.
 */

/* F.35 Slice 1: ucontext-backed coroutine state for one in-flight
 * handler invocation on an async_io pool. Each pool worker maintains
 * a per-invocation `lotus_coro_t` so the handler can `park_on_fd`
 * (swapcontext back to drain), let the pool service other work, and
 * later resume from where it parked. Linked into the pool's
 * `parked_head` list while waiting on epoll; freed when the handler
 * returns naturally. */
typedef struct lotus_coro {
    ucontext_t        ctx;          /* saved registers + SP for resume */
    void             *stack;        /* mmap'd or malloc'd stack base */
    size_t            stack_size;   /* alloc'd size of `stack` */
    int               parked_fd;    /* fd this coro is parked on (-1 when running) */
    int               done;         /* 1 once the handler has returned */
    /* Handler invocation parameters captured at coro creation. The
     * thunk reads them after swapcontext and tail-calls the handler. */
    void             *handler;
    void             *self_ptr;
    void             *payload_ptr;
    /* Intrusive list pointers — used for the pool's `parked_head`
     * chain (when this coro is waiting on epoll) and the pool's
     * free-list of reusable coro slots (later). */
    struct lotus_coro *next;
} lotus_coro_t;

typedef struct lotus_coop_pool {
    /* Name as registered (null-terminated, <= 63 chars). Stored
     * inline so lookup doesn't chase an extra pointer. Pool
     * names are typically short ("io", "render", "main") so 64
     * bytes covers every realistic case. */
    char              name[64];
    /* Cell ring buffer + tail/head/cap — same shape as mailbox. */
    lotus_bus_cell_t *cells;
    size_t            head;
    size_t            tail;
    size_t            cap;
    int               shutdown;
    pthread_mutex_t   lock;
    pthread_cond_t    not_empty;
    /* Worker pthread; set once start_all has run. */
    pthread_t         worker;
    int               worker_started;
    /* F.35 Slice 1: async_io state. Dormant when `async_io_enabled`
     * is 0 — pool runs the classic blocking-syscall worker loop.
     * When non-zero, `epoll_fd` is open and the worker uses the
     * coro-dispatching variant of `drain_one` so handlers can park
     * via `lotus_coop_park_on_fd`.
     *
     * `drain_ctx` is the worker thread's own context — the swap
     * target when a coro parks or returns. `current_coro` tracks
     * which coro is on-CPU at any moment so `park_on_fd` (called
     * from within a handler) knows whose context to save.
     *
     * `parked_head` is the linked list of coros parked on this
     * pool's epoll. epoll_wait wakeups walk it to find which coro
     * to resume. */
    int               async_io_enabled;
    int               epoll_fd;
    ucontext_t        drain_ctx;
    lotus_coro_t     *current_coro;
    lotus_coro_t     *parked_head;
} lotus_coop_pool_t;

/* F.35 Slice 1: per-coro stack size. 64 KiB is the same default the
 * pthread library uses for "small" stacks; covers handler bodies
 * with reasonable depth without inflating per-coro memory cost.
 * Tunable via the F.35 follow-on `@stack_size(N)` annotation if a
 * workload ever needs a different default per locus. */
#define LOTUS_CORO_STACK_BYTES (64 * 1024)

#define LOTUS_COOP_POOL_INITIAL_CAP 64
#define LOTUS_COOP_POOL_MAX 16

static lotus_coop_pool_t *g_coop_pools[LOTUS_COOP_POOL_MAX];
static size_t             g_coop_pool_count = 0;

lotus_coop_pool_t *lotus_coop_pool_lookup(const char *name) {
    if (!name) return NULL;
    for (size_t i = 0; i < g_coop_pool_count; i++) {
        if (strcmp(g_coop_pools[i]->name, name) == 0) {
            return g_coop_pools[i];
        }
    }
    return NULL;
}

/* Register (or look up) a cooperative pool by name. Idempotent —
 * calling twice with the same name returns the same pool ptr,
 * which simplifies codegen (emit the call per-subscriber
 * registration; the first call wins, subsequent calls just
 * resolve to it). Pool "main" is a no-op: it returns NULL since
 * its work runs inline on the main thread's existing
 * cooperative queue. */
lotus_coop_pool_t *lotus_coop_pool_register(const char *name) {
    if (!name) return NULL;
    if (strcmp(name, "main") == 0) return NULL;
    lotus_coop_pool_t *existing = lotus_coop_pool_lookup(name);
    if (existing) return existing;
    if (g_coop_pool_count >= LOTUS_COOP_POOL_MAX) return NULL;
    lotus_coop_pool_t *p =
        (lotus_coop_pool_t *)malloc(sizeof(lotus_coop_pool_t));
    if (!p) return NULL;
    p->cells = (lotus_bus_cell_t *)
        malloc(LOTUS_COOP_POOL_INITIAL_CAP * sizeof(lotus_bus_cell_t));
    if (!p->cells) { free(p); return NULL; }
    p->cap  = LOTUS_COOP_POOL_INITIAL_CAP;
    p->head = 0;
    p->tail = 0;
    p->shutdown = 0;
    p->worker_started = 0;
    pthread_mutex_init(&p->lock, NULL);
    pthread_cond_init(&p->not_empty, NULL);
    /* F.35 Slice 1: async_io defaults off. Slice 2's codegen flips
     * this via `lotus_coop_pool_enable_async_io` for pools whose
     * placement entries declare `where async_io`. */
    p->async_io_enabled = 0;
    p->epoll_fd         = -1;
    p->current_coro     = NULL;
    p->parked_head      = NULL;
    /* Truncated copy — the bound is LOTUS_COOP_POOL_MAX-name's-
     * worth and pool names are conventionally short. Anything
     * longer than 63 bytes is the caller's fault and gets
     * clipped silently. */
    size_t n = strlen(name);
    if (n > 63) n = 63;
    memcpy(p->name, name, n);
    p->name[n] = '\0';
    g_coop_pools[g_coop_pool_count++] = p;
    return p;
}

void lotus_coop_pool_post(lotus_coop_pool_t *p,
                          void *handler,
                          void *self_ptr,
                          const void *payload_src,
                          size_t payload_size) {
    if (!p) return;
    /* Two-tier payload storage; see queue_enqueue for the
     * design rationale. */
    void *heap_buf = NULL;
    if (payload_size > LOTUS_PAYLOAD_INLINE) {
        heap_buf = malloc(payload_size);
        if (!heap_buf) return;
        if (payload_src) {
            memcpy(heap_buf, payload_src, payload_size);
        }
    }
    pthread_mutex_lock(&p->lock);
    if (p->tail == p->cap) {
        size_t live = p->tail - p->head;
        if (p->head > 0) {
            memmove(p->cells, p->cells + p->head,
                    live * sizeof(lotus_bus_cell_t));
            p->head = 0;
            p->tail = live;
        }
        if (p->tail == p->cap) {
            size_t new_cap = p->cap * 2;
            lotus_bus_cell_t *new_cells = (lotus_bus_cell_t *)
                realloc(p->cells, new_cap * sizeof(lotus_bus_cell_t));
            if (!new_cells) {
                pthread_mutex_unlock(&p->lock);
                if (heap_buf) free(heap_buf);
                return;
            }
            p->cells = new_cells;
            p->cap   = new_cap;
        }
    }
    lotus_bus_cell_t *slot = &p->cells[p->tail++];
    slot->handler      = handler;
    slot->self_ptr     = self_ptr;
    slot->payload_size = payload_size;
    slot->payload_heap = heap_buf;
    if (!heap_buf && payload_size > 0 && payload_src) {
        memcpy(slot->payload_inline, payload_src, payload_size);
    }
    /* F.32-4-prefetch (2026-05-24): hint the receiver's L1 to
     * pull the freshly-written slot's cache line. The receiver
     * pool's drain loop will read these bytes within ~µs; the
     * prefetch arrives well ahead of the consumer's load,
     * eliminating the cache-miss stall on the consumer side
     * (~10-50 ns saved per cell). Write-intent locality 3
     * (high temporal reuse) because the drain pops + reads
     * the cell almost immediately. Zero-cost on the producer
     * side — single instruction, no stall.
     *
     * Build-flag toggle (2026-05-25): set
     * LOTUS_DISABLE_PREFETCH=1 in the build environment to
     * compile this site out. Used by the A/B harness to
     * isolate the prefetch's perf contribution on
     * `bus_dispatch_cross_pool`. Default (env unset): prefetch
     * enabled — matches shipped behavior. */
#ifndef LOTUS_DISABLE_PREFETCH
    __builtin_prefetch(slot, 1, 3);
#endif
    pthread_cond_broadcast(&p->not_empty);
    pthread_mutex_unlock(&p->lock);
}

/* Drain a single cell on the pool's worker thread. Blocks on
 * the condvar until a cell is available or shutdown is
 * signaled with empty queue. Returns 1 after a cell ran, 0
 * after shutdown-empty. */
static int lotus_coop_pool_drain_one(lotus_coop_pool_t *p) {
    pthread_mutex_lock(&p->lock);
    while (p->head >= p->tail && !p->shutdown) {
        pthread_cond_wait(&p->not_empty, &p->lock);
    }
    if (p->head >= p->tail) {
        p->head = 0;
        p->tail = 0;
        pthread_mutex_unlock(&p->lock);
        return 0;
    }
    lotus_bus_cell_t cell_copy = p->cells[p->head++];
    if (p->head >= p->tail) {
        p->head = 0;
        p->tail = 0;
    }
    pthread_mutex_unlock(&p->lock);
    void *payload_ptr = NULL;
    if (cell_copy.payload_size > 0) {
        payload_ptr = cell_copy.payload_heap
            ? cell_copy.payload_heap
            : (void *)cell_copy.payload_inline;
    }
    ((lotus_handler_fn)cell_copy.handler)(
        cell_copy.self_ptr, payload_ptr);
    if (cell_copy.payload_heap) free(cell_copy.payload_heap);
    return 1;
}

/* F.35 Slice 1: thread-locals tracking which pool / coro is on-CPU
 * on this worker thread. `park_on_fd` (called from inside a handler)
 * consults `g_current_coro_tls` to find itself, and the pool ptr is
 * cached on the coro at alloc time so park can reach the pool's
 * epoll fd + drain context. */
static __thread lotus_coro_t      *g_current_coro_tls = NULL;
static __thread lotus_coop_pool_t *g_current_pool_tls = NULL;

/* Pool-inheritance fix (2026-05-29): the cooperative pool whose
 * worker thread is currently on-CPU, or NULL if called from the
 * main thread (or any non-pool-worker thread). Codegen uses this
 * to route a child locus instantiated INSIDE a method/handler
 * body that is itself running on a pool worker — the child's
 * run() posts to, and its bus subscriptions register against,
 * the pool the parent is actually executing on. Compile-time
 * placement (main-locus params fields) still wins where it's
 * known; this is the fallback for the in-method-body case where
 * the codegen has no static pool name. */
lotus_coop_pool_t *lotus_coop_pool_current(void) {
    return g_current_pool_tls;
}

/* Enable async_io mode for a pool: opens an epoll fd. Idempotent;
 * safe to call before or after the worker thread starts (the worker
 * checks `async_io_enabled` on each loop iteration). Called from
 * Slice 2's codegen for pools whose placement entries declare
 * `where async_io`. Returns 0 on success, -1 on epoll_create
 * failure (caller decides whether to fall back to blocking mode). */
int lotus_coop_pool_enable_async_io(lotus_coop_pool_t *p) {
    if (!p) return -1;
    if (p->async_io_enabled) return 0;
    int fd = epoll_create1(EPOLL_CLOEXEC);
    if (fd < 0) {
        return -1;
    }
    p->epoll_fd = fd;
    /* Publish epoll_fd + enable flag together via release fence so
     * the worker thread (which checks the flag without holding the
     * pool lock) sees a consistent pair. */
    __atomic_store_n(&p->async_io_enabled, 1, __ATOMIC_RELEASE);
    return 0;
}

/* F.35 Slice 1: thunk invoked by makecontext. Reads handler+args
 * from the current coro (stashed before swapcontext), invokes the
 * handler on the coro's dedicated stack, then marks the coro done
 * and swaps back to the pool's drain context. */
static void lotus_coro_thunk(void) {
    lotus_coro_t *c = g_current_coro_tls;
    if (!c || !c->handler) {
        /* Defensive — shouldn't happen, but if it does, jump back
         * to the drain context so we don't run off the end of the
         * stack. */
        if (g_current_pool_tls) {
            setcontext(&g_current_pool_tls->drain_ctx);
        }
        return;
    }
    ((lotus_handler_fn)c->handler)(c->self_ptr, c->payload_ptr);
    c->done = 1;
    /* uc_link would handle this implicitly, but being explicit is
     * cheaper to reason about — and lets the worker recognize that
     * the coro is freeable rather than re-parked. */
    setcontext(&g_current_pool_tls->drain_ctx);
}

/* F.35 Slice 1: allocate a coro for one handler invocation. Each
 * invocation gets its own stack (mmap'd with guard pages would be
 * ideal; v1 uses malloc for portability, falls back to no guard).
 * Returns NULL on OOM. */
static lotus_coro_t *lotus_coro_alloc(lotus_coop_pool_t *p,
                                       void *handler,
                                       void *self_ptr,
                                       void *payload_ptr) {
    lotus_coro_t *c = (lotus_coro_t *)malloc(sizeof(lotus_coro_t));
    if (!c) return NULL;
    c->stack = malloc(LOTUS_CORO_STACK_BYTES);
    if (!c->stack) { free(c); return NULL; }
    c->stack_size  = LOTUS_CORO_STACK_BYTES;
    c->parked_fd   = -1;
    c->done        = 0;
    c->handler     = handler;
    c->self_ptr    = self_ptr;
    c->payload_ptr = payload_ptr;
    c->next        = NULL;
    if (getcontext(&c->ctx) != 0) {
        free(c->stack);
        free(c);
        return NULL;
    }
    c->ctx.uc_stack.ss_sp   = c->stack;
    c->ctx.uc_stack.ss_size = c->stack_size;
    c->ctx.uc_link          = &p->drain_ctx;
    makecontext(&c->ctx, lotus_coro_thunk, 0);
    return c;
}

static void lotus_coro_free(lotus_coro_t *c) {
    if (!c) return;
    if (c->stack) free(c->stack);
    free(c);
}

/* F.35 Slice 1: park the current coro on `fd`. Called from inside a
 * handler running on a coro stack (typically from inside a blocking-
 * I/O primitive that detected EAGAIN). Registers `fd` with the pool's
 * epoll, links the coro onto the pool's parked list, then swaps back
 * to the pool's drain context. Returns 0 when the coro resumes
 * (epoll said `fd` is ready); -1 on epoll error or pool shutdown.
 *
 * `events` is a bitmask of EPOLLIN / EPOLLOUT — what the caller
 * wants to wait for. EPOLLET / EPOLLONESHOT are not exposed; the
 * runtime uses level-triggered + manual deregistration so a coro
 * that wakes can re-park without an extra epoll_ctl cycle. */
int lotus_coop_park_on_fd(int fd, uint32_t events) {
    lotus_coop_pool_t *p = g_current_pool_tls;
    lotus_coro_t      *c = g_current_coro_tls;
    if (!p || !c) {
        /* Called from a non-async_io context — caller should have
         * checked. Defensive return-error so a stray call doesn't
         * silently corrupt state. */
        return -1;
    }
    if (!p->async_io_enabled || p->epoll_fd < 0) {
        return -1;
    }
    struct epoll_event ev;
    memset(&ev, 0, sizeof(ev));
    ev.events   = events;
    ev.data.ptr = c;
    if (epoll_ctl(p->epoll_fd, EPOLL_CTL_ADD, fd, &ev) < 0) {
        return -1;
    }
    c->parked_fd = fd;
    /* Link onto parked head — single-threaded access (only the
     * worker touches this list), no lock needed. */
    c->next = p->parked_head;
    p->parked_head = c;
    /* Swap to drain. Returns here when the worker swaps back in
     * after epoll_wait wakeup. The fd has already been removed
     * from epoll by the resume path before swap-back. */
    swapcontext(&c->ctx, &p->drain_ctx);
    /* Resumed. The resume path cleared parked_fd to -1 and
     * detached us from parked_head. */
    return 0;
}

/* F.35 Slice 1: async-aware drain. Runs in the pool worker thread.
 * Each iteration:
 *   1. epoll_wait if there are parked coros (with timeout 0 if the
 *      cell queue is non-empty so cells get priority; -1 / blocking
 *      if cells are empty and parked is non-empty).
 *   2. For each ready fd, find the parked coro, deregister the fd,
 *      detach from parked list, swapcontext into it. (Coro runs
 *      until it parks again or returns.)
 *   3. Drain one cell from the bus queue on a fresh coro stack.
 *
 * Returns 0 on shutdown-with-empty-queue-and-no-parked; 1 after a
 * cell or wakeup advanced state (caller loops). */
static int lotus_coop_pool_drain_one_async(lotus_coop_pool_t *p) {
    /* (1) Service parked-fd wakeups first. epoll_wait timeout
     * choice: 0 (non-blocking) when a cell is pending so we hand
     * cells to the dispatcher promptly; -1 (block) only when no
     * cells AND parked exist; skip entirely when neither holds. */
    if (p->parked_head) {
        pthread_mutex_lock(&p->lock);
        int cell_pending = (p->head < p->tail);
        int shutdown_set = p->shutdown;
        pthread_mutex_unlock(&p->lock);
        if (shutdown_set && !cell_pending) {
            /* Wake parked coros so they can observe shutdown and
             * unwind. Not implemented in Slice 1 — for now, just
             * return and let shutdown_all join the worker; the
             * parked coros leak their stacks until process exit. */
            return 0;
        }
        int timeout_ms = cell_pending ? 0 : -1;
        struct epoll_event events[16];
        int n = epoll_wait(p->epoll_fd, events, 16, timeout_ms);
        if (n < 0) {
            if (errno != EINTR) {
                /* epoll error — bail to caller; worker exits. */
                return 0;
            }
            n = 0;
        }
        for (int i = 0; i < n; i++) {
            lotus_coro_t *c = (lotus_coro_t *)events[i].data.ptr;
            if (!c) continue;
            /* Deregister fd; level-triggered semantics mean we MUST
             * detach now to avoid re-firing on the next epoll_wait. */
            (void)epoll_ctl(p->epoll_fd, EPOLL_CTL_DEL, c->parked_fd, NULL);
            /* Detach from parked list. */
            lotus_coro_t **pp = &p->parked_head;
            while (*pp && *pp != c) pp = &(*pp)->next;
            if (*pp == c) *pp = c->next;
            c->next      = NULL;
            c->parked_fd = -1;
            /* Resume the coro. Returns here when it parks again or
             * the handler returns. */
            g_current_coro_tls = c;
            swapcontext(&p->drain_ctx, &c->ctx);
            g_current_coro_tls = NULL;
            if (c->done) {
                lotus_coro_free(c);
            }
        }
        if (n > 0) return 1;
        if (cell_pending == 0) return 1;
    }
    /* (2) Drain one cell on a fresh coro stack. */
    pthread_mutex_lock(&p->lock);
    while (p->head >= p->tail && !p->shutdown) {
        /* If parked coros exist, we cannot block on the condvar
         * forever — epoll might be the wake signal. Drop the lock
         * and loop back to step (1). */
        if (p->parked_head) {
            pthread_mutex_unlock(&p->lock);
            return 1;
        }
        pthread_cond_wait(&p->not_empty, &p->lock);
    }
    if (p->head >= p->tail) {
        p->head = 0;
        p->tail = 0;
        pthread_mutex_unlock(&p->lock);
        return 0;
    }
    lotus_bus_cell_t cell_copy = p->cells[p->head++];
    if (p->head >= p->tail) {
        p->head = 0;
        p->tail = 0;
    }
    pthread_mutex_unlock(&p->lock);
    void *payload_ptr = NULL;
    if (cell_copy.payload_size > 0) {
        payload_ptr = cell_copy.payload_heap
            ? cell_copy.payload_heap
            : (void *)cell_copy.payload_inline;
    }
    /* Heap payload outlives the cell on the dispatch path; we copy
     * the pointer into the coro so the thunk can read it. The free
     * happens after the coro returns (parked or not). For Slice 1
     * we conservatively leak heap payloads on coro-park because
     * the coro retains the pointer until it resumes-and-completes.
     * Tighten in Slice 3 when blocking I/O is wired up and the
     * leak shape becomes observable. */
    lotus_coro_t *c =
        lotus_coro_alloc(p, cell_copy.handler, cell_copy.self_ptr, payload_ptr);
    if (!c) {
        /* OOM on coro alloc — fall back to direct invocation. The
         * handler runs on the worker's stack; if it parks via
         * `park_on_fd`, the call returns -1 (no current coro). */
        ((lotus_handler_fn)cell_copy.handler)(
            cell_copy.self_ptr, payload_ptr);
        if (cell_copy.payload_heap) free(cell_copy.payload_heap);
        return 1;
    }
    g_current_coro_tls = c;
    swapcontext(&p->drain_ctx, &c->ctx);
    g_current_coro_tls = NULL;
    if (c->done) {
        if (cell_copy.payload_heap) free(cell_copy.payload_heap);
        lotus_coro_free(c);
    }
    /* If c is not done (it parked), the cell's payload_heap is
     * retained by the coro and freed when it resumes-and-completes
     * (Slice 3 wiring). */
    return 1;
}

static void *lotus_coop_pool_worker(void *arg) {
    lotus_coop_pool_t *p = (lotus_coop_pool_t *)arg;
    g_current_pool_tls = p;
    /* F.35 Slice 1: async_io vs classic-blocking branch. The flag
     * is acquire-loaded once per outer loop iteration so a late
     * enable (called between worker start and first drain) takes
     * effect on the next pass. */
    while (1) {
        int async = __atomic_load_n(&p->async_io_enabled, __ATOMIC_ACQUIRE);
        int progressed;
        if (async) {
            progressed = lotus_coop_pool_drain_one_async(p);
        } else {
            progressed = lotus_coop_pool_drain_one(p);
        }
        if (!progressed) break;
    }
    g_current_pool_tls = NULL;
    return NULL;
}

void lotus_coop_pool_start_all(void) {
    /* Flip the bus-queue multi-thread flag BEFORE spawning any
     * worker. pthread_create's release semantics make the store
     * visible to the new thread, so the worker never observes
     * the flag unset on its first bus interaction. Required for
     * correctness on cross-pool cooperative workloads — without
     * this, two cooperative threads both took the unlocked
     * bus_queue_drain path and TSAN-raced on head/tail.
     * (2026-05-26 substrate-race fix.) */
    if (g_coop_pool_count > 0) {
        __atomic_store_n(&g_bus_has_pinned, 1, __ATOMIC_RELEASE);
        /* Subregion-freelist latch: pool workers create/destroy
         * handler-scratch subregions of shared parent arenas
         * concurrently with main. Set before the spawn loop so
         * the first transition happens single-threaded. */
        lotus_mark_multithreaded();
    }
    for (size_t i = 0; i < g_coop_pool_count; i++) {
        lotus_coop_pool_t *p = g_coop_pools[i];
        if (p->worker_started) continue;
        if (pthread_create(&p->worker, NULL,
                           lotus_coop_pool_worker, p) == 0) {
            p->worker_started = 1;
        }
    }
}

/* F.35 Slice 4 (2026-05-28): per-pool residency dump. Writes one
 * line per registered cooperative pool to the supplied fd, naming
 * the pool, its mode (async_io / blocking), parked coro count,
 * and pending cell-queue depth. Cheap to call; no signal-safety
 * guarantees (acquires per-pool mutex). Surfaced to user code as
 * `std::process::dump_pool_residency()` for embedding in heartbeat
 * ticks on long-running daemons. */
void lotus_coop_pool_dump_parked_counts(int fd) {
    if (fd < 0) return;
    char buf[256];
    int n = snprintf(buf, sizeof(buf),
                     "--- coop pool residency (count=%zu) ---\n",
                     g_coop_pool_count);
    if (n > 0) (void)write(fd, buf, (size_t)n);
    for (size_t i = 0; i < g_coop_pool_count; i++) {
        lotus_coop_pool_t *p = g_coop_pools[i];
        if (!p) continue;
        pthread_mutex_lock(&p->lock);
        size_t pending = (p->tail >= p->head) ? (p->tail - p->head) : 0;
        pthread_mutex_unlock(&p->lock);
        /* parked_head walk is safe without the pool lock because only
         * the worker thread mutates the list — readers see a possibly-
         * stale snapshot, which is the right semantic for a residency
         * dump (it's diagnostic, not a synchronization point). */
        size_t parked = 0;
        for (lotus_coro_t *c = p->parked_head; c; c = c->next) parked++;
        const char *mode = p->async_io_enabled ? "async_io" : "blocking";
        n = snprintf(buf, sizeof(buf),
                     "  [%s] mode=%s parked=%zu pending=%zu\n",
                     p->name, mode, parked, pending);
        if (n > 0) (void)write(fd, buf, (size_t)n);
    }
}

void lotus_coop_pool_shutdown_all(void) {
    /* Two-phase: signal all pools first so they can drain
     * pending cells in parallel, then join. */
    for (size_t i = 0; i < g_coop_pool_count; i++) {
        lotus_coop_pool_t *p = g_coop_pools[i];
        if (!p->worker_started) continue;
        pthread_mutex_lock(&p->lock);
        p->shutdown = 1;
        pthread_cond_broadcast(&p->not_empty);
        pthread_mutex_unlock(&p->lock);
    }
    for (size_t i = 0; i < g_coop_pool_count; i++) {
        lotus_coop_pool_t *p = g_coop_pools[i];
        if (!p->worker_started) continue;
        pthread_join(p->worker, NULL);
        p->worker_started = 0;
    }
}

/* Tear down the pool registry. Called at program exit AFTER
 * shutdown_all. Frees cell buffers + the pool structs. F.35 Slice 1:
 * also close the epoll fd and free any still-parked coros (their
 * stacks would otherwise leak — process exit reclaims either way,
 * but explicit free keeps valgrind / leak detectors quiet). */
void lotus_coop_pool_destroy_all(void) {
    for (size_t i = 0; i < g_coop_pool_count; i++) {
        lotus_coop_pool_t *p = g_coop_pools[i];
        if (!p) continue;
        pthread_cond_destroy(&p->not_empty);
        pthread_mutex_destroy(&p->lock);
        if (p->cells) free(p->cells);
        if (p->epoll_fd >= 0) {
            close(p->epoll_fd);
            p->epoll_fd = -1;
        }
        lotus_coro_t *c = p->parked_head;
        while (c) {
            lotus_coro_t *next = c->next;
            lotus_coro_free(c);
            c = next;
        }
        p->parked_head = NULL;
        free(p);
        g_coop_pools[i] = NULL;
    }
    g_coop_pool_count = 0;
}

/*
 * Process-wide bus router (m45-followup proper-fix).
 *
 * Replaces the per-program LLVM-side {bus.entries, bus.count,
 * lotus.bus_dispatch} triple. Storage is a heap-allocated dynamic
 * vec that grows on demand, so adding a new subscription has no
 * compile-time-known capacity ceiling. Multiple instances of the
 * same subscribed locus type each get their own entry without
 * needing the m45 quickfix's INSTANCES_PER_TYPE multiplier.
 *
 * Entry shape mirrors the prior LLVM struct exactly: subject (NUL
 * marks deregistered, courtesy of `lotus_bus_quarantine_self`),
 * subscriber's locus self pointer, handler fn pointer, and an
 * optional mailbox (null = cooperative subscriber → cells go to
 * the program-wide queue; non-null = pinned subscriber → cells
 * post to that locus's mailbox).
 *
 * No mutex on the router itself: registration runs inside
 * single-threaded instantiation paths, dispatch's payload-copy
 * happens through the queue/mailbox locks, and quarantine runs on
 * the cooperative thread (pinned loci don't have closures so
 * never quarantine). If pinned-side registration ever lands, this
 * acquires a mutex.
 */

/* m60: per-payload deserializer signature. Codegen synthesizes
 * `__deserialize_T` per bus payload type and passes the fn ptr
 * to lotus_bus_register; the reader thread (m59) calls it on
 * recv'd wire bytes to reconstruct the struct before dispatching
 * to the local handler. v0.1 wire format is identity (memcpy of
 * sizeof(T) bytes), so the reconstructed struct equals the
 * publisher's original struct. Returns the size written into
 * `dst` on success, -1 on error. */
typedef ssize_t (*lotus_deserialize_fn)(const void *src,
                                        size_t n,
                                        void *dst,
                                        size_t cap);

typedef struct lotus_bus_entry {
    const char           *subject;
    void                 *self_ptr;
    void                 *handler;
    lotus_mailbox_t      *mailbox;
    lotus_deserialize_fn  deserialize;     /* m60: nullable */
    /* F.31 Phase 4: cooperative-pool routing. Non-null when the
     * subscriber's enclosing locus is placed on a named pool
     * other than `main`; the dispatch path posts the cell to
     * this pool's queue instead of the global cooperative queue.
     * Null for both pinned subscribers (mailbox carries them)
     * and main-thread cooperative subscribers (global queue
     * still). Mutually exclusive with mailbox by construction;
     * dispatch checks mailbox first, then coop_pool. */
    lotus_coop_pool_t    *coop_pool;
    /* Phase 3 (2026-05-25): routing-key filter. See
     * `spec/semantics.md` § "Phase 3: routing keys". 0 = no
     * filter (today's default; behaves as before); 1 = specific-
     * key filter, only fire when published key equals (key_lo,
     * key_hi); 2 = catch-unmatched fallback (reserved for
     * v0.2 of the impl when the fallback policy lands). u128 key
     * stored uniformly as two u64 halves; narrower scalar types
     * zero-extend at register time. */
    uint8_t               key_filter_kind;
    uint64_t              key_lo;
    uint64_t              key_hi;
} lotus_bus_entry_t;

static lotus_bus_entry_t *g_bus_entries = NULL;
static size_t             g_bus_count   = 0;
static size_t             g_bus_cap     = 0;

/* Per-thread scratch for the wire dispatch path (2026-05-29).
 * These were `char buf[LOTUS_PAYLOAD_MAX]` (64 KiB) stack arrays
 * inside lotus_bus_dispatch{,_keyed} (the publish-side serialize
 * buffer) and lotus_bus_dispatch_wire{,_keyed} (the per-subscriber
 * deserialize buffer). A serialized publish from inside an F.35
 * async_io coro handler runs `dispatch -> dispatch_wire`, putting
 * 64 KiB + 64 KiB on the coro's 64 KiB stack
 * (LOTUS_CORO_STACK_BYTES) — a guaranteed stack overflow (SIGSEGV
 * in the dispatch_wire prologue, frame under lotus_coro_thunk).
 * Moving them to thread-local storage takes them off the coro
 * stack. Coro-safe: only one coro runs per worker at a time, and
 * neither dispatch nor dispatch_wire parks between filling the
 * buffer and copying it out (deserialize + enqueue, no I/O), so a
 * given thread's buffer is never aliased by a second coro
 * mid-use. `_wire` (publish serialize output) and `_struct`
 * (per-sub deserialize) are distinct because the dispatch ->
 * dispatch_wire chain holds both live at once. Reader threads keep
 * their own stack arrays (they run on full-size pthreads). */
static __thread char g_tls_bus_wire_buf[LOTUS_PAYLOAD_MAX];
static __thread char g_tls_bus_struct_buf[LOTUS_PAYLOAD_MAX];

#define LOTUS_BUS_ROUTER_INITIAL_CAP 16

/* m94: subject wildcard matching.
 *
 * v0 supports one wildcard form: a trailing "**" that matches
 * zero or more remaining dot-separated segments. So "log.app.**"
 * matches "log.app" (the root), "log.app.db", "log.app.db.query"
 * — the publishing logger's own subject AND any descendant.
 * This is the cascade-friendly semantics: subscribing to
 * `log.app.**` captures the whole sub-tree including its root.
 *
 * "**" must appear at the end of the pattern, preceded either by
 * "." or by nothing (the bare "**" pattern matches every subject).
 * "**" in any other position rejects.
 *
 * Returns 1 on match, 0 otherwise. NULL inputs are treated as
 * non-matching. Patterns without "**" fall through to strcmp —
 * the cheap path stays cheap.
 */
int lotus_subject_match(const char *pattern, const char *subject) {
    if (!pattern || !subject) return 0;
    /* Pointer-equal fast path: both sides typically reference the
     * same merged `unnamed_addr` global. LLVM coalesces identical
     * string constants, so `subscribe "S"` + `<- "S"` use the
     * same address. Skips strlen + strstr + strcmp for the
     * common literal-subject case (`bus_dispatch` / `stream_*`
     * patterns) — ~5-10 ns/publish-per-subscriber on a no-
     * wildcard subject. */
    if (pattern == subject) return 1;
    size_t plen = strlen(pattern);
    if (plen < 2) {
        /* Too short to contain "**". */
        return strcmp(pattern, subject) == 0;
    }
    /* "**" is supported only as a trailing wildcard. Anywhere
     * else we treat as no match (rather than try-and-fail
     * matching) so a typo like "log.**.error" doesn't silently
     * match a stray subject. */
    if (pattern[plen - 1] == '*' && pattern[plen - 2] == '*') {
        if (plen == 2) {
            /* Bare "**" — matches every subject. */
            return 1;
        }
        /* Must be preceded by '.', else "log**" — invalid. */
        if (pattern[plen - 3] != '.') return 0;
        /* Pattern is "<root>." + "**". Two valid forms:
         *   - subject equals root (no trailing segments)
         *   - subject starts with "<root>." and has tail bytes
         */
        size_t root_len = plen - 3;        /* "<root>" length */
        size_t prefix_len = plen - 2;      /* "<root>." length */
        if (strlen(subject) == root_len &&
            strncmp(pattern, subject, root_len) == 0) {
            return 1;
        }
        if (strncmp(pattern, subject, prefix_len) != 0) return 0;
        return subject[prefix_len] != '\0';
    }
    /* Pattern contains "**" but not at the end — reject. */
    if (strstr(pattern, "**") != NULL) return 0;
    /* No wildcard. */
    return strcmp(pattern, subject) == 0;
}

/* m58: forward-declare the remote-transport fanout hooks defined
 * at the bottom of this file. Dispatch and router_destroy call
 * them after the local-table loops so cross-process subscribers
 * receive the same publishes that local subscribers do. The
 * remote table and load-config implementation live next to the
 * AF_UNIX transport section because they're tightly coupled to
 * lotus_transport_create / send / destroy. */
void lotus_bus_remote_fanout(const char *subject,
                             const void *payload,
                             size_t payload_size);
void lotus_bus_remote_destroy_all(void);

/* m59: subscriber-side reader thread support.
 *
 * Reader threads (one per LISTEN-role transport opened from the
 * deployment-config) loop on lotus_transport_recv and need to
 * dispatch incoming bytes into the same local-handler set that
 * in-process publishers reach via lotus_bus_dispatch. To do
 * that without plumbing the cooperative queue pointer through
 * the transport layer, codegen sets it on a global at boot via
 * lotus_bus_set_queue, and the reader thread calls
 * lotus_bus_local_dispatch which reads the global. Pinned
 * subscribers route via mailbox (thread-safe by construction);
 * cooperative subscribers enqueue onto the cooperative queue
 * (mutex-protected, see lotus_bus_queue_enqueue), so the
 * reader thread is safely a producer alongside main + any
 * pinned threads. */
void lotus_bus_local_dispatch(lotus_bus_queue_t *queue,
                              const char *subject,
                              const void *payload,
                              size_t payload_size);
void lotus_bus_set_queue(lotus_bus_queue_t *queue);

/* Forward declaration so lotus_bus_register can delegate. The
 * symbol is the F.31-extended register; its body lands just
 * below this thunk. */
void lotus_bus_register_with_pool(const char *subject,
                                  void *self_ptr,
                                  void *handler,
                                  lotus_mailbox_t *mailbox,
                                  lotus_deserialize_fn deserialize,
                                  lotus_coop_pool_t *coop_pool);

/* Phase 3 forward decl (2026-05-25). */
void lotus_bus_register_keyed(const char *subject,
                              void *self_ptr,
                              void *handler,
                              lotus_mailbox_t *mailbox,
                              lotus_deserialize_fn deserialize,
                              lotus_coop_pool_t *coop_pool,
                              uint8_t key_filter_kind,
                              uint64_t key_lo,
                              uint64_t key_hi);

void lotus_bus_register(const char *subject,
                        void *self_ptr,
                        void *handler,
                        lotus_mailbox_t *mailbox,
                        lotus_deserialize_fn deserialize) {
    lotus_bus_register_with_pool(subject, self_ptr, handler,
                                 mailbox, deserialize, NULL);
}

/* F.31 Phase 4 extension. Same as lotus_bus_register but with
 * an explicit cooperative-pool ptr — codegen passes the
 * subscriber's pool here when the subscriber's enclosing locus
 * is placed on a non-main cooperative pool. Pool ptr is NULL
 * for main-thread cooperative subscribers (legacy path) and
 * for pinned subscribers (mailbox carries them). */
void lotus_bus_register_with_pool(const char *subject,
                                  void *self_ptr,
                                  void *handler,
                                  lotus_mailbox_t *mailbox,
                                  lotus_deserialize_fn deserialize,
                                  lotus_coop_pool_t *coop_pool) {
    lotus_bus_register_keyed(subject, self_ptr, handler, mailbox,
                              deserialize, coop_pool,
                              /* key_filter_kind */ 0,
                              /* key_lo */ 0,
                              /* key_hi */ 0);
}

/* Phase 3 (2026-05-25, spec/semantics.md § "Phase 3: routing
 * keys"). Codegen-side: for any locus-bus `subscribe TOPIC ...
 * where key == EXPR;`, the locus's birth lifecycle calls this
 * with the evaluated EXPR value (zero-extended to u128) and
 * filter kind 1. Unkeyed subscribes still route through
 * lotus_bus_register_with_pool, which delegates here with
 * key_filter_kind = 0 (no filter, receive every message on
 * the subject — today's pre-Phase-3 behavior). The dispatch
 * walk in lotus_bus_local_dispatch_keyed honors the filter. */
void lotus_bus_register_keyed(const char *subject,
                              void *self_ptr,
                              void *handler,
                              lotus_mailbox_t *mailbox,
                              lotus_deserialize_fn deserialize,
                              lotus_coop_pool_t *coop_pool,
                              uint8_t key_filter_kind,
                              uint64_t key_lo,
                              uint64_t key_hi) {
    if (g_bus_count == g_bus_cap) {
        size_t new_cap = g_bus_cap == 0
            ? LOTUS_BUS_ROUTER_INITIAL_CAP
            : g_bus_cap * 2;
        lotus_bus_entry_t *grown = (lotus_bus_entry_t *)
            realloc(g_bus_entries, new_cap * sizeof(lotus_bus_entry_t));
        if (!grown) return;     /* drop on OOM — graceful degrade */
        g_bus_entries = grown;
        g_bus_cap     = new_cap;
    }
    lotus_bus_entry_t *e = &g_bus_entries[g_bus_count++];
    e->subject     = subject;
    e->self_ptr    = self_ptr;
    e->handler     = handler;
    e->mailbox     = mailbox;
    e->deserialize = deserialize;
    e->coop_pool   = coop_pool;
    e->key_filter_kind = key_filter_kind;
    e->key_lo      = key_lo;
    e->key_hi      = key_hi;
}

/* Forward decl: defined alongside the other LOTUS_BUS_LOG_*
 * helpers below. The dispatch fns use it to gate the broad
 * silent-drop diagnostic. */
static int lotus_bus_log_drop_enabled(void);

/* Dispatch a published message to every subscriber of `subject`.
 * `queue` is the program-wide cooperative queue (passed in by
 * codegen rather than C-runtime-owned because the queue's
 * lifecycle is bound to main's prelude/exit, not to whatever
 * first triggers a register). Pinned subscribers route via their
 * mailbox; cooperative subscribers enqueue onto `queue`. */
/* m59 refactor: extracted from lotus_bus_dispatch so the m59
 * reader thread can dispatch recv'd bytes into the same local-
 * handler set without going through transport fanout (which
 * would re-emit them remotely and loop forever). */
void lotus_bus_local_dispatch(lotus_bus_queue_t *queue,
                              const char *subject,
                              const void *payload,
                              size_t payload_size) {
    if (!subject) return;
    size_t delivered = 0;
    for (size_t i = 0; i < g_bus_count; i++) {
        lotus_bus_entry_t *e = &g_bus_entries[i];
        if (!e->subject) continue;          /* deregistered */
        /* m94: pattern-match in case the subscriber registered a
         * wildcard subject (e.g. "log.**"). The fast path —
         * pattern with no '**' — costs one strcmp. */
        if (!lotus_subject_match(e->subject, subject)) continue;
        /* Phase 3 (2026-05-25): a subscriber with a specific-key
         * filter (kind=1) silently skips the unkeyed-publish path.
         * Unkeyed publishes should only land on receive-all
         * subscribers (kind=0). Otherwise an unkeyed `<- value`
         * would deliver to filtered subscribers regardless of the
         * filter — bypassing the routing-key contract. */
        if (e->key_filter_kind != 0) continue;
        if (e->mailbox) {
            lotus_mailbox_post(e->mailbox, e->handler, e->self_ptr,
                               payload, payload_size);
            delivered++;
        } else if (e->coop_pool) {
            /* F.31 Phase 4: subscriber's locus is on a named
             * non-main cooperative pool. Route to that pool's
             * queue so the handler fires on the pool's worker
             * thread (single-threaded-method invariant). */
            lotus_coop_pool_post(e->coop_pool, e->handler, e->self_ptr,
                                 payload, payload_size);
            delivered++;
        } else if (queue) {
            lotus_bus_queue_enqueue(queue, e->handler, e->self_ptr,
                                    payload, payload_size);
            delivered++;
        }
    }
    if (delivered == 0 && lotus_bus_log_drop_enabled()) {
        fprintf(stderr,
                "[bus] publish dropped: no local subscribers for "
                "subject=\"%s\" (g_bus_count=%zu)\n",
                subject, g_bus_count);
    }
}

/* Phase 3 (2026-05-25): the keyed-dispatch core. Walks the same
 * g_bus_entries array but applies the routing-key filter at each
 * entry: specific-key subscribers (kind=1) fire only when the
 * stored (key_lo, key_hi) matches the published key; receive-all
 * subscribers (kind=0) fire on every keyed publish too (an
 * "audit-all" sink can subscribe without a key filter and still
 * see keyed traffic).
 *
 * v0.1 ships the swallow policy: when no specific-key match
 * fires, the message is dropped silently. The `fail` and
 * `fallback` policies (typecheck-rejected at v0.1 of the impl)
 * will land in a follow-up — the tri-state structure of
 * key_filter_kind already reserves kind=2 for catch-unmatched
 * fallback subscribers when that policy ships.
 *
 * Set `LOTUS_BUS_LOG_UNMATCHED=1` in the env for development-
 * time visibility into no-match keyed publishes. The diag walks
 * once more if no specific match fired and reports subscriber
 * counts on stderr. Off by default. */
static int lotus_bus_log_unmatched_enabled(void) {
    static int cached = -1;
    if (cached < 0) {
        const char *s = getenv("LOTUS_BUS_LOG_UNMATCHED");
        cached = (s && s[0] == '1') ? 1 : 0;
    }
    return cached || lotus_bus_log_drop_enabled();
}

/* 2026-05-28: LOTUS_BUS_LOG_DROP=1 is the broad superset env
 * var — it implies UNMATCHED + DESERIALIZE_DROP AND covers
 * additional silent-drop sites the narrower vars miss:
 *
 *   - publish dispatch with serialize_fn returning <= 0
 *   - publish dispatch via lotus_bus_local_dispatch /
 *     _dispatch_wire that matches zero subscribers
 *   - per-entry deserialize <= 0 on the LOCAL-fanout path
 *     (DESERIALIZE_DROP only covers the udp:// reader path)
 *   - remote-fanout send errors
 *
 * Reach for LOG_DROP first when investigating a "publish
 * appears to succeed but handler doesn't fire" symptom. The
 * narrower vars stay supported for their specific bring-up
 * scenarios (cross-topic multicast noise on udp reader,
 * routing-key miss accounting). */
static int lotus_bus_log_drop_enabled(void) {
    static int cached = -1;
    if (cached < 0) {
        const char *s = getenv("LOTUS_BUS_LOG_DROP");
        cached = (s && s[0] == '1') ? 1 : 0;
    }
    return cached;
}

/* 2026-05-28: LOTUS_BUS_LOG_DESERIALIZE_DROP=1 surfaces silent
 * drops in the udp:// reader thread when (a) no deserializer
 * is registered for the inbound subject, or (b) the deserialize
 * function returns <= 0 (size mismatch, bounded-read failure,
 * etc.). Off by default — the silent-skip is correct for
 * cross-topic noise on shared multicast groups, but during
 * bring-up the lack of any signal is load-bearing on debug
 * cycles. Three udp:// handoffs this week traced back to
 * silent-skip-on-deserialize. */
static int lotus_bus_log_deserialize_drop_enabled(void) {
    static int cached = -1;
    if (cached < 0) {
        const char *s = getenv("LOTUS_BUS_LOG_DESERIALIZE_DROP");
        cached = (s && s[0] == '1') ? 1 : 0;
    }
    return cached || lotus_bus_log_drop_enabled();
}

void lotus_bus_local_dispatch_keyed(lotus_bus_queue_t *queue,
                                     const char *subject,
                                     const void *payload,
                                     size_t payload_size,
                                     uint64_t key_lo,
                                     uint64_t key_hi) {
    if (!subject) return;
    int matched_specific = 0;
    size_t specific_subs_on_subject = 0;
    size_t unkeyed_subs_on_subject = 0;
    for (size_t i = 0; i < g_bus_count; i++) {
        lotus_bus_entry_t *e = &g_bus_entries[i];
        if (!e->subject) continue;
        if (!lotus_subject_match(e->subject, subject)) continue;
        if (e->key_filter_kind == 1) {
            specific_subs_on_subject++;
            if (e->key_lo != key_lo || e->key_hi != key_hi) continue;
            matched_specific = 1;
        } else if (e->key_filter_kind == 0) {
            unkeyed_subs_on_subject++;
        } else {
            /* kind == 2: catch-unmatched fallback subscriber.
             * Reserved for v0.2 of the impl; today's typecheck
             * rejects `on_unmatched: fallback` so kind=2 should
             * never appear at v0.1. Skip in the specific-match
             * pass; the fallback pass below would dispatch it. */
            continue;
        }
        if (e->mailbox) {
            lotus_mailbox_post(e->mailbox, e->handler, e->self_ptr,
                               payload, payload_size);
        } else if (e->coop_pool) {
            lotus_coop_pool_post(e->coop_pool, e->handler, e->self_ptr,
                                 payload, payload_size);
        } else if (queue) {
            lotus_bus_queue_enqueue(queue, e->handler, e->self_ptr,
                                    payload, payload_size);
        }
    }
    /* Phase 3 v0.2 hook: when no specific-key match fired AND
     * any fallback subscribers exist for this subject, fire
     * them. The typecheck currently rejects `on_unmatched:
     * fallback` at v0.1 so kind=2 never appears, but the
     * dispatch shape is here for the v0.2 wiring. */
    if (!matched_specific) {
        for (size_t i = 0; i < g_bus_count; i++) {
            lotus_bus_entry_t *e = &g_bus_entries[i];
            if (!e->subject) continue;
            if (!lotus_subject_match(e->subject, subject)) continue;
            if (e->key_filter_kind != 2) continue;
            if (e->mailbox) {
                lotus_mailbox_post(e->mailbox, e->handler, e->self_ptr,
                                   payload, payload_size);
            } else if (e->coop_pool) {
                lotus_coop_pool_post(e->coop_pool, e->handler,
                                     e->self_ptr, payload, payload_size);
            } else if (queue) {
                lotus_bus_queue_enqueue(queue, e->handler, e->self_ptr,
                                        payload, payload_size);
            }
        }
        if (lotus_bus_log_unmatched_enabled()) {
            fprintf(stderr,
                    "[bus] subject=\"%s\" key_lo=%" PRIu64
                    " key_hi=%" PRIu64
                    " no_specific_match (%zu specific subs on subject; "
                    "%zu unkeyed)\n",
                    subject, key_lo, key_hi,
                    specific_subs_on_subject,
                    unkeyed_subs_on_subject);
        }
    }
}

/* m105 (Wave B inbound): adapter-driven inbound dispatch.
 *
 * The symmetric inbound counterpart to lotus_bus_remote_fanout's
 * outbound path. An adapter locus's run-loop (or any user code
 * driving an adapter) calls this with the wire-format bytes it
 * just received from the protocol layer; the runtime looks up
 * the subject's deserialize fn (registered alongside the local
 * subscribers via codegen) to convert wire bytes back to in-
 * memory struct bytes, then hands those to lotus_bus_local_dispatch
 * for fanout into the local handler set.
 *
 * Mirrors the unix reader-thread path in
 * lotus_bus_reader_thread_main but exposed as a callable for
 * Hale code via the `std::bus::__local_dispatch` primitive.
 * Out-of-band (non-bus) recv loops can use this too; the only
 * contract is "wire_bytes is one whole serialized payload."
 *
 * Silent no-op if no subscriber for the subject (matches the
 * unix path) or if the wire-bytes fail deserialization.
 *
 * Forward decl here; body lives further down once
 * g_bus_queue_for_remote is in scope (parallel to the
 * lotus_bus_remote_fanout split).
 */
void lotus_bus_dispatch_wire(const char *subject,
                             const void *wire_bytes,
                             size_t wire_size);

/* Phase-3 Task 9 (2026-05-20): forward decl of the caller-arena
 * TLS pointer. Defined further down alongside the other
 * caller_arena helpers; needed here because lotus_bus_dispatch_wire
 * sets it per-subscriber to route deserialize allocations into
 * each subscriber's own __arena. */
extern __thread lotus_arena_t *lotus_current_caller_arena;

/* m70: lotus_bus_dispatch's signature grew a 5th arg — a per-
 * subject serialize fn pointer (NULL for cooperative-only
 * publishers; codegen always passes the right one for cross-
 * process-capable subjects). Local dispatch enqueues struct
 * bytes (in-memory layout); remote fanout serializes those
 * bytes through the supplied fn into the wire format the
 * reader thread will deserialize. Splitting local-vs-remote
 * here lets the wire format diverge from the in-memory struct
 * layout (variable-width Strings, length-prefixed) without
 * breaking the local in-process path. */
typedef ssize_t (*lotus_serialize_fn)(const void *src,
                                       void *dst,
                                       size_t cap);

/* Forward decl — the remote-entries table is defined further
 * down in this file. `lotus_bus_dispatch` checks this to skip
 * the serialize+fanout work when no remote subscribers exist. */
static inline int lotus_bus_has_remote_entries(void);

/* Phase 3 (2026-05-25): keyed-publish entry point. Codegen
 * routes `Topic <- value` here when the topic declares
 * `keyed_by FIELD` (key already extracted from value at the
 * publish call site by GEP-and-load). Mirrors
 * `lotus_bus_dispatch`'s wire-then-local fanout shape; the only
 * extra cost is the runtime per-entry (key_lo, key_hi) compare
 * in the local-dispatch walk. v0.1 only fanouts to intra-
 * process subscribers; remote subscribers receive an unkeyed
 * cross-process publish and filter on their own side after
 * deserialize. */
void lotus_bus_dispatch_wire_keyed(const char *subject,
                                    const void *wire_bytes,
                                    size_t wire_size,
                                    uint64_t key_lo,
                                    uint64_t key_hi);

/* Phase 3 fail policy (2026-05-25): same shape as
 * lotus_bus_dispatch_keyed but returns the match-count signal
 * caller-side codegen can branch on. Returns 1 when at least one
 * specific-key (kind=1) subscriber fired, 0 when none did. The
 * fail-policy `or raise` codegen branch checks this and routes
 * to `lotus_root_panic` on 0; the `or discard` branch ignores
 * the result.
 *
 * `or handler(err)` / `or fail <payload>` (which would need
 * BusUnmatchedKey err payload synthesis) is deferred to v0.2 of
 * the impl. */
int lotus_bus_dispatch_keyed_fallible(lotus_bus_queue_t *queue,
                                       const char *subject,
                                       const void *struct_payload,
                                       size_t struct_size,
                                       lotus_serialize_fn serialize_fn,
                                       uint64_t key_lo,
                                       uint64_t key_hi);

/* Forward decl so the fallible variant's body can delegate to
 * the unkeyed-only-dispatch path without C complaining about
 * implicit declaration. Body lives below. */
void lotus_bus_dispatch_keyed(lotus_bus_queue_t *queue,
                              const char *subject,
                              const void *struct_payload,
                              size_t struct_size,
                              lotus_serialize_fn serialize_fn,
                              uint64_t key_lo,
                              uint64_t key_hi);

int lotus_bus_dispatch_keyed_fallible(lotus_bus_queue_t *queue,
                                       const char *subject,
                                       const void *struct_payload,
                                       size_t struct_size,
                                       lotus_serialize_fn serialize_fn,
                                       uint64_t key_lo,
                                       uint64_t key_hi) {
    /* Pre-walk for the match signal. Same logic as the dispatch
     * itself but without the actual fire — we just need to know
     * "would anyone fire?" so the caller can route the no-match
     * branch. The dispatch then proceeds normally below. */
    int matched = 0;
    for (size_t i = 0; i < g_bus_count; i++) {
        lotus_bus_entry_t *e = &g_bus_entries[i];
        if (!e->subject) continue;
        if (!lotus_subject_match(e->subject, subject)) continue;
        if (e->key_filter_kind == 1
            && e->key_lo == key_lo
            && e->key_hi == key_hi) {
            matched = 1;
            break;
        }
    }
    lotus_bus_dispatch_keyed(queue, subject, struct_payload,
                              struct_size, serialize_fn, key_lo, key_hi);
    return matched;
}

void lotus_bus_dispatch_keyed(lotus_bus_queue_t *queue,
                              const char *subject,
                              const void *struct_payload,
                              size_t struct_size,
                              lotus_serialize_fn serialize_fn,
                              uint64_t key_lo,
                              uint64_t key_hi) {
    if (serialize_fn) {
        char *wire_buf = g_tls_bus_wire_buf;   /* off the coro stack */
        ssize_t wire_size = serialize_fn(struct_payload, wire_buf,
                                         LOTUS_PAYLOAD_MAX);
        if (wire_size > 0) {
            lotus_bus_dispatch_wire_keyed(
                subject, wire_buf, (size_t)wire_size, key_lo, key_hi);
            if (lotus_bus_has_remote_entries()) {
                /* Remote fanout: v0.1 sends the wire bytes
                 * unkeyed to remote subscribers; remote-side bus
                 * routers filter on their end (route metadata is
                 * carried in the payload's keyed_by field, so
                 * the remote bus has what it needs to dispatch). */
                lotus_bus_remote_fanout(subject, wire_buf,
                                         (size_t)wire_size);
            }
            return;
        }
        return;
    }
    /* No serialize codec: dispatch verbatim with key filter. */
    lotus_bus_local_dispatch_keyed(
        queue, subject, struct_payload, struct_size, key_lo, key_hi);
}

void lotus_bus_dispatch(lotus_bus_queue_t *queue,
                        const char *subject,
                        const void *struct_payload,
                        size_t struct_size,
                        lotus_serialize_fn serialize_fn) {
    /* Phase-3 Task 11 (2026-05-20): per-subscriber arena routing
     * for the intra-process path. Previously this enqueued the
     * publisher's struct bytes verbatim into each subscriber's
     * cell; String / Bytes pointers stayed aliased to the
     * publisher's arena. For long-running publishers (a high-rate
     * normalizer class) that's an unbounded leak — the
     * publisher's arena accumulates per-publish allocations
     * forever, with no cap (the global-arena cap doesn't apply
     * to locus-owned arenas).
     *
     * The fix: when a serialize_fn is available, route through
     * the wire-format path (Task 9). The serialize/deserialize
     * round-trip rebuilds the struct in each subscriber's own
     * arena via the TLS routing, so payload pointers end up
     * bounded by the subscriber's lifecycle. The cost is one
     * serialize + N deserializes per publish (N = matching
     * subscribers); roughly 2x the previous cooperative-only
     * dispatch cost in the typical 1-sub case. Trade-off:
     * correctness (no unbounded leak) vs. throughput.
     *
     * When `serialize_fn` is NULL — a payload-typeless
     * subject the codegen didn't synthesize a wire codec for —
     * fall back to the legacy verbatim path. Payload pointers
     * stay aliased to publisher's arena; subscribers retain
     * the dangling-by-design behavior the pre-Task-11 v1
     * shipped with. */
    if (serialize_fn) {
        char *wire_buf = g_tls_bus_wire_buf;   /* off the coro stack */
        ssize_t wire_size = serialize_fn(struct_payload, wire_buf,
                                         LOTUS_PAYLOAD_MAX);
        if (wire_size > 0) {
            /* Local fanout via per-sub deserialize-into-sub-arena.
             * Reuses the wire-dispatch path's TLS routing
             * machinery. */
            lotus_bus_dispatch_wire(subject, wire_buf, (size_t)wire_size);
            /* Remote fanout: send the already-serialized wire
             * bytes to each CONNECT-role transport bound to
             * this subject. The serialize cost is amortized
             * across local + remote (was previously paid only
             * for remote). */
            if (lotus_bus_has_remote_entries()) {
                lotus_bus_remote_fanout(subject, wire_buf,
                                         (size_t)wire_size);
            }
            return;
        }
        /* serialize failure → drop the publish. The cooperative
         * surface treats this as a no-op (matches the prior
         * remote-only failure mode). */
        if (lotus_bus_log_drop_enabled()) {
            fprintf(stderr,
                    "[bus] publish dropped: serialize_fn returned %zd "
                    "for subject=\"%s\" (struct_size=%zu, "
                    "buf_cap=%d)\n",
                    wire_size, subject, struct_size, LOTUS_PAYLOAD_MAX);
        }
        return;
    }
    /* Legacy verbatim local fanout (no wire codec). */
    lotus_bus_local_dispatch(queue, subject, struct_payload, struct_size);
}

/* m41b semantic: null-out subject for any entry whose self
 * matches `self_ptr`. Subsequent `lotus_bus_dispatch` calls skip
 * those slots — quarantined subscribers stop receiving messages. */
void lotus_bus_quarantine_self(void *self_ptr) {
    for (size_t i = 0; i < g_bus_count; i++) {
        if (g_bus_entries[i].self_ptr == self_ptr) {
            g_bus_entries[i].subject = NULL;
        }
    }
}

void lotus_bus_router_destroy(void) {
    if (g_bus_entries) free(g_bus_entries);
    g_bus_entries = NULL;
    g_bus_count   = 0;
    g_bus_cap     = 0;
    /* m58: also tear down any remote-bound transports the
     * deployment-config loader opened at boot. */
    lotus_bus_remote_destroy_all();
}

/*
 * Pinned-thread CPU affinity helper (m28c).
 *
 * `: schedule pinned(core=N)` annotations route through here:
 * codegen emits a call to lotus_set_core_affinity right after
 * pthread_create succeeds, with the user-declared core index.
 * We wrap pthread_setaffinity_np behind a stable C helper so
 * codegen doesn't have to construct a cpu_set_t directly
 * (cpu_set_t is opaque + size-variable across glibc versions).
 *
 * If the affinity call fails (e.g., core index out of range,
 * permission denied in restricted environments) we silently
 * succeed — the thread runs without affinity, falling back to
 * normal OS scheduling. v0 prefers "best effort" over hard-
 * error here so a CI box with fewer cores than the source
 * declares doesn't refuse to start the binary.
 */
void lotus_set_core_affinity(unsigned long tid, int core) {
    cpu_set_t cpuset;
    CPU_ZERO(&cpuset);
    CPU_SET(core, &cpuset);
    (void)pthread_setaffinity_np(
        (pthread_t)tid, sizeof(cpu_set_t), &cpuset);
}

/*
 * Pinned-thread entry (m28a + m28b).
 *
 * The C-runtime adapter `lotus_thread_entry` is gone — m28a
 * synthesizes a per-locus `__pinned_main_<LocusName>` LLVM
 * function whose signature is exactly pthread's `void *(*)(void *)`.
 * That function takes self_ptr as its sole argument and runs
 * birth → run → (mailbox loop) → drain → dissolve in sequence
 * (each only if the locus declared it) before returning NULL.
 * The mailbox loop is included only when the pinned locus
 * declares `bus subscribe`; the codegen branches on that at
 * compile time (m28b).
 */

/*
 * String helpers (m36).
 *
 * Strings in the codegen are NUL-terminated byte arrays. A
 * literal lives as a module-global; a concat / slice result
 * lives in an arena, owned by the caller's locus. All string
 * ops preserve the "value is a pointer" shape — Codegen's
 * CodegenTy::String maps to a basic ptr_t at the LLVM level
 * regardless of provenance.
 *
 * Lifetimes follow the spec/memory.md region rule: results land
 * in whatever arena the caller passes (their current locus's
 * arena, or the program-wide arena in `main` and free fns).
 * No per-string free; the arena's wholesale destroy reclaims
 * everything together.
 */
char *lotus_str_concat(lotus_arena_t *a, const char *l, const char *r) {
    size_t lL = strlen(l);
    size_t lR = strlen(r);
    char *out = (char *)lotus_arena_alloc(a, lL + lR + 1, 1);
    if (!out) return NULL;
    memcpy(out, l, lL);
    memcpy(out + lL, r, lR);
    out[lL + lR] = '\0';
    return out;
}

int lotus_str_eq(const char *l, const char *r) {
    return strcmp(l, r) == 0 ? 1 : 0;
}

/* m49: deep-copy a string into the destination arena. Used at
 * free-fn return boundaries: the body's subregion is about to be
 * destroyed, so any String the body returns gets cloned into the
 * caller's arena first. The returned pointer outlives the
 * subregion destroy. Same shape as concat with a NULL right side
 * — kept as a separate symbol so the call-site IR is one helper
 * call, not a concat-with-empty-literal dance. */
/* 2026-05-21: clone-skip optimizations for lotus_str_clone /
 * lotus_bytes_clone. Two cases pass through without allocating:
 *
 *  (a) Static-literal skip — src is in the binary's .rodata /
 *      initialized-data range. String literals (`"foo"`) lower
 *      to globals in .rodata; cloning them is wasted because
 *      the original pointer is already program-lifetime. Bounded
 *      by linker-provided `__executable_start` and `_edata`
 *      (glibc exports both on every Linux build).
 *
 *  (b) Same-arena skip — src is already inside one of dest's
 *      chunks. This catches the dominant pond/metrics pattern
 *      where a Counter / Gauge's `.inc()` / `.set()` reads
 *      `e = store.get(self.key)` and writes back
 *      `store.set(MetricEntry { key: e.key, name: e.name, ... })`.
 *      `e.key` and `e.name` were cloned into the store's arena
 *      on the original insert; re-cloning them into the same
 *      arena on every update wastes O(N) bytes per call. Each
 *      arena typically holds 1-10 chunks, so the walk is cheap
 *      (single-digit ns at typical chunk counts). Long-running
 *      arenas with hundreds of chunks pay more — sortable
 *      chunk lookup is a future optimization if a workload
 *      surfaces the cost.
 *
 * Both skips return src unchanged. The hashmap.set / vec.push /
 * locus-field-init deep-copy paths use this for their String
 * field clones; the savings compound when the same metric is
 * updated thousands of times per second. */
#if defined(__GLIBC__)
extern char __executable_start[];
extern char _edata[];
static inline int lotus_str_is_static_literal(const char *s) {
    return s >= __executable_start && s < _edata;
}
#else
static inline int lotus_str_is_static_literal(const char *s) {
    (void)s;
    return 0;
}
#endif

static int lotus_ptr_in_arena(const lotus_arena_t *a, const void *p) {
    if (!a || !p) return 0;
    const char *cp = (const char *)p;
    for (const lotus_arena_chunk_t *c = a->head; c; c = c->next) {
        const char *base = (const char *)(c + 1);
        if (cp >= base && cp < base + c->cap) return 1;
    }
    return 0;
}

/* Public surface for the codegen's same-arena skip at cross-arena
 * store boundaries (hashmap.set, vec.set, vec.push,
 * ring_buffer.push). The inner str/bytes clone helpers still use
 * the static inline above for tighter codegen on the hot
 * scalar-field paths; this wrapper exists so the LLVM-side
 * outer-struct skip can call into the same arena walk without
 * exposing the chunk struct layout. Returns 1 if `p` is inside
 * one of `a`'s chunk data regions, 0 otherwise. O(chunks) — at
 * steady-state arenas usually hold 1-10 chunks so the cost is
 * single-digit ns; long-running arenas with hundreds of chunks
 * pay more (a sortable chunk index is a future optimization if
 * a workload surfaces the walk as a hotspot). */
int lotus_arena_contains_ptr(const lotus_arena_t *a, const void *p) {
    return lotus_ptr_in_arena(a, p);
}

char *lotus_str_clone(lotus_arena_t *a, const char *s) {
    if (lotus_str_is_static_literal(s)) {
        return (char *)s;
    }
    if (lotus_ptr_in_arena(a, s)) {
        return (char *)s;
    }
    size_t n = strlen(s);
    char *out = (char *)lotus_arena_alloc(a, n + 1, 1);
    if (!out) return NULL;
    memcpy(out, s, n);
    out[n] = '\0';
    return out;
}

/* 2026-05-22 PM: in-place String reassignment for the
 * `self.X = String_value` field-assign hot path. The motivating
 * leak class (a downstream daemon / SymbolBook): every per-delta
 * `self.last_venue_ts = venue_ts` clones venue_ts into self.__arena
 * via lotus_str_clone — and the OLD self.last_venue_ts bytes are
 * unreachable but unfreeable (arena allocators don't track per-
 * allocation lifetimes). 18 events / 4 min × 64KB pool-chunk
 * granularity = 1.15 MiB / 4 min across 3 books, the entire
 * remaining structural drift after the sret + populate-rework
 * series.
 *
 * Fix: when `old_ptr` is an arena-resident, NUL-terminated String
 * we ourselves allocated, its buffer is exactly `strlen(old) + 1`
 * bytes (lotus_str_clone / lotus_str_slice / lotus_str_concat /
 * etc. all sit on that invariant — `lotus_arena_alloc(a, n + 1,
 * 1)`). So `strlen(old)` is an upper bound on writable capacity.
 * If the incoming String's length fits, memcpy + NUL-terminate
 * directly onto `old`'s buffer and return `old` unchanged. The
 * slot's pointer stays put; no new arena bytes consumed. When the
 * new String is longer, fall back to lotus_str_clone (a fresh
 * allocation in `a`); the old buffer leaks per the structural
 * arena limitation, but the rate is bounded by how often the
 * field genuinely grows in length rather than the per-update
 * frequency.
 *
 * Static-literal skip: writing into a .rodata pointer would
 * segfault. Detect via the same __executable_start / _edata
 * boundary used elsewhere; fall back to clone. This also covers
 * the locus-init default case (slot starts pointing at the empty-
 * literal sentinel).
 *
 * Same-arena passthrough: when `new` is already in `a`, the
 * caller has nothing to clone — but the old buffer would leak if
 * we just returned `new`. Two cases:
 *   - `new == old` (rebinding to itself): return old, no-op.
 *   - `new != old` but both in a: copy new's content into old's
 *     buffer if it fits (reuses old's slot), else fall back to
 *     new (old leaks). Net: same number of bytes live as before. */
char *lotus_str_assign_in_place(lotus_arena_t *a, char *old,
                                 const char *new_s) {
    if (!new_s) return NULL;
    if (!old) return lotus_str_clone(a, new_s);
    if (old == new_s) return old;
    if (lotus_str_is_static_literal(old)) {
        return lotus_str_clone(a, new_s);
    }
    /* old is an arena-owned NUL-terminated buffer. strlen(old)
     * is its capacity (sans NUL). */
    size_t old_len = strlen(old);
    size_t new_len = strlen(new_s);
    if (new_len <= old_len) {
        memcpy(old, new_s, new_len);
        old[new_len] = '\0';
        return old;
    }
    /* New is longer than old's buffer — clone. */
    return lotus_str_clone(a, new_s);
}

/* lotus_bytes_clone is defined further down (alongside the
 * other Bytes helpers) so the forward references to
 * lotus_bytes_create / lotus_bytes_len / lotus_bytes_data
 * resolve cleanly. */

int64_t lotus_str_len(const char *s) {
    return (int64_t)strlen(s);
}

/* 2026-05-26 — direct byte-access on a String at offset `i`,
 * returning the byte value (0..255) or -1 on out-of-range.
 * Symmetric with lotus_bytes_at but skips the
 * std::bytes::from_string(s) trip, which allocates a fresh
 * Bytes copy of the entire String every call. Used by stdlib
 * scan helpers (JSON walkers, etc.) that peek at single bytes
 * inside a large source String. The strlen call dominates cost
 * for large inputs — for tight scan loops use
 * lotus_str_byte_at_unchecked instead. */
int64_t lotus_str_byte_at(const char *s, int64_t i) {
    if (!s || i < 0) return -1;
    int64_t n = (int64_t)strlen(s);
    if (i >= n) return -1;
    return (int64_t)(unsigned char)s[i];
}

/* Unchecked byte access — no strlen, no bounds check. Caller
 * MUST guarantee 0 <= i < strlen(s); the typical pattern is
 * computing the bound once (via len(json) at the scanner's
 * entry point) and iterating `while p < bound`. Used by the
 * JSON range-walker scanners where the bound is already known
 * from len(json) computed at function entry. The point of this
 * variant is to avoid a per-byte-access strlen — for a 5 MB
 * source string scanned 100k times, an O(N) strlen per call
 * compounds to seconds of pure-CPU; the unchecked variant is
 * a single load. Misuse (i out of range) is UB. */
int64_t lotus_str_byte_at_unchecked(const char *s, int64_t i) {
    return (int64_t)(unsigned char)s[i];
}

/*
 * Substring `s[lo..hi]` with exclusive `hi`. Bounds clamp so
 * out-of-range indices produce a (possibly empty) string rather
 * than crashing — matches the interpreter and avoids a forced
 * runtime panic for off-by-one mistakes. Result is a fresh
 * arena-owned NUL-terminated copy.
 */
char *lotus_str_slice(lotus_arena_t *a, const char *s,
                      int64_t lo, int64_t hi) {
    int64_t n = (int64_t)strlen(s);
    if (lo < 0) lo = 0;
    if (lo > n) lo = n;
    if (hi < lo) hi = lo;
    if (hi > n) hi = n;
    int64_t len = hi - lo;
    char *out = (char *)lotus_arena_alloc(a, (size_t)len + 1, 1);
    if (!out) return NULL;
    if (len > 0) {
        memcpy(out, s + lo, (size_t)len);
    }
    out[len] = '\0';
    return out;
}

/*
 * to_string helpers (m37). Each renders one primitive into a
 * fresh NUL-terminated arena buffer using the same printf-style
 * format that `println` uses, so a value written via to_string
 * + concat reads identical to the same value passed to println.
 *
 * Buffer sizes:
 *   - i64  → max 20 digits + sign + NUL = 22 bytes; round up.
 *   - %g   → typical max ~24 chars for normal magnitudes; 32
 *     covers headroom for denormals and -DBL_MAX.
 *   - duration → i64 + "ns" suffix.
 */
char *lotus_str_from_int(lotus_arena_t *a, int64_t n) {
    char *out = (char *)lotus_arena_alloc(a, 32, 1);
    if (!out) return NULL;
    snprintf(out, 32, "%lld", (long long)n);
    return out;
}

char *lotus_str_from_float(lotus_arena_t *a, double f) {
    char *out = (char *)lotus_arena_alloc(a, 32, 1);
    if (!out) return NULL;
    snprintf(out, 32, "%g", f);
    return out;
}

char *lotus_str_from_duration(lotus_arena_t *a, int64_t ns) {
    char *out = (char *)lotus_arena_alloc(a, 32, 1);
    if (!out) return NULL;
    snprintf(out, 32, "%lldns", (long long)ns);
    return out;
}

/*
 * starts_with / contains (m38).
 *
 * Both return i32 0/1 (codegen truncates to i1). Empty
 * prefix / sub matches any string (matches Rust semantics).
 * No locale folding — byte-exact comparison so the result
 * doesn't drift across systems.
 */
int lotus_str_starts_with(const char *s, const char *prefix) {
    if (!s || !prefix) return 0;
    size_t lp = strlen(prefix);
    if (lp == 0) return 1;
    return strncmp(s, prefix, lp) == 0 ? 1 : 0;
}

int lotus_str_contains(const char *s, const char *sub) {
    if (!s || !sub) return 0;
    if (*sub == '\0') return 1;
    return strstr(s, sub) ? 1 : 0;
}

/*
 * m84: byte index of first occurrence of `sub` in `s`, or -1 if
 * not found. Mirrors lotus_str_contains's strstr-based search but
 * returns the position rather than just a presence flag — needed
 * by Phase 3's HTTP request parser (locating ` ` between method
 * and path, `\r\n` at the end of the request line, etc.). Empty
 * needle returns 0 by convention; null inputs return -1.
 */
/*
 * m89: Bytes value primitives.
 *
 * A Bytes value is a single arena-allocated pointer to a blob
 * laid out as `[i64 len][u8 data[len]]`. The leading length
 * makes the value self-describing — same single-pointer ABI
 * as String, but binary content with embedded NUL bytes
 * doesn't truncate (NUL is not a terminator here).
 *
 * Memory: allocated via lotus_arena_alloc on the caller's
 * arena, so the lifetime matches the locus or fn whose arena
 * it came from. v0 has no resize/append — Bytes is created
 * once with a known length (via read, recv, etc.) and lives
 * as long as the caller's arena does.
 */
void *lotus_bytes_create(lotus_arena_t *a, int64_t len) {
    if (len < 0) {
        return NULL;
    }
    /* sizeof(int64_t) for the prefix + len bytes for the body. */
    size_t blob = sizeof(int64_t) + (size_t)len;
    void *p = lotus_arena_alloc(a, blob, 8);
    if (!p) {
        return NULL;
    }
    *(int64_t *)p = len;
    /* Body bytes left uninitialized — caller fills them via
     * lotus_bytes_data(). Zeroing here would double the cost
     * for callers that overwrite the whole blob immediately
     * (the common case: read syscall reads into it, recv
     * fills it, etc.). */
    return p;
}

int64_t lotus_bytes_len(const void *b) {
    if (!b) return 0;
    return *(const int64_t *)b;
}

void *lotus_bytes_data(void *b) {
    if (!b) return NULL;
    return (char *)b + sizeof(int64_t);
}

/* B2 / G5 bytes-literal helper: allocate a Bytes blob in `a` and
 * copy `len` bytes from `src` into it. Used by codegen to lower
 * `b"..."` literals without a per-literal dance of create +
 * memcpy at the IR level. `src` may be NULL when `len == 0`. */
void *lotus_bytes_from_buf(lotus_arena_t *a, const void *src, int64_t len) {
    void *blob = lotus_bytes_create(a, len);
    if (!blob || len <= 0) {
        return blob;
    }
    memcpy(lotus_bytes_data(blob), src, (size_t)len);
    return blob;
}

/* F.30 (2026-05-20): deep-copy a Bytes blob (length-prefixed,
 * may contain embedded NULs) into `a`. The companion to
 * `lotus_str_clone` for the binary path; needed for
 * `std::bytes::clone(view)` to upgrade a non-owning BytesView
 * into an owned arena-backed Bytes blob.
 *
 * Returns NULL on alloc failure; the caller (the Hale-side
 * `std::bytes::clone` lowering) wraps NULL in the empty-bytes
 * sentinel via the existing patterns. */
void *lotus_bytes_clone(lotus_arena_t *a, const void *src) {
    if (!a || !src) return NULL;
    /* Same clone-skip optimizations as lotus_str_clone — static-
     * literal Bytes (b"..." globals) and same-arena clones pass
     * through without re-allocating. Bytes carries a length
     * prefix, so the in-arena check uses the prefix address
     * (start of the blob), not the payload. */
    if (lotus_str_is_static_literal((const char *)src)) {
        return (void *)src;
    }
    if (lotus_ptr_in_arena(a, src)) {
        return (void *)src;
    }
    int64_t len = lotus_bytes_len(src);
    if (len < 0) len = 0;
    void *blob = lotus_bytes_create(a, len);
    if (!blob) return NULL;
    if (len > 0) {
        memcpy(
            lotus_bytes_data(blob),
            (const char *)src + sizeof(int64_t),
            (size_t)len);
    }
    return blob;
}

/* 2026-05-22 PM: in-place Bytes reassignment, the Bytes companion
 * to lotus_str_assign_in_place. Same shape: at `self.X = Bytes_value`
 * sites, reuse the existing slot's buffer when the new payload fits
 * inside it. Closes the gotcha-#5-Bytes-companion case so the
 * substrate's "self.X = heap_value" leak class is symmetric across
 * String and Bytes.
 *
 * Bytes layout is `[int64_t len][len bytes payload]`; the buffer's
 * physical size is `sizeof(int64_t) + len`. We use the prefix value
 * as both the logical length AND the available capacity (no
 * separate capacity field in the v0 representation). After an
 * in-place reduce, the prefix is updated to `new_len`; subsequent
 * assigns compare against the (now-smaller) prefix, so a field
 * whose values oscillate up and down across hot-path calls will
 * degrade toward "always clone" as the prefix shrinks. For bounded-
 * variance fields (the typical case — fixed-size frame headers,
 * checksums, etc.) the prefix stays constant and the in-place
 * path holds. Spec callout in spec/memory.md Phase-4 perf follow-
 * on #7.
 *
 * Static-literal skip + null-old skip + same-pointer skip mirror
 * lotus_str_assign_in_place's logic. */
void *lotus_bytes_assign_in_place(lotus_arena_t *a, void *old,
                                   const void *new_b) {
    if (!new_b) return NULL;
    if (!old) return lotus_bytes_clone(a, new_b);
    if (old == new_b) return old;
    if (lotus_str_is_static_literal((const char *)old)) {
        return lotus_bytes_clone(a, new_b);
    }
    int64_t old_cap = lotus_bytes_len(old);
    int64_t new_len = lotus_bytes_len(new_b);
    if (new_len < 0) new_len = 0;
    if (old_cap < 0) old_cap = 0;
    if (new_len <= old_cap) {
        *(int64_t *)old = new_len;
        if (new_len > 0) {
            memcpy(
                (char *)old + sizeof(int64_t),
                (const char *)new_b + sizeof(int64_t),
                (size_t)new_len);
        }
        return old;
    }
    return lotus_bytes_clone(a, new_b);
}

int64_t lotus_str_index_of(const char *s, const char *sub) {
    if (!s || !sub) return -1;
    if (*sub == '\0') return 0;
    const char *hit = strstr(s, sub);
    if (!hit) return -1;
    return (int64_t)(hit - s);
}

/*
 * m48: render a Decimal value (i128 mantissa with implicit
 * scale 9 — i.e., mantissa × 10^-9) into a NUL-terminated
 * string. The i128 is passed as two i64 halves (hi:lo) since
 * the LLVM/C ABI for __int128 is awkward to wire; codegen
 * splits the value before the call.
 *
 * Output format trims trailing zeros + dangling decimal point,
 * matching the interpreter's DecimalVal::display so both
 * backends print identically. Caller passes a buffer of at
 * least LOTUS_DECIMAL_BUF_LEN bytes.
 */
#define LOTUS_DECIMAL_BUF_LEN 64

/* Helper used internally — exposed forward-decl form so the
 * arena-allocating sibling can call it. */
void lotus_decimal_to_string(int64_t hi, uint64_t lo, char *buf);

/*
 * Variant of lotus_decimal_to_string that allocates the buffer
 * inside the caller's arena and returns a pointer to it.
 * Mirrors lotus_str_from_float for the Float case.
 */
char *lotus_str_from_decimal(lotus_arena_t *a, int64_t hi, uint64_t lo) {
    char *out = (char *)lotus_arena_alloc(a, LOTUS_DECIMAL_BUF_LEN, 1);
    if (!out) return NULL;
    lotus_decimal_to_string(hi, lo, out);
    return out;
}

/* std::decimal::to_float (2026-05-21): direct i128 → f64
 * conversion at scale 9 (`mantissa × 10^-9`). Replaces the
 * `to_string(d)` → strip → `parse_float` ASCII round-trip
 * downstream consumers were doing for Decimal → Float in
 * hot paths. The C cast from __int128 to double is one
 * compiler intrinsic; the / 1e9 applies the implicit scale.
 * Loss-of-precision on very large mantissas is the same
 * floor as `(double)i64`, just over a wider range. */
double lotus_decimal_to_float(int64_t hi, uint64_t lo) {
    __int128 m = ((__int128)hi << 64) | (__int128)lo;
    return (double)m / 1.0e9;
}

void lotus_decimal_to_string(int64_t hi, uint64_t lo, char *buf) {
    __int128 m = ((__int128)hi << 64) | (__int128)lo;
    int neg = m < 0;
    unsigned __int128 abs = neg ? (unsigned __int128)(-m) : (unsigned __int128)m;
    unsigned __int128 pow9 = 1000000000ULL;
    unsigned __int128 int_part = abs / pow9;
    unsigned __int128 frac_part = abs % pow9;
    char *p = buf;
    if (neg) {
        *p++ = '-';
    }
    /* int_part may exceed 64 bits when the mantissa's integer
     * part is over 10^19. The simple fast path covers the
     * common case; the fallback decomposes into 10^18 chunks. */
    if ((int_part >> 64) == 0) {
        p += snprintf(p, 32, "%llu", (unsigned long long)int_part);
    } else {
        unsigned __int128 base = 1000000000000000000ULL;
        unsigned __int128 hi_part = int_part / base;
        unsigned __int128 lo_part = int_part % base;
        p += snprintf(p, 48, "%llu%018llu",
            (unsigned long long)hi_part,
            (unsigned long long)lo_part);
    }
    if (frac_part != 0) {
        char fb[16];
        snprintf(fb, sizeof(fb), "%09llu", (unsigned long long)frac_part);
        size_t end = strlen(fb);
        while (end > 0 && fb[end - 1] == '0') {
            end--;
        }
        if (end > 0) {
            *p++ = '.';
            memcpy(p, fb, end);
            p += end;
        }
    }
    *p = '\0';
}

/*
 * m57: AF_UNIX transport for the cross-process bus.
 *
 * First substrate piece of the cross-process bus arc. Provides a
 * minimal "raw bytes between two processes over a unix socket"
 * surface: create a transport in listener or connector role, send
 * one message, recv one message, destroy. SOCK_SEQPACKET preserves
 * message boundaries so each lotus_transport_send shows up as
 * exactly one lotus_transport_recv — matches bus cell semantics
 * with no framing layer at this milestone.
 *
 * No protocol, no subject binding, no deployment-config: this is
 * the kernel-level transport substrate. m58 wires deployment-config
 * subject -> transport URL routing on top of these primitives;
 * m59 adds per-payload serializers; m60 weaves multi-binary builds
 * + fitter-applier-pair end-to-end. Source-level lotus stays
 * transport-agnostic per notes/open-questions #8.
 *
 * Lifecycle:
 *   - LISTEN role: bind + listen + accept. Blocks
 *     lotus_transport_create until exactly one connector connects.
 *   - CONNECT role: connect with retry-on-ENOENT/ECONNREFUSED for
 *     ~1s, then fail. Lets the connector start before the listener
 *     races to bind without needing an external sync point.
 *
 * Errors return NULL (create) or -1 (send/recv) and write a
 * perror-style message to stderr. v0.1 prefers fail-fast over
 * recovery — the protocol layer above this re-creates on failure.
 */

#define LOTUS_TRANSPORT_LISTEN  0
#define LOTUS_TRANSPORT_CONNECT 1

typedef struct lotus_transport {
    int   conn_fd;        /* duplex SEQPACKET fd carrying messages */
    int   listen_fd;      /* listener role only; -1 for connector */
    char *path;           /* listener role only; owned, unlinked on destroy */
    int   role;
} lotus_transport_t;

static int lotus__transport_set_addr(struct sockaddr_un *addr,
                                     const char *path) {
    size_t len = strlen(path);
    /* sun_path includes the NUL — reject anything that would not fit. */
    if (len + 1 > sizeof(addr->sun_path)) {
        errno = ENAMETOOLONG;
        return -1;
    }
    memset(addr, 0, sizeof(*addr));
    addr->sun_family = AF_UNIX;
    memcpy(addr->sun_path, path, len + 1);
    return 0;
}

lotus_transport_t *lotus_transport_create(const char *path, int role) {
    if (!path) {
        errno = EINVAL;
        return NULL;
    }
    struct sockaddr_un addr;
    if (lotus__transport_set_addr(&addr, path) != 0) {
        perror("lotus_transport_create: addr");
        return NULL;
    }

    int sock = socket(AF_UNIX, SOCK_SEQPACKET, 0);
    if (sock < 0) {
        perror("lotus_transport_create: socket");
        return NULL;
    }

    if (role == LOTUS_TRANSPORT_LISTEN) {
        /* Best-effort: clear any stale socket file so bind succeeds
         * after a previous run was killed without destroy(). */
        unlink(path);
        if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
            perror("lotus_transport_create: bind");
            close(sock);
            return NULL;
        }
        if (listen(sock, 1) < 0) {
            perror("lotus_transport_create: listen");
            close(sock);
            unlink(path);
            return NULL;
        }
        int conn = accept(sock, NULL, NULL);
        if (conn < 0) {
            perror("lotus_transport_create: accept");
            close(sock);
            unlink(path);
            return NULL;
        }
        lotus_transport_t *t = (lotus_transport_t *)calloc(1, sizeof(*t));
        if (!t) {
            close(conn);
            close(sock);
            unlink(path);
            return NULL;
        }
        t->conn_fd   = conn;
        t->listen_fd = sock;
        t->path      = strdup(path);
        t->role      = role;
        return t;
    }

    if (role == LOTUS_TRANSPORT_CONNECT) {
        /* Retry connect on ENOENT/ECONNREFUSED for up to ~1s so a
         * connector that races ahead of the listener's bind/listen
         * still succeeds once the listener becomes ready. */
        struct timespec backoff = { 0, 5L * 1000L * 1000L };  /* 5ms */
        int attempts = 200;                                   /* 200 × 5ms */
        while (attempts-- > 0) {
            if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) == 0) {
                lotus_transport_t *t =
                    (lotus_transport_t *)calloc(1, sizeof(*t));
                if (!t) {
                    close(sock);
                    return NULL;
                }
                t->conn_fd   = sock;
                t->listen_fd = -1;
                t->path      = NULL;
                t->role      = role;
                return t;
            }
            if (errno != ENOENT && errno != ECONNREFUSED) {
                perror("lotus_transport_create: connect");
                close(sock);
                return NULL;
            }
            nanosleep(&backoff, NULL);
        }
        fprintf(stderr,
                "lotus_transport_create: connect to %s timed out\n",
                path);
        close(sock);
        return NULL;
    }

    fprintf(stderr, "lotus_transport_create: invalid role %d\n", role);
    close(sock);
    errno = EINVAL;
    return NULL;
}

int lotus_transport_send(lotus_transport_t *t,
                         const void *buf,
                         size_t len) {
    if (!t || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    ssize_t n = send(t->conn_fd, buf, len, 0);
    if (n < 0) {
        perror("lotus_transport_send");
        return -1;
    }
    return 0;
}

ssize_t lotus_transport_recv(lotus_transport_t *t,
                             void *buf,
                             size_t cap) {
    if (!t || (!buf && cap > 0)) {
        errno = EINVAL;
        return -1;
    }
    ssize_t n = recv(t->conn_fd, buf, cap, 0);
    if (n < 0) {
        perror("lotus_transport_recv");
        return -1;
    }
    return n;
}

void lotus_transport_destroy(lotus_transport_t *t) {
    if (!t) return;
    if (t->conn_fd >= 0) close(t->conn_fd);
    if (t->listen_fd >= 0) close(t->listen_fd);
    if (t->path) {
        unlink(t->path);
        free(t->path);
    }
    free(t);
}

/*
 * m72: TCP transport (AF_INET) — sibling adapter to the AF_UNIX
 * SEQPACKET transport above.
 *
 * Design context (project_tcp_framing.md): the transport surface
 * contracts to deliver atomic messages — one send produces one
 * recv of the same byte sequence at the other end. SEQPACKET
 * satisfies this via kernel datagram semantics; TCP satisfies it
 * by length-prefix framing inside this adapter. The bus layer
 * above is transport-agnostic and treats every transport as
 * "give me the next whole message." Future transports (TLS, QUIC,
 * shared-memory rings) will each pick whatever internal mechanism
 * satisfies the same atomic-message contract.
 *
 * Wire format per message:
 *   [8-byte little-endian uint64 length] [N bytes payload]
 * The 8-byte LE length matches the m70 per-field serializer's
 * String framing convention.
 *
 * Sanity cap: LOTUS_TCP_MAX_MSG_BYTES rejects pathologically
 * large length headers before any allocation or recv loop runs,
 * preventing a malicious or buggy peer from claiming 2^63 bytes
 * and stalling the receiver.
 *
 * Lifecycle mirrors lotus_transport:
 *   - LISTEN role: socket + SO_REUSEADDR + bind + listen + accept.
 *     Blocks lotus_tcp_create until exactly one connector connects.
 *   - CONNECT role: connect with retry on ECONNREFUSED for ~1s.
 *
 * SO_REUSEADDR is set on the listener so a freshly-released port
 * (very recent test runs, dev iteration) doesn't trip TIME_WAIT.
 * TCP_NODELAY is set on the connection so single small messages
 * aren't held by Nagle's algorithm — the bus's typical workload
 * is request/response-shaped where latency matters more than
 * coalescing.
 *
 * Errors return NULL (create) or -1 (send/recv); recv also
 * returns -1 if the framed length exceeds `cap` (caller's buffer
 * too small) or LOTUS_TCP_MAX_MSG_BYTES (cap regardless).
 */

#define LOTUS_TCP_LISTEN  0
#define LOTUS_TCP_CONNECT 1

/* 8 MB ceiling. Generous for typed bus payloads while still
 * making a malicious 2^63 length header an immediate -1. */
#define LOTUS_TCP_MAX_MSG_BYTES (8u * 1024u * 1024u)

typedef struct lotus_tcp {
    int   conn_fd;     /* the connected stream socket */
    int   listen_fd;   /* listener role only; -1 for connector */
    int   role;
    uint16_t port;     /* the actual bound/connected port (esp. when listen requested 0) */
} lotus_tcp_t;

/* Read exactly `n` bytes into `buf` from `fd`, looping over short
 * reads. Returns 0 on success, -1 on error or EOF before n bytes.
 * Used by recv to reassemble the framed message — TCP is a byte
 * stream, so a single read may return any prefix of the requested
 * count. */
static int lotus__tcp_read_full(int fd, void *buf, size_t n) {
    char  *p = (char *)buf;
    size_t left = n;
    while (left > 0) {
        ssize_t r = read(fd, p, left);
        if (r > 0) {
            p    += (size_t)r;
            left -= (size_t)r;
            continue;
        }
        if (r == 0) {
            /* peer closed mid-message — surface as EIO so the
             * caller sees a non-zero errno. */
            errno = EIO;
            return -1;
        }
        if (errno == EINTR) continue;
        return -1;
    }
    return 0;
}

/* Write exactly `n` bytes from `buf` to `fd`, looping over short
 * writes. Mirrors lotus__tcp_read_full. */
static int lotus__tcp_write_full(int fd, const void *buf, size_t n) {
    const char *p = (const char *)buf;
    size_t      left = n;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        /* w == 0 on a regular fd is unusual; treat as error. */
        return -1;
    }
    return 0;
}

/* Encode a host-order uint64 as 8 little-endian bytes. */
static void lotus__u64_to_le(uint64_t v, unsigned char out[8]) {
    for (int i = 0; i < 8; i++) {
        out[i] = (unsigned char)(v >> (i * 8));
    }
}

/* Decode 8 little-endian bytes to a host-order uint64. */
static uint64_t lotus__u64_from_le(const unsigned char in[8]) {
    uint64_t v = 0;
    for (int i = 0; i < 8; i++) {
        v |= ((uint64_t)in[i]) << (i * 8);
    }
    return v;
}

lotus_tcp_t *lotus_tcp_create(const char *host, uint16_t port, int role) {
    /* host=NULL is allowed for both roles: listener interprets as
     * INADDR_ANY (bind-on-any-interface); connector defaults to
     * 127.0.0.1 since "no peer specified" means same-host. */
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    if (role == LOTUS_TCP_LISTEN) {
        if (!host) {
            addr.sin_addr.s_addr = htonl(INADDR_ANY);
        } else if (inet_pton(AF_INET, host, &addr.sin_addr) != 1) {
            fprintf(stderr,
                    "lotus_tcp_create: invalid listen host %s\n", host);
            errno = EINVAL;
            return NULL;
        }
    } else if (role == LOTUS_TCP_CONNECT) {
        const char *h = host ? host : "127.0.0.1";
        if (inet_pton(AF_INET, h, &addr.sin_addr) != 1) {
            fprintf(stderr,
                    "lotus_tcp_create: invalid connect host %s\n", h);
            errno = EINVAL;
            return NULL;
        }
    } else {
        fprintf(stderr, "lotus_tcp_create: invalid role %d\n", role);
        errno = EINVAL;
        return NULL;
    }

    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        perror("lotus_tcp_create: socket");
        return NULL;
    }

    if (role == LOTUS_TCP_LISTEN) {
        int one = 1;
        if (setsockopt(sock, SOL_SOCKET, SO_REUSEADDR,
                       &one, sizeof(one)) < 0) {
            perror("lotus_tcp_create: SO_REUSEADDR");
            close(sock);
            return NULL;
        }
        if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
            perror("lotus_tcp_create: bind");
            close(sock);
            return NULL;
        }
        /* If port=0 the OS picked one; getsockname tells us which. */
        socklen_t alen = sizeof(addr);
        if (getsockname(sock, (struct sockaddr *)&addr, &alen) < 0) {
            perror("lotus_tcp_create: getsockname");
            close(sock);
            return NULL;
        }
        if (listen(sock, 1) < 0) {
            perror("lotus_tcp_create: listen");
            close(sock);
            return NULL;
        }
        int conn = accept(sock, NULL, NULL);
        if (conn < 0) {
            perror("lotus_tcp_create: accept");
            close(sock);
            return NULL;
        }
        int nodelay = 1;
        (void)setsockopt(conn, IPPROTO_TCP, TCP_NODELAY,
                         &nodelay, sizeof(nodelay));
        lotus_tcp_t *t = (lotus_tcp_t *)calloc(1, sizeof(*t));
        if (!t) {
            close(conn);
            close(sock);
            return NULL;
        }
        t->conn_fd   = conn;
        t->listen_fd = sock;
        t->role      = role;
        t->port      = ntohs(addr.sin_port);
        return t;
    }

    /* CONNECT: retry on ECONNREFUSED for ~1s so a connector that
     * races ahead of the listener's bind/listen still succeeds
     * once the listener becomes ready. Mirrors the unix-socket
     * adapter. */
    struct timespec backoff = { 0, 5L * 1000L * 1000L };  /* 5ms */
    int attempts = 200;                                   /* 200 × 5ms */
    while (attempts-- > 0) {
        if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) == 0) {
            int nodelay = 1;
            (void)setsockopt(sock, IPPROTO_TCP, TCP_NODELAY,
                             &nodelay, sizeof(nodelay));
            lotus_tcp_t *t = (lotus_tcp_t *)calloc(1, sizeof(*t));
            if (!t) {
                close(sock);
                return NULL;
            }
            t->conn_fd   = sock;
            t->listen_fd = -1;
            t->role      = role;
            t->port      = port;
            return t;
        }
        if (errno != ECONNREFUSED && errno != EAGAIN) {
            perror("lotus_tcp_create: connect");
            close(sock);
            return NULL;
        }
        nanosleep(&backoff, NULL);
    }
    fprintf(stderr,
            "lotus_tcp_create: connect to port %u timed out\n",
            (unsigned)port);
    close(sock);
    return NULL;
}

uint16_t lotus_tcp_port(lotus_tcp_t *t) {
    return t ? t->port : 0;
}

int lotus_tcp_send(lotus_tcp_t *t, const void *buf, size_t len) {
    if (!t || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    if ((uint64_t)len > LOTUS_TCP_MAX_MSG_BYTES) {
        errno = EMSGSIZE;
        return -1;
    }
    unsigned char header[8];
    lotus__u64_to_le((uint64_t)len, header);
    if (lotus__tcp_write_full(t->conn_fd, header, sizeof(header)) < 0) {
        perror("lotus_tcp_send: header");
        return -1;
    }
    if (len > 0 && lotus__tcp_write_full(t->conn_fd, buf, len) < 0) {
        perror("lotus_tcp_send: payload");
        return -1;
    }
    return 0;
}

ssize_t lotus_tcp_recv(lotus_tcp_t *t, void *buf, size_t cap) {
    if (!t || (!buf && cap > 0)) {
        errno = EINVAL;
        return -1;
    }
    unsigned char header[8];
    if (lotus__tcp_read_full(t->conn_fd, header, sizeof(header)) < 0) {
        /* don't perror on the common EOF case — the caller knows
         * a -1 here means "stream ended or read error"; spammy
         * stderr would obscure the actual program output. */
        return -1;
    }
    uint64_t len = lotus__u64_from_le(header);
    if (len > LOTUS_TCP_MAX_MSG_BYTES) {
        fprintf(stderr,
                "lotus_tcp_recv: framed length %llu exceeds %u\n",
                (unsigned long long)len, LOTUS_TCP_MAX_MSG_BYTES);
        errno = EMSGSIZE;
        return -1;
    }
    if (len > (uint64_t)cap) {
        fprintf(stderr,
                "lotus_tcp_recv: framed length %llu exceeds caller cap %zu\n",
                (unsigned long long)len, cap);
        errno = EMSGSIZE;
        return -1;
    }
    if (len == 0) return 0;
    if (lotus__tcp_read_full(t->conn_fd, buf, (size_t)len) < 0) {
        perror("lotus_tcp_recv: payload");
        return -1;
    }
    return (ssize_t)len;
}

void lotus_tcp_destroy(lotus_tcp_t *t) {
    if (!t) return;
    if (t->conn_fd >= 0) close(t->conn_fd);
    if (t->listen_fd >= 0) close(t->listen_fd);
    free(t);
}

/*
 * m73b: split-shape primitives reachable from Hale source.
 *
 * lotus_tcp_create collapses bind+listen+accept into one
 * blocking call — convenient for the m72 driver tests but wrong
 * for a Listener locus pattern where birth() should not block on
 * an incoming connection. The locus's lifecycle wants:
 *
 *   birth():     bind+listen     -> listen_fd          (non-blocking)
 *   run():       accept (loop)   -> conn_fd per peer   (blocks per accept)
 *   dissolve():  close(listen_fd)
 *
 * These three functions provide that split. Hale source
 * reaches them via the magic `std::io::tcp::__*` path-call
 * primitives wired up in codegen (m73b path-call additions). The
 * `__` prefix is internal-only; the polished user surface is
 * the Listener / Stream loci that wrap these calls in idiomatic
 * lifecycle bodies.
 *
 * fds are returned as plain ints; -1 signals error (errno set).
 * Callers stash the listen_fd on `self` in birth() and read it
 * back in run/dissolve via the standard locus self-field
 * mechanics — no opaque handle struct needed because the
 * Listener locus IS the handle.
 */

int lotus_tcp_listen_socket(const char *host, uint16_t port) {
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    if (!host) {
        addr.sin_addr.s_addr = htonl(INADDR_ANY);
    } else if (inet_pton(AF_INET, host, &addr.sin_addr) != 1) {
        fprintf(stderr,
                "lotus_tcp_listen_socket: invalid host %s\n", host);
        errno = EINVAL;
        return -1;
    }
    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        perror("lotus_tcp_listen_socket: socket");
        return -1;
    }
    int one = 1;
    if (setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one)) < 0) {
        perror("lotus_tcp_listen_socket: SO_REUSEADDR");
        close(sock);
        return -1;
    }
    /* v1.x polish (2026-05-20): SO_REUSEPORT in addition to
     * SO_REUSEADDR. The pair covers more restart-within-TIME_WAIT
     * edge cases than SO_REUSEADDR alone — specifically when the
     * previous process exited via SIGKILL with TCP state still in
     * the kernel's tear-down window. Surfaced by an HTTP /metrics
     * port 9100 restart-within-60s. SO_REUSEPORT is Linux 3.9+
     * and is best-effort: log + continue if the kernel rejects
     * the option, since SO_REUSEADDR already covers the common
     * case. */
#ifdef SO_REUSEPORT
    if (setsockopt(sock, SOL_SOCKET, SO_REUSEPORT, &one, sizeof(one)) < 0) {
        /* Not fatal — keep going with just SO_REUSEADDR. */
    }
#endif
    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        perror("lotus_tcp_listen_socket: bind");
        close(sock);
        return -1;
    }
    if (listen(sock, 16) < 0) {
        perror("lotus_tcp_listen_socket: listen");
        close(sock);
        return -1;
    }
    return sock;
}

/* F.35 Slice 3 (2026-05-28): "am I on an async_io pool?" check
 * that the blocking-I/O syscall wrappers use to decide whether to
 * park-on-EAGAIN or block the OS thread. The TLS pool ptr is set
 * by the cooperative pool worker before invoking each handler
 * coro; outside a worker context (e.g. fn main() prelude, pinned
 * thread bodies) the pointer is NULL and the check returns 0,
 * preserving the classic blocking semantics. */
static int lotus_io_on_async_io_pool(void) {
    lotus_coop_pool_t *p = g_current_pool_tls;
    return p && p->async_io_enabled;
}

/* F.35 Slice 3: idempotent O_NONBLOCK toggle. Called by the
 * I/O wrappers on async_io pools before the first syscall on
 * each fd. The fcntl(F_GETFL) round-trip costs ~1us; we do it
 * each call rather than tracking a per-fd "already set" bit
 * because fds can cross pool boundaries (listener creates a fd
 * on its pinned thread, publishes it via the bus, worker reads
 * on the async_io pool) and the worker can't know if the
 * upstream already set the flag. */
static void lotus_io_set_nonblock(int fd) {
    if (fd < 0) return;
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags < 0) return;
    if (flags & O_NONBLOCK) return;
    (void)fcntl(fd, F_SETFL, flags | O_NONBLOCK);
}

int lotus_tcp_accept_one(int listen_fd) {
    /* F.35 Slice 3: on async_io pools, accept(2) is non-blocking +
     * park on EAGAIN. Classic blocking accept on every other path. */
    int async = lotus_io_on_async_io_pool();
    if (async) {
        lotus_io_set_nonblock(listen_fd);
    }
    for (;;) {
        int conn = accept(listen_fd, NULL, NULL);
        if (conn >= 0) {
            int nodelay = 1;
            (void)setsockopt(conn, IPPROTO_TCP, TCP_NODELAY,
                             &nodelay, sizeof(nodelay));
            return conn;
        }
        if (errno == EINTR) continue;
        if (async && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            /* Park the calling coro on `listen_fd`-readable. When
             * a connection arrives, epoll fires and the worker
             * resumes us; retry the accept. */
            if (lotus_coop_park_on_fd(listen_fd, EPOLLIN) == 0) {
                continue;
            }
        }
        perror("lotus_tcp_accept_one: accept");
        return -1;
    }
}

int lotus_tcp_connect(const char *host, uint16_t port) {
    /* Mirrors lotus_tcp_create's CONNECT-role logic but returns a
     * raw fd so it can be wrapped by `std::io::tcp::Stream {
     * conn_fd }` from Hale source. Same retry-on-ECONNREFUSED
     * shape (~1s window).
     *
     * C6 (pond follow-up): fast path is still numeric-address
     * via inet_pton; when that returns 0 (host isn't a dotted
     * quad), fall back to getaddrinfo(host, port_str, AF_INET +
     * SOCK_STREAM) and use the first returned address. The numeric
     * path is bit-for-bit identical to the pre-C6 behavior — only
     * non-numeric hosts take the DNS branch. */
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    const char *h = host ? host : "127.0.0.1";
    int pton = inet_pton(AF_INET, h, &addr.sin_addr);
    if (pton != 1) {
        /* C6: getaddrinfo fallback for hostname resolution. We map
         * gai errors onto the existing IoError errno taxonomy so
         * callers don't need a new error kind: EAI_NONAME (unknown
         * host) -> ENOENT ("not_found"); everything else (DNS
         * server failure, transient, no-address-for-family, etc.)
         * -> EHOSTUNREACH ("host_unreachable"). gai_strerror is
         * printed to stderr for diagnostic visibility but doesn't
         * cross the IoError boundary. */
        struct addrinfo hints;
        struct addrinfo *res = NULL;
        memset(&hints, 0, sizeof(hints));
        hints.ai_family   = AF_INET;
        hints.ai_socktype = SOCK_STREAM;
        char port_str[16];
        snprintf(port_str, sizeof(port_str), "%u", (unsigned)port);
        int gai = getaddrinfo(h, port_str, &hints, &res);
        if (gai != 0 || res == NULL) {
            fprintf(stderr,
                    "lotus_tcp_connect: resolve %s: %s\n",
                    h, gai_strerror(gai));
            if (res) freeaddrinfo(res);
            errno = (gai == EAI_NONAME) ? ENOENT : EHOSTUNREACH;
            return -1;
        }
        /* First result wins — round-robin / multi-A handling is
         * the caller's job (they can pre-resolve if they want it). */
        struct sockaddr_in *resolved = (struct sockaddr_in *)res->ai_addr;
        addr.sin_addr = resolved->sin_addr;
        freeaddrinfo(res);
    }
    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        perror("lotus_tcp_connect: socket");
        return -1;
    }
    struct timespec backoff = { 0, 5L * 1000L * 1000L };
    int attempts = 200;
    while (attempts-- > 0) {
        if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) == 0) {
            int nodelay = 1;
            (void)setsockopt(sock, IPPROTO_TCP, TCP_NODELAY,
                             &nodelay, sizeof(nodelay));
            return sock;
        }
        if (errno != ECONNREFUSED && errno != EAGAIN) {
            perror("lotus_tcp_connect: connect");
            close(sock);
            return -1;
        }
        nanosleep(&backoff, NULL);
    }
    fprintf(stderr,
            "lotus_tcp_connect: connect to %s:%u timed out\n",
            h, (unsigned)port);
    close(sock);
    return -1;
}

int lotus_tcp_close_fd(int fd) {
    if (fd < 0) return 0;
    return close(fd);
}

/* C-iii (2026-05-21): graceful interrupt for a blocking accept().
 * shutdown(SHUT_RDWR) on a listen socket forces accept() to
 * return immediately with an error on every OS that ships POSIX
 * sockets — Linux returns EBADF/EINVAL, macOS returns EINVAL.
 * The fd stays open; the caller's accept loop is expected to
 * notice the failure and break. We don't close the fd here so
 * dissolve()'s subsequent close stays the canonical teardown
 * path (and so racing threads can't get a fresh fd handed
 * back from the kernel between this call and dissolve).
 *
 * Returns the shutdown() return value (0 on success, -1 on
 * error; not fatal — already-shutdown / closed fd is a no-op
 * from the caller's perspective). Safe to call from any
 * thread, including cross-scheduler — that's the whole point.
 */
int lotus_tcp_shutdown_listen_socket(int fd) {
    if (fd < 0) return 0;
    return shutdown(fd, SHUT_RDWR);
}

/* Forward decl — defined later (next to lotus_bus_payload_arena
 * proper). Lets the UDP block below build Bytes blobs in the
 * payload arena. */
void *lotus_bus_payload_arena_alloc(size_t size, size_t align);

/*
 * Raw UDP primitives. Datagram socket (SOCK_DGRAM) — preserves
 * per-message boundaries from the kernel, no framing needed at
 * this layer, no delivery guarantee (per UDP semantics). The
 * bus's "deliver one whole message" contract is therefore NOT
 * satisfied by UDP at the substrate level; cross-host bus over
 * UDP is the application's problem (retry, reorder, etc.).
 *
 * Surface (intentionally minimal for v1):
 *   - lotus_udp_bind(host, port) — create a socket bound to
 *     (host, port). host=NULL or "0.0.0.0" → INADDR_ANY.
 *   - lotus_udp_sendto(fd, host, port, buf, len) — send one
 *     datagram (best-effort) to the named peer.
 *   - lotus_udp_recv(fd, buf, cap) — receive one datagram.
 *     Returns bytes received, or -1 on error.
 *   - lotus_udp_close(fd) — close the socket.
 *
 * For held-open send-only sockets where you want a fixed peer,
 * pair lotus_udp_bind(NULL, 0) with repeated lotus_udp_sendto.
 * For receive, bind(host, port) then loop on lotus_udp_recv.
 *
 * Errors return -1 with errno set. lotus_udp_recv truncates
 * messages larger than `cap` (per recvfrom man page).
 */

int lotus_udp_bind(const char *host, uint16_t port) {
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    if (!host || *host == '\0' || strcmp(host, "0.0.0.0") == 0) {
        addr.sin_addr.s_addr = htonl(INADDR_ANY);
    } else if (inet_pton(AF_INET, host, &addr.sin_addr) != 1) {
        fprintf(stderr, "lotus_udp_bind: invalid host %s\n", host);
        errno = EINVAL;
        return -1;
    }
    int sock = socket(AF_INET, SOCK_DGRAM, 0);
    if (sock < 0) {
        perror("lotus_udp_bind: socket");
        return -1;
    }
    int one = 1;
    if (setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one)) < 0) {
        perror("lotus_udp_bind: SO_REUSEADDR");
        close(sock);
        return -1;
    }
    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        perror("lotus_udp_bind: bind");
        close(sock);
        return -1;
    }
    return sock;
}

int lotus_udp_sendto(int fd, const char *host, uint16_t port,
                     const void *buf, size_t len) {
    if (fd < 0 || !host) {
        errno = EINVAL;
        return -1;
    }
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    if (inet_pton(AF_INET, host, &addr.sin_addr) != 1) {
        fprintf(stderr, "lotus_udp_sendto: invalid host %s\n", host);
        errno = EINVAL;
        return -1;
    }
    ssize_t n = sendto(fd, buf, len, 0,
                       (struct sockaddr *)&addr, sizeof(addr));
    if (n < 0) {
        return -1;
    }
    return 0;
}

ssize_t lotus_udp_recv(int fd, void *buf, size_t cap) {
    if (fd < 0 || (!buf && cap > 0)) {
        errno = EINVAL;
        return -1;
    }
    ssize_t n = recvfrom(fd, buf, cap, 0, NULL, NULL);
    return n;
}

int lotus_udp_close(int fd) {
    if (fd < 0) return 0;
    return close(fd);
}

/* 2026-05-26 — UDP multicast surface (P1). Each fn maps directly
 * to a single setsockopt call. `iface` may be NULL or empty to
 * mean INADDR_ANY (the kernel picks an interface). Errors
 * return -1 with errno set, matching the rest of the
 * lotus_udp_* family.
 *
 * `group` must be an IPv4 dotted-quad string in the 224.0.0.0/4
 * range; IPv6 multicast support is a separate follow-on
 * (`lotus_udp6_join_group` will mirror this shape on
 * `IPV6_JOIN_GROUP`). */
int lotus_udp_join_group(int fd, const char *group, const char *iface) {
    if (fd < 0 || !group) {
        errno = EINVAL;
        return -1;
    }
    struct ip_mreq mreq;
    memset(&mreq, 0, sizeof(mreq));
    if (inet_pton(AF_INET, group, &mreq.imr_multiaddr) != 1) {
        errno = EINVAL;
        return -1;
    }
    if (!iface || *iface == '\0') {
        mreq.imr_interface.s_addr = htonl(INADDR_ANY);
    } else if (inet_pton(AF_INET, iface, &mreq.imr_interface) != 1) {
        errno = EINVAL;
        return -1;
    }
    return setsockopt(fd, IPPROTO_IP, IP_ADD_MEMBERSHIP,
                      &mreq, sizeof(mreq));
}

int lotus_udp_leave_group(int fd, const char *group, const char *iface) {
    if (fd < 0 || !group) {
        errno = EINVAL;
        return -1;
    }
    struct ip_mreq mreq;
    memset(&mreq, 0, sizeof(mreq));
    if (inet_pton(AF_INET, group, &mreq.imr_multiaddr) != 1) {
        errno = EINVAL;
        return -1;
    }
    if (!iface || *iface == '\0') {
        mreq.imr_interface.s_addr = htonl(INADDR_ANY);
    } else if (inet_pton(AF_INET, iface, &mreq.imr_interface) != 1) {
        errno = EINVAL;
        return -1;
    }
    return setsockopt(fd, IPPROTO_IP, IP_DROP_MEMBERSHIP,
                      &mreq, sizeof(mreq));
}

int lotus_udp_set_multicast_ttl(int fd, int ttl) {
    if (fd < 0) { errno = EINVAL; return -1; }
    if (ttl < 0 || ttl > 255) { errno = EINVAL; return -1; }
    unsigned char ttl_u8 = (unsigned char)ttl;
    return setsockopt(fd, IPPROTO_IP, IP_MULTICAST_TTL,
                      &ttl_u8, sizeof(ttl_u8));
}

int lotus_udp_set_multicast_loop(int fd, int enabled) {
    if (fd < 0) { errno = EINVAL; return -1; }
    unsigned char on = enabled ? 1 : 0;
    return setsockopt(fd, IPPROTO_IP, IP_MULTICAST_LOOP,
                      &on, sizeof(on));
}

int lotus_udp_set_multicast_iface(int fd, const char *addr) {
    if (fd < 0 || !addr) { errno = EINVAL; return -1; }
    struct in_addr in;
    memset(&in, 0, sizeof(in));
    if (!*addr) {
        in.s_addr = htonl(INADDR_ANY);
    } else if (inet_pton(AF_INET, addr, &in) != 1) {
        errno = EINVAL;
        return -1;
    }
    return setsockopt(fd, IPPROTO_IP, IP_MULTICAST_IF,
                      &in, sizeof(in));
}

/* 2026-05-26 — transparent setsockopt / getsockopt pass-through
 * (P2). Apps that know setsockopt from the Linux man pages can
 * tune any option without us pre-deciding the curated surface.
 * level/name come from named Int constants in the
 * std::io::sockopt module (e.g. SOL_SOCKET = 1, SO_RCVBUF = 8 on
 * Linux); the C primitives don't validate them — invalid combos
 * surface as the kernel's EINVAL / ENOPROTOOPT. */
int lotus_udp_setsockopt_int(int fd, int level, int name, int value) {
    if (fd < 0) { errno = EINVAL; return -1; }
    return setsockopt(fd, level, name, &value, sizeof(value));
}

int lotus_udp_setsockopt_bool(int fd, int level, int name, int enabled) {
    if (fd < 0) { errno = EINVAL; return -1; }
    int on = enabled ? 1 : 0;
    return setsockopt(fd, level, name, &on, sizeof(on));
}

/* Returns the int-typed option value on success; INT_MIN on
 * error (callers that need to distinguish a genuine INT_MIN
 * value from an error should use the upcoming
 * lotus_udp_getsockopt_int_out variant — for the common case
 * (buffer sizes, TTLs, byte counts) INT_MIN is a clean
 * sentinel). */
int lotus_udp_getsockopt_int(int fd, int level, int name) {
    if (fd < 0) { errno = EINVAL; return INT_MIN; }
    int value = 0;
    socklen_t sz = sizeof(value);
    if (getsockopt(fd, level, name, &value, &sz) < 0) {
        return INT_MIN;
    }
    return value;
}

/* 2026-05-26 — `std::io::sockopt::*` named-constant getters.
 * Each returns the platform's actual numeric value of the
 * corresponding setsockopt level / name / TOS / flag, so apps
 * can write
 *
 *     std::io::udp::set_option_int(fd,
 *         std::io::sockopt::SOL_SOCKET(),
 *         std::io::sockopt::SO_RCVBUF(),
 *         4 * 1024 * 1024) or raise;
 *
 * without us pre-deciding the curated UDP surface. Values
 * come from <sys/socket.h> / <netinet/in.h>, so they track
 * the platform (Linux constants differ from BSD / macOS;
 * Hale's already Linux-centric for cooperative scheduling
 * primitives, but the getter abstraction keeps the door open).
 */
#define LOTUS_SOCKOPT_GETTER(NAME) \
    int lotus_sockopt_##NAME(void) { return (int)NAME; }

LOTUS_SOCKOPT_GETTER(SOL_SOCKET)
LOTUS_SOCKOPT_GETTER(IPPROTO_IP)
LOTUS_SOCKOPT_GETTER(IPPROTO_IPV6)
LOTUS_SOCKOPT_GETTER(IPPROTO_TCP)
LOTUS_SOCKOPT_GETTER(IPPROTO_UDP)
LOTUS_SOCKOPT_GETTER(SO_REUSEADDR)
LOTUS_SOCKOPT_GETTER(SO_REUSEPORT)
LOTUS_SOCKOPT_GETTER(SO_RCVBUF)
LOTUS_SOCKOPT_GETTER(SO_SNDBUF)
LOTUS_SOCKOPT_GETTER(SO_RCVTIMEO)
LOTUS_SOCKOPT_GETTER(SO_SNDTIMEO)
LOTUS_SOCKOPT_GETTER(SO_BROADCAST)
LOTUS_SOCKOPT_GETTER(SO_KEEPALIVE)
LOTUS_SOCKOPT_GETTER(SO_LINGER)
LOTUS_SOCKOPT_GETTER(SO_PRIORITY)
LOTUS_SOCKOPT_GETTER(IP_TTL)
LOTUS_SOCKOPT_GETTER(IP_TOS)
LOTUS_SOCKOPT_GETTER(IP_MULTICAST_TTL)
LOTUS_SOCKOPT_GETTER(IP_MULTICAST_LOOP)
LOTUS_SOCKOPT_GETTER(IP_MULTICAST_IF)
LOTUS_SOCKOPT_GETTER(IP_ADD_MEMBERSHIP)
LOTUS_SOCKOPT_GETTER(IP_DROP_MEMBERSHIP)
LOTUS_SOCKOPT_GETTER(IP_PKTINFO)
#ifdef SO_BINDTODEVICE
LOTUS_SOCKOPT_GETTER(SO_BINDTODEVICE)
#else
int lotus_sockopt_SO_BINDTODEVICE(void) { return -1; }
#endif
/* IP_MTU_DISCOVER + IP_PMTUDISC_* (2026-05-27) — let apps opt
 * into kernel-side fragmentation when running on a path whose
 * MTU isn't end-to-end jumbo. Default (Linux) is
 * IP_PMTUDISC_WANT (DF=1, fail-with-EMSGSIZE on
 * oversized-datagram). Setting IP_MTU_DISCOVER to
 * IP_PMTUDISC_DONT clears DF so the upstream router
 * fragments — degrades to per-fragment loss multiplier but
 * unblocks delivery on a sub-MTU path. Linux-only; the
 * #ifdef guards keep the door open for other platforms. */
#ifdef IP_MTU_DISCOVER
LOTUS_SOCKOPT_GETTER(IP_MTU_DISCOVER)
#else
int lotus_sockopt_IP_MTU_DISCOVER(void) { return -1; }
#endif
#ifdef IP_PMTUDISC_DONT
LOTUS_SOCKOPT_GETTER(IP_PMTUDISC_DONT)
#else
int lotus_sockopt_IP_PMTUDISC_DONT(void) { return -1; }
#endif
#ifdef IP_PMTUDISC_WANT
LOTUS_SOCKOPT_GETTER(IP_PMTUDISC_WANT)
#else
int lotus_sockopt_IP_PMTUDISC_WANT(void) { return -1; }
#endif
#ifdef IP_PMTUDISC_DO
LOTUS_SOCKOPT_GETTER(IP_PMTUDISC_DO)
#else
int lotus_sockopt_IP_PMTUDISC_DO(void) { return -1; }
#endif
#ifdef IP_PMTUDISC_PROBE
LOTUS_SOCKOPT_GETTER(IP_PMTUDISC_PROBE)
#else
int lotus_sockopt_IP_PMTUDISC_PROBE(void) { return -1; }
#endif
#undef LOTUS_SOCKOPT_GETTER

/* 2026-05-26 — UDP P4: recv_with_source + set_*_timeout.
 *
 * `recv_with_source` captures the sender's IP + port into
 * thread-local storage and returns the datagram bytes. Two
 * companion getters (`last_source_host`, `last_source_port`)
 * read the TLS slots. Pattern mirrors C's errno + strerror
 * — apps know to read the source IMMEDIATELY after the recv.
 * The TLS is reset to "0.0.0.0:0" if a subsequent recv fails;
 * holding the source across an unrelated stdlib call is
 * caller's responsibility (the slots are stable across any
 * call that doesn't itself touch them, but the contract is
 * "read right after recv").
 *
 * Buffer sizes: INET_ADDRSTRLEN = 16 covers any IPv4 dotted-
 * quad; INET6_ADDRSTRLEN = 46 leaves headroom for the v6
 * follow-on. */
static __thread char g_udp_last_source_host[64] = "0.0.0.0";
static __thread int64_t g_udp_last_source_port = 0;

void *lotus_udp_recv_bytes_with_source(int fd, int max_bytes) {
    if (fd < 0 || max_bytes <= 0) {
        errno = EINVAL;
        return NULL;
    }
    char stack_buf[65536];
    size_t cap = (size_t)max_bytes;
    if (cap > sizeof(stack_buf)) cap = sizeof(stack_buf);
    struct sockaddr_in src;
    socklen_t src_len = sizeof(src);
    memset(&src, 0, sizeof(src));
    ssize_t n = recvfrom(fd, stack_buf, cap, 0,
                         (struct sockaddr *)&src, &src_len);
    if (n < 0) {
        /* Reset source TLS so a stale value isn't read by a
         * caller that ignores the err path. */
        g_udp_last_source_host[0] = '0';
        g_udp_last_source_host[1] = '.';
        g_udp_last_source_host[2] = '0';
        g_udp_last_source_host[3] = '.';
        g_udp_last_source_host[4] = '0';
        g_udp_last_source_host[5] = '.';
        g_udp_last_source_host[6] = '0';
        g_udp_last_source_host[7] = '\0';
        g_udp_last_source_port = 0;
        return NULL;
    }
    if (src.sin_family == AF_INET) {
        if (!inet_ntop(AF_INET, &src.sin_addr,
                       g_udp_last_source_host,
                       sizeof(g_udp_last_source_host))) {
            g_udp_last_source_host[0] = '\0';
        }
        g_udp_last_source_port = (int64_t)ntohs(src.sin_port);
    } else {
        g_udp_last_source_host[0] = '\0';
        g_udp_last_source_port = 0;
    }
    /* Build the Bytes blob in the bus payload arena. */
    size_t blob_size = sizeof(int64_t) + (size_t)n;
    void *blob = lotus_bus_payload_arena_alloc(blob_size, 8);
    if (!blob) return NULL;
    *(int64_t *)blob = (int64_t)n;
    if (n > 0) {
        memcpy((char *)blob + sizeof(int64_t), stack_buf, (size_t)n);
    }
    return blob;
}

/* Returns a String in the bus payload arena holding the last
 * recv'd datagram's source IP. NUL-terminated; safe across the
 * Hale String ABI. Returns "" if no recv_with_source has been
 * called on this thread yet. */
const char *lotus_udp_last_source_host(void) {
    size_t n = strlen(g_udp_last_source_host);
    char *out = (char *)lotus_bus_payload_arena_alloc(n + 1, 1);
    if (!out) return "";
    memcpy(out, g_udp_last_source_host, n);
    out[n] = '\0';
    return out;
}

int64_t lotus_udp_last_source_port(void) {
    return g_udp_last_source_port;
}

/* 2026-05-26 — UDP send/recv timeouts via SO_RCVTIMEO /
 * SO_SNDTIMEO. These take a struct timeval (not a plain int)
 * so they can't ride the set_option_int pass-through.
 * Accepts a Hale Duration (i64 nanoseconds); 0 means "no
 * timeout" (the default; blocking).
 *
 * 2026-05-27 — the helper is fd-generic, so the TCP siblings
 * (`lotus_tcp_set_*_timeout_ns` below) share it. setsockopt
 * with SO_RCVTIMEO / SO_SNDTIMEO works the same way on
 * SOCK_STREAM and SOCK_DGRAM sockets — both gate `recv` /
 * `send` blocking with the same timeval shape. */
static int sock_set_timeout_ns(int fd, int name, int64_t ns) {
    if (fd < 0) { errno = EINVAL; return -1; }
    if (ns < 0) { errno = EINVAL; return -1; }
    struct timeval tv;
    tv.tv_sec = (time_t)(ns / 1000000000);
    tv.tv_usec = (suseconds_t)((ns % 1000000000) / 1000);
    return setsockopt(fd, SOL_SOCKET, name, &tv, sizeof(tv));
}

int lotus_udp_set_recv_timeout_ns(int fd, int64_t ns) {
    return sock_set_timeout_ns(fd, SO_RCVTIMEO, ns);
}

int lotus_udp_set_send_timeout_ns(int fd, int64_t ns) {
    return sock_set_timeout_ns(fd, SO_SNDTIMEO, ns);
}

/* 2026-05-27 — TCP send/recv timeouts via the same
 * SO_RCVTIMEO / SO_SNDTIMEO mechanism. Unblocks recv loops
 * that need to do periodic work (silence detection,
 * heartbeats, watchdog timers) — previously a `recv_bytes`
 * would block indefinitely waiting for the next byte and
 * the surrounding loop had no way to wake up on a quiet
 * connection. Closes the gap with std::io::udp (P4,
 * 2026-05-26) where the same surface already shipped. */
int lotus_tcp_set_recv_timeout_ns(int fd, int64_t ns) {
    return sock_set_timeout_ns(fd, SO_RCVTIMEO, ns);
}

int lotus_tcp_set_send_timeout_ns(int fd, int64_t ns) {
    return sock_set_timeout_ns(fd, SO_SNDTIMEO, ns);
}

/* Hale-side wrappers that adapt the raw primitives to the
 * String / Bytes ABI. The String variant of sendto takes a
 * NUL-terminated string and computes its length internally;
 * the Bytes variants take the standard [i64 len][u8 body[len]]
 * blob and walk the prefix. recv_bytes allocates the result
 * in the bus payload arena like its tcp sibling. */

int lotus_udp_sendto_str(int fd, const char *host, uint16_t port,
                         const char *msg) {
    if (!msg) {
        errno = EINVAL;
        return -1;
    }
    return lotus_udp_sendto(fd, host, port, msg, strlen(msg));
}

void *lotus_udp_recv_bytes_global(int fd, int max_bytes) {
    if (fd < 0 || max_bytes <= 0) {
        errno = EINVAL;
        return NULL;
    }
    /* Use a stack buffer for the kernel handoff; the result is
     * copied into a Bytes blob in the payload arena. UDP max
     * packet size is ~65507 bytes on IPv4, so a 64KB stack
     * buffer covers anything. */
    char stack_buf[65536];
    size_t cap = (size_t)max_bytes;
    if (cap > sizeof(stack_buf)) cap = sizeof(stack_buf);
    ssize_t n = recvfrom(fd, stack_buf, cap, 0, NULL, NULL);
    if (n < 0) return NULL;
    /* Hand-build a Bytes blob ([i64 len][body]) in the global
     * payload arena via the public alloc helper. Mirrors the
     * lotus_bytes_create shape (without needing a direct
     * lotus_arena_t handle to the static g_bus_payload_arena). */
    size_t blob_size = sizeof(int64_t) + (size_t)n;
    void *blob = lotus_bus_payload_arena_alloc(blob_size, 8);
    if (!blob) return NULL;
    *(int64_t *)blob = (int64_t)n;
    if (n > 0) {
        memcpy((char *)blob + sizeof(int64_t), stack_buf, (size_t)n);
    }
    return blob;
}

/*
 * m81: send / recv on a connected TCP fd, exposed to Hale
 * as String-shaped operations. send_str writes the bytes of
 * the NUL-terminated input (length via strlen — embedded NULs
 * truncate, mirroring m75's std::io::fs::write_file behavior;
 * binary I/O waits on Bytes codegen). recv_str reads up to
 * max_bytes into a freshly-allocated buffer in the lazy global
 * payload arena, NUL-terminates at the actual byte count, and
 * returns a stable pointer the caller can hold for the program
 * lifetime (same ownership model as m75's read_file).
 */

/* Forward decl — defined later in this file. */
void *lotus_bus_payload_arena_alloc(size_t size, size_t align);

int lotus_tcp_send_str(int fd, const char *msg) {
    if (fd < 0) {
        errno = EBADF;
        return -1;
    }
    if (!msg) {
        errno = EINVAL;
        return -1;
    }
    /* F.35 Slice 3: park on EPOLLOUT for async_io pools. */
    int async = lotus_io_on_async_io_pool();
    if (async) {
        lotus_io_set_nonblock(fd);
    }
    size_t len = strlen(msg);
    const char *p = msg;
    size_t      left = len;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        if (w < 0 && async && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            if (lotus_coop_park_on_fd(fd, EPOLLOUT) == 0) continue;
        }
        perror("lotus_tcp_send_str: write");
        return -1;
    }
    return 0;
}

/*
 * m89: write a Bytes blob to a TCP fd. Uses the explicit
 * length stored in the blob's prefix (not strlen) so embedded
 * NUL bytes don't truncate. write(2) loop handles partial
 * writes; returns 0 on full send, -1 on error.
 */
int lotus_tcp_send_bytes(int fd, const void *bytes_ptr) {
    if (fd < 0) {
        errno = EBADF;
        return -1;
    }
    if (!bytes_ptr) {
        errno = EINVAL;
        return -1;
    }
    int64_t total = lotus_bytes_len(bytes_ptr);
    if (total < 0) {
        errno = EINVAL;
        return -1;
    }
    /* F.35 Slice 3: on async_io pools, park on EPOLLOUT when the
     * send buffer fills (EAGAIN). The fd is set non-blocking once
     * here; subsequent writes inherit the flag. */
    int async = lotus_io_on_async_io_pool();
    if (async) {
        lotus_io_set_nonblock(fd);
    }
    const char *p = (const char *)bytes_ptr + sizeof(int64_t);
    size_t left = (size_t)total;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        if (w < 0 && async && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            if (lotus_coop_park_on_fd(fd, EPOLLOUT) == 0) continue;
        }
        perror("lotus_tcp_send_bytes: write");
        return -1;
    }
    return 0;
}

const char *lotus_tcp_recv_str(int fd, int max_bytes) {
    /* Stable empty-string sentinel — same trick as g_empty_str
     * but local to this function-family because m81 may run
     * before lotus_env_init has cleared the env globals. */
    static const char empty[1] = { 0 };
    if (fd < 0 || max_bytes <= 0) {
        return empty;
    }
    /* F.35 Slice 3: park on EAGAIN for async_io pools, classic
     * blocking read otherwise. */
    int async = lotus_io_on_async_io_pool();
    if (async) {
        lotus_io_set_nonblock(fd);
    }
    size_t cap = (size_t)max_bytes;
    char *buf = (char *)lotus_bus_payload_arena_alloc(cap + 1, 1);
    if (!buf) {
        return empty;
    }
    ssize_t n;
    for (;;) {
        n = read(fd, buf, cap);
        if (n >= 0) break;
        if (errno == EINTR) continue;
        if (async && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            if (lotus_coop_park_on_fd(fd, EPOLLIN) == 0) continue;
        }
        /* Treat read errors as "got nothing" at this level —
         * the buffer is in the lazy arena so it persists; the
         * stable empty-string sentinel signals "no data" to
         * the caller. */
        return empty;
    }
    /* NUL-terminate at the actual bytes-read offset; a zero-byte
     * read (peer closed cleanly) yields an empty string at the
     * arena buffer. */
    buf[(size_t)n] = '\0';
    return buf;
}

/* Phase 2g: forward decls for the lotus_*_bytes helpers below.
 * Their bodies live next to the other global-payload-arena
 * wrappers (after lotus_bus_payload_arena_alloc at ~line 2814)
 * because that's where g_bus_payload_arena is first declared. */
void *lotus_tcp_recv_bytes(int fd, int max_bytes);
const char *lotus_str_from_bytes(const void *b);
void *lotus_bytes_from_str(const char *s);
int64_t lotus_bytes_at(const void *b, int64_t i);
void *lotus_bytes_slice(const void *b, int64_t lo, int64_t hi);

/* Wave B: same-shape forward decl for the bus-payload-arena
 * accessor used by lotus_bus_remote_fanout's adapter dispatch
 * path. Body lives with the rest of the payload-arena machinery
 * once g_bus_payload_arena itself is in scope. */
lotus_arena_t *lotus_bus_payload_arena_get(void);

/* Phase 2e + 2f + C9: forward decls for fs primitives whose
 * bodies need g_bus_payload_arena (declared further below) so
 * the returned String outlives the call frame. */
int64_t lotus_fs_list_dir_count(const char *path);
const char *lotus_fs_list_dir_at(const char *path, int64_t idx);
const char *lotus_fs_mktemp(const char *prefix, const char *suffix);

/*
 * m74: filesystem primitives (`std::io::fs::*` substrate).
 *
 * One-shot synchronous file operations. POSIX wrappers, no
 * caching, no buffering — the same shape POSIX presents,
 * surfaced through a small C ABI that codegen calls from the
 * `std::io::fs::__*` magic-path stdlib primitives.
 *
 * Shape choice: each function takes raw pointers + sizes and
 * returns either a count (read/size) or 0/-1 status (write/
 * exists). No opaque file-handle struct because Phase-1 file
 * operations are one-shot — there's no lifetime-of-a-stream
 * concept to manage. A future milestone that needs streaming
 * reads/writes adds a separate `lotus_fs_open` / `_read` /
 * `_close` family alongside this one.
 *
 * read_dir is deliberately deferred: the variable-length
 * output story (NUL-separated buffer? iteration model? per-
 * entry callback?) deserves its own design pass and is not
 * needed for the m76 capstone (which reads + writes a config
 * file and a log file, not a directory listing).
 */

/* Read up to `out_cap` bytes from `path` into `out_buf`.
 * Returns bytes read (>=0) on success, -1 on error (errno set).
 * If the file is larger than `out_cap` the surplus is silently
 * dropped — the caller decides whether that's acceptable by
 * comparing the return against the cap. Files larger than what
 * fits in size_t are not supported (extremely rare on the v0
 * target). */
/* 2026-05-21: size-tolerant read variant that doesn't trust
 * fstat. For synthesized files (`/proc` entries, `/sys`
 * entries, FIFO pipes, sockets) fstat returns `st_size = 0`
 * even though read(2) yields real content — the existing
 * read_file path pre-allocates a 0-byte buffer and reads
 * nothing, surfacing an empty String. Surfaced by an attempt
 * to expose `/proc/self/statm` as a metrics gauge.
 *
 * This variant reads into a growing buffer (4 KiB initial,
 * doubling, capped at 64 MiB) and returns a NUL-terminated
 * String pointer allocated in `a`. NULL on any error (open,
 * read, alloc-fail, exceeded cap). Used by the std::io::fs::
 * read_file codegen path; the older fstat-then-read function
 * stays callable for tests that wrote against the old buffer-
 * provided-by-caller shape.
 *
 * 64 MiB cap is a runaway-guard, not a memory budget — for
 * the /proc use case real files are 4–64 KiB. Any caller
 * hitting the cap is reading something they probably want a
 * streaming API for. We surface "exceeded cap" as the same
 * NULL the open/read errors do; callers can distinguish via
 * errno (EFBIG when the cap fires). */
#define LOTUS_FS_READ_FILE_GROWING_CAP ((size_t)64 * 1024 * 1024)

char *lotus_fs_read_file_growing(lotus_arena_t *a, const char *path) {
    if (!a || !path) {
        errno = EINVAL;
        return NULL;
    }
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        return NULL;
    }
    size_t cap = 4096;
    char *buf = (char *)lotus_arena_alloc(a, cap + 1, 1);
    if (!buf) {
        close(fd);
        errno = ENOMEM;
        return NULL;
    }
    size_t used = 0;
    for (;;) {
        if (used == cap) {
            size_t new_cap = cap * 2;
            if (new_cap > LOTUS_FS_READ_FILE_GROWING_CAP) {
                close(fd);
                errno = EFBIG;
                return NULL;
            }
            char *new_buf =
                (char *)lotus_arena_alloc(a, new_cap + 1, 1);
            if (!new_buf) {
                close(fd);
                errno = ENOMEM;
                return NULL;
            }
            memcpy(new_buf, buf, used);
            buf = new_buf;
            cap = new_cap;
        }
        ssize_t r = read(fd, buf + used, cap - used);
        if (r > 0) {
            used += (size_t)r;
            continue;
        }
        if (r == 0) break;
        if (errno == EINTR) continue;
        close(fd);
        return NULL;
    }
    close(fd);
    buf[used] = '\0';
    return buf;
}

ssize_t lotus_fs_read_file(const char *path,
                           void *out_buf,
                           size_t out_cap) {
    if (!path || (!out_buf && out_cap > 0)) {
        errno = EINVAL;
        return -1;
    }
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        /* keep the diagnostic terse — perror would be noisy
         * for the common "file not found" case; callers that
         * want to distinguish errors check errno. */
        return -1;
    }
    char *p = (char *)out_buf;
    size_t left = out_cap;
    ssize_t total = 0;
    while (left > 0) {
        ssize_t r = read(fd, p, left);
        if (r > 0) {
            p     += (size_t)r;
            left  -= (size_t)r;
            total += r;
            continue;
        }
        if (r == 0) break;             /* EOF */
        if (errno == EINTR) continue;  /* interrupted; retry */
        close(fd);
        return -1;
    }
    close(fd);
    return total;
}

/*
 * m89: read whole file as a Bytes blob. Allocates a fresh
 * Bytes value on the caller's arena sized to the file's
 * length, fills it from the fd, returns the pointer. NULL
 * on any error (file missing, permission denied, etc.) —
 * caller distinguishes via errno. Used by std::io::fs::
 * read_bytes for binary file I/O where String's NUL
 * truncation would silently corrupt the data.
 */
void *lotus_fs_read_bytes(lotus_arena_t *a, const char *path) {
    if (!a || !path) {
        errno = EINVAL;
        return NULL;
    }
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        return NULL;
    }
    /* Stat to size the blob exactly. fstat keeps us off a
     * second open; on regular files st_size is the byte
     * count we need. */
    struct stat st;
    if (fstat(fd, &st) < 0) {
        close(fd);
        return NULL;
    }
    int64_t size = (int64_t)st.st_size;
    void *blob = lotus_bytes_create(a, size);
    if (!blob) {
        close(fd);
        errno = ENOMEM;
        return NULL;
    }
    char *body = (char *)lotus_bytes_data(blob);
    size_t left = (size_t)size;
    while (left > 0) {
        ssize_t r = read(fd, body, left);
        if (r > 0) {
            body += (size_t)r;
            left -= (size_t)r;
            continue;
        }
        if (r == 0) break;
        if (errno == EINTR) continue;
        close(fd);
        return NULL;
    }
    close(fd);
    /* If the file shrank between fstat and read (race), the
     * trailing bytes are uninitialized blob memory. v0
     * accepts that — the next milestone might re-read st_size
     * after the loop and patch the prefix down. */
    return blob;
}

/*
 * m90: enumerate a directory's entries, returning a single
 * String with one entry per line (`\n`-separated, trailing
 * newline included). Skips `.` and `..` so callers don't
 * have to filter them. Errors (path missing, not a
 * directory, permission denied) return an empty string —
 * same soft-fail shape as the rest of std::io::fs.
 *
 * v0 design choice: newline-separated String, not Bytes /
 * not a List<String>, so the substrate composes with the
 * existing String primitives (index_of, slice). When Hale
 * grows a generic List<T> this can grow a sibling
 * `list_dir_entries(path) -> [String]` API; for Phase 5's
 * doc-server need (enumerate `.md` files in docs/), the
 * String shape is sufficient — user code walks newlines via
 * std::str::index_of("\n").
 *
 * Filenames with embedded `\n` would corrupt this format.
 * POSIX permits them (only `\0` and `/` are illegal in path
 * segments) but they're rare; v0 documents the limitation
 * and chooses the simpler shape.
 */
const char *lotus_fs_list_dir(lotus_arena_t *a, const char *path) {
    static const char empty[1] = { 0 };
    if (!a || !path) {
        return empty;
    }
    DIR *dir = opendir(path);
    if (!dir) {
        return empty;
    }
    /* First pass: tally the byte count we need. struct
     * dirent's d_name is NUL-terminated; we add 1 byte per
     * entry for the joining `\n` (plus the trailing one). */
    size_t total = 0;
    struct dirent *e;
    while ((e = readdir(dir)) != NULL) {
        if (strcmp(e->d_name, ".") == 0
            || strcmp(e->d_name, "..") == 0) {
            continue;
        }
        total += strlen(e->d_name) + 1;
    }
    rewinddir(dir);

    /* Allocate (total + 1) for the trailing NUL terminator. */
    char *buf = (char *)lotus_arena_alloc(a, total + 1, 1);
    if (!buf) {
        closedir(dir);
        return empty;
    }
    /* Second pass: copy entry names + newlines. Because
     * filesystems can change between rewinddir and the
     * second readdir, the actual bytes copied may differ
     * from the first-pass tally; we cap by `total` to
     * avoid overrun and accept that a directory mutated
     * mid-call may lose late-arriving entries. v0 considers
     * directory-listing under concurrent mutation an out-of-
     * scope concern. */
    char *p = buf;
    size_t left = total;
    while ((e = readdir(dir)) != NULL && left > 0) {
        if (strcmp(e->d_name, ".") == 0
            || strcmp(e->d_name, "..") == 0) {
            continue;
        }
        size_t nlen = strlen(e->d_name);
        if (nlen + 1 > left) break;
        memcpy(p, e->d_name, nlen);
        p[nlen] = '\n';
        p += nlen + 1;
        left -= nlen + 1;
    }
    *p = '\0';
    closedir(dir);
    return buf;
}

/* Write exactly `len` bytes from `buf` to `path`. Truncates
 * any existing file. Returns 0 on success, -1 on error. */
int lotus_fs_write_file(const char *path,
                        const void *buf,
                        size_t len) {
    if (!path || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        return -1;
    }
    const char *p = (const char *)buf;
    size_t left = len;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        close(fd);
        return -1;
    }
    /* close return matters for write_file: a deferred filesystem
     * error (e.g. NFS write-back) surfaces here, not in write(). */
    if (close(fd) != 0) {
        return -1;
    }
    return 0;
}

/* Append `len` bytes of `buf` to `path`. Creates the file with
 * mode 0644 if it doesn't exist; otherwise opens existing for
 * append. Returns 0 on success, -1 on error (errno set).
 * Companion to lotus_fs_write_file (which truncates); ergonomics
 * milestone resolves the apps/log-router friction "no append
 * primitive forces buffer-everything-then-flush at dissolve". */
int lotus_fs_write_file_append(const char *path,
                               const void *buf,
                               size_t len) {
    if (!path || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    int fd = open(path, O_WRONLY | O_CREAT | O_APPEND, 0644);
    if (fd < 0) {
        return -1;
    }
    const char *p = (const char *)buf;
    size_t left = len;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        close(fd);
        return -1;
    }
    if (close(fd) != 0) {
        return -1;
    }
    return 0;
}

/* Create the directory at `path` with mode 0755. Returns 0 on
 * success, -1 on error (errno set; EEXIST when the directory
 * already exists). NOT recursive — callers that want
 * `mkdir -p`-style semantics should test parent existence
 * themselves. Resolves apps/ssg friction "no mkdir / create_dir
 * forces shell-out via README precondition". */
int lotus_fs_mkdir(const char *path) {
    if (!path) {
        errno = EINVAL;
        return -1;
    }
    if (mkdir(path, 0755) < 0) {
        return -1;
    }
    return 0;
}

/* C9 (pond/logfmt rotation): atomic rename `src` → `dst`. POSIX
 * rename(2); atomic on the same filesystem, EXDEV cross-fs.
 * Returns 0 on success, -1 on error (errno set). The codegen
 * wrapper anchors the IoError.path to `dst` because the
 * destination is the more diagnostic of the two on the common
 * failure modes (target dir missing, target already a non-empty
 * dir, cross-fs, etc.). */
int lotus_fs_rename(const char *src, const char *dst) {
    if (!src || !dst) {
        errno = EINVAL;
        return -1;
    }
    if (rename(src, dst) < 0) {
        return -1;
    }
    return 0;
}

/* C9 (pond/logfmt rotation): unlink `path`. POSIX unlink(2) —
 * removes a regular file or symlink. Directories require rmdir
 * (not yet exposed). Returns 0 on success, -1 on error (errno
 * set; ENOENT when the path didn't exist, EISDIR on a directory
 * target). */
int lotus_fs_unlink(const char *path) {
    if (!path) {
        errno = EINVAL;
        return -1;
    }
    if (unlink(path) < 0) {
        return -1;
    }
    return 0;
}

/* Returns the size of `path` in bytes, or -1 on error. Follows
 * symlinks (stat, not lstat). */
int64_t lotus_fs_file_size(const char *path) {
    if (!path) {
        errno = EINVAL;
        return -1;
    }
    struct stat st;
    if (stat(path, &st) < 0) {
        return -1;
    }
    return (int64_t)st.st_size;
}

/* Returns 1 if `path` exists, 0 otherwise. Errors that aren't
 * "doesn't exist" (e.g. EACCES on a parent dir) also return 0;
 * the caller can disambiguate via errno if needed. */
int lotus_fs_file_exists(const char *path) {
    if (!path) {
        errno = EINVAL;
        return 0;
    }
    struct stat st;
    return stat(path, &st) == 0 ? 1 : 0;
}

/*
 * Held-open file substrate (std::io::file::File).
 *
 * Complements the one-shot std::io::fs::* path-calls above:
 * those open+do+close per call; this family hands a raw fd back
 * to the Hale-side File locus, which stashes it on self.fd
 * and runs lotus_file_close in its dissolve(). The locus's
 * scope-exit dissolve (per let-bound deferred-dissolve rules)
 * makes "open a file, do held-state I/O, get cleanup for free"
 * the same shape Stream / Listener already have for sockets.
 *
 * Mode string semantics (POSIX flag derivation):
 *   "r"  → O_RDONLY
 *   "w"  → O_WRONLY | O_CREAT | O_TRUNC   (mode 0644)
 *   "a"  → O_WRONLY | O_CREAT | O_APPEND  (mode 0644)
 *   "r+" → O_RDWR
 *   "w+" → O_RDWR   | O_CREAT | O_TRUNC   (mode 0644)
 *
 * Returned String data (read_line) lives in the global bus
 * payload arena, matching the lifetime semantics of
 * std::io::fs::read_file — the buffer survives the call frame
 * and the eventual File.dissolve(), so a String escaping the
 * File's scope stays valid.
 */

/* Open `path` with POSIX mode derived from `mode_str`. Returns
 * the fd (>= 0) on success, -1 on error (errno set). The mode
 * string is one of {"r","w","a","r+","w+"} — anything else is
 * EINVAL. */
int lotus_file_open(const char *path, const char *mode_str) {
    if (!path || !mode_str) {
        errno = EINVAL;
        return -1;
    }
    int flags;
    int create_perm = 0;
    if (strcmp(mode_str, "r") == 0) {
        flags = O_RDONLY;
    } else if (strcmp(mode_str, "w") == 0) {
        flags = O_WRONLY | O_CREAT | O_TRUNC;
        create_perm = 0644;
    } else if (strcmp(mode_str, "a") == 0) {
        flags = O_WRONLY | O_CREAT | O_APPEND;
        create_perm = 0644;
    } else if (strcmp(mode_str, "r+") == 0) {
        flags = O_RDWR;
    } else if (strcmp(mode_str, "w+") == 0) {
        flags = O_RDWR | O_CREAT | O_TRUNC;
        create_perm = 0644;
    } else {
        errno = EINVAL;
        return -1;
    }
    int fd = (create_perm != 0)
        ? open(path, flags, create_perm)
        : open(path, flags);
    return fd;
}

/* Close a held-open fd. Returns 0 on success, -1 on error.
 * Idempotent for fd == -1 (the "never opened" sentinel) — that
 * lets File.dissolve() blind-close on the params default
 * without synthesizing a phantom IoError. */
int lotus_file_close(int fd) {
    if (fd < 0) return 0;
    return close(fd);
}

/* Read one '\n'-terminated line from `fd`, returning a heap-
 * allocated NUL-terminated string in the global bus payload
 * arena. The terminating newline is included if present
 * (matches getline(3) convention); EOF without a trailing
 * newline returns whatever bytes were read. EOF with no
 * available bytes returns an empty string (the stable empty
 * sentinel) — callers paired with lotus_file_at_eof can
 * distinguish that from a genuine empty line. Read errors
 * also collapse to the empty-string sentinel; surfacing them
 * is a v1.x follow-up (the at_eof / read_line loop pattern
 * doesn't have a clean fallible attach point because of the
 * empty-line vs EOF ambiguity without a sum-type result).
 *
 * Capped at 8 MB per line; longer lines are truncated at the
 * cap and the caller sees the partial line (no error). The
 * truncation is deliberate over a stream-extension API for v1
 * simplicity — typical config / log lines are well under this. */
#define LOTUS_FILE_LINE_CAP (8u * 1024u * 1024u)

const char *lotus_file_read_line_global(int fd) {
    static const char empty[1] = { 0 };
    if (fd < 0) {
        errno = EINVAL;
        return empty;
    }
    /* Single-byte reads — getline(3) would be slicker but
     * requires a FILE* and we hold a raw fd. The kernel
     * read-ahead cache makes this not-terrible for typical
     * input sizes; if it shows up in a profile, swap to a
     * readbuf per-fd later. */
    char  stack_buf[4096];
    char *heap_buf = NULL;
    size_t cap = sizeof(stack_buf);
    size_t len = 0;
    char *buf = stack_buf;
    for (;;) {
        if (len >= LOTUS_FILE_LINE_CAP) {
            break;     /* truncate at cap */
        }
        if (len + 1 >= cap) {
            size_t new_cap = cap * 2;
            if (new_cap > LOTUS_FILE_LINE_CAP + 1) {
                new_cap = LOTUS_FILE_LINE_CAP + 1;
            }
            char *grown = (char *)realloc(
                (buf == stack_buf) ? NULL : heap_buf, new_cap);
            if (!grown) {
                if (heap_buf) free(heap_buf);
                return empty;
            }
            if (buf == stack_buf) {
                memcpy(grown, stack_buf, len);
            }
            heap_buf = grown;
            buf      = grown;
            cap      = new_cap;
        }
        char ch;
        ssize_t r = read(fd, &ch, 1);
        if (r > 0) {
            buf[len++] = ch;
            if (ch == '\n') break;
            continue;
        }
        if (r == 0) {
            /* EOF: if we have buffered bytes, return them as
             * the last (unterminated) line. If we have nothing,
             * return the stable empty-string sentinel. */
            if (len == 0) {
                if (heap_buf) free(heap_buf);
                return empty;
            }
            break;
        }
        if (errno == EINTR) continue;
        /* Read error: collapse to empty-string for the v1
         * non-fallible surface. Caller's at_eof() loop terminates
         * naturally because subsequent reads also hit EOF/error. */
        if (heap_buf) free(heap_buf);
        return empty;
    }
    buf[len] = '\0';
    /* Anchor result in the global bus payload arena so it
     * outlives the call frame AND the File locus's dissolve. */
    char *out = (char *)lotus_bus_payload_arena_alloc(len + 1, 1);
    if (!out) {
        if (heap_buf) free(heap_buf);
        return empty;
    }
    memcpy(out, buf, len + 1);
    if (heap_buf) free(heap_buf);
    return out;
}

/* Returns 1 if `fd` has no more bytes to read, 0 otherwise, -1
 * on error. Implemented via lseek(SEEK_CUR) vs lseek(SEEK_END)
 * comparison — works for regular files; for pipes / sockets the
 * function returns -1 with errno=ESPIPE (caller should not use
 * at_eof on non-seekable fds). */
int lotus_file_at_eof(int fd) {
    if (fd < 0) {
        errno = EINVAL;
        return -1;
    }
    off_t cur = lseek(fd, 0, SEEK_CUR);
    if (cur < 0) return -1;
    off_t end = lseek(fd, 0, SEEK_END);
    if (end < 0) return -1;
    /* Restore position. */
    if (lseek(fd, cur, SEEK_SET) < 0) return -1;
    return (cur >= end) ? 1 : 0;
}

/* Seek `fd` to absolute byte offset `offset`. Returns 0 on
 * success, -1 on error. */
int lotus_file_seek(int fd, int64_t offset) {
    if (fd < 0 || offset < 0) {
        errno = EINVAL;
        return -1;
    }
    return (lseek(fd, (off_t)offset, SEEK_SET) >= 0) ? 0 : -1;
}

/* Write all `len` bytes from `buf` to `fd`, looping over short
 * writes. Returns 0 on success, -1 on error. */
int lotus_file_write_all(int fd, const void *buf, size_t len) {
    if (fd < 0 || (!buf && len > 0)) {
        errno = EINVAL;
        return -1;
    }
    const char *p = (const char *)buf;
    size_t left = len;
    while (left > 0) {
        ssize_t w = write(fd, p, left);
        if (w > 0) {
            p    += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        return -1;
    }
    return 0;
}

/* Surface the current platform errno to the LLVM-side fallible-
 * dispatch wrappers. Each `std::io::fs::*` / `std::io::tcp::*`
 * primitive sets errno on failure; the codegen-side wrapper
 * reads it back via this helper and synthesizes an `IoError`
 * payload. Same global-state contract as POSIX itself — assumes
 * the wrapper calls this immediately after the failing call
 * with no intervening errno-setting syscalls. */
int32_t lotus_get_errno(void) {
    return (int32_t)errno;
}

/* Map a platform errno code to a stable kind-tag string the
 * IoError payload carries. Returns a pointer into a static
 * table; caller must not free. The kind taxonomy is the
 * agent-facing vocabulary — keep it small and intuitive.
 * Unmapped codes return "io" as the catch-all. */
const char *lotus_io_error_kind(int32_t errno_val) {
    switch (errno_val) {
        case 0:           return "";
        case ENOENT:      return "not_found";
        case EACCES:      return "permission_denied";
        case EPERM:       return "permission_denied";
        case EISDIR:      return "is_dir";
        case ENOTDIR:     return "not_dir";
        case EEXIST:      return "already_exists";
        case ENOTEMPTY:   return "not_empty";
        case ENOSPC:      return "no_space";
        case ENAMETOOLONG: return "name_too_long";
        case EINVAL:      return "invalid";
        case EAGAIN:      return "would_block";
#if defined(EWOULDBLOCK) && (EWOULDBLOCK != EAGAIN)
        case EWOULDBLOCK: return "would_block";
#endif
        case ETIMEDOUT:   return "timeout";
        case ECONNREFUSED: return "connection_refused";
        case ECONNRESET:  return "connection_reset";
        case ECONNABORTED: return "connection_aborted";
        case EHOSTUNREACH: return "host_unreachable";
        case ENETUNREACH: return "network_unreachable";
        case EADDRINUSE:  return "address_in_use";
        case EPIPE:       return "broken_pipe";
        case EINTR:       return "interrupted";
        /* C2 (pond/subprocess): subprocess-specific errnos. ESRCH is
         * "no such process" (typically from kill() against a pid
         * that has already been reaped). ECHILD is "no child
         * processes" (waitpid against a non-child). E2BIG surfaces
         * when argv is over the kernel limit at exec time. */
        case ESRCH:       return "not_found";
        case ECHILD:      return "not_found";
        case E2BIG:       return "invalid";
        default:          return "io";
    }
}

/* Locates the extension within `path` — including the leading
 * dot (".go", ".md") — or returns NULL when there is no
 * extension. The lookup operates on the basename: a dot inside
 * an earlier directory segment ("a.b/c") does NOT count as the
 * file's extension, and a leading-dot file (".bashrc",
 * "src/.config") has no extension by this rule. Mirrors the
 * conventional split used by Python's os.path.splitext and
 * Rust's Path::extension.
 *
 * Internal helper: the returned pointer (when non-NULL) aliases
 * `path`. External callers go through lotus_fs_extension_global,
 * which copies the slice into the program-lifetime payload arena
 * so the result is safe to stash past the call frame. */
static const char *lotus_fs_extension_locate(const char *path) {
    if (!path) return NULL;
    const char *base = path;
    for (const char *p = path; *p; p++) {
        if (*p == '/') base = p + 1;
    }
    const char *dot = NULL;
    for (const char *p = base; *p; p++) {
        if (*p == '.' && p != base) dot = p;
    }
    return dot;
}

/*
 * m77: process environment + argv access.
 *
 * Captures argc/argv in main's prelude (codegen emits a call
 * to lotus_env_init at the top of main, before any user code
 * runs) and exposes:
 *
 *   - args_count: argc
 *   - arg(i):     argv[i] for valid i, else stable empty string
 *   - var(name):  getenv(name) or stable empty string
 *   - var_exists: getenv(name) != NULL
 *
 * Hale Strings need NUL-terminated, pointer-stable buffers.
 * argv entries and getenv returns satisfy both (POSIX: argv
 * strings are NUL-terminated and live for main's lifetime;
 * getenv returns valid until a setenv/putenv we don't have a
 * surface for in v0). The empty-string sentinel is a single
 * NUL byte at static address — also pointer-stable for the
 * program's life.
 */
static int          g_argc       = 0;
static char *const *g_argv       = NULL;
static const char   g_empty_str[1] = { 0 };

void lotus_env_init(int argc, char *const *argv) {
    g_argc = argc;
    g_argv = argv;
    /* F.32-4c (2026-05-24): opt-in mlockall for latency-critical
     * programs. Set LOTUS_LOCK_MEMORY=1 in the environment to
     * lock all current + future pages and eliminate page-fault
     * stalls on hot-path arena allocation. HFT-grade processes
     * use this in concert with hugepages (F.32-4a) and pinned
     * placement.
     *
     * Prereqs: caller's RLIMIT_MEMLOCK must be high enough (or
     * root). Common shell: `ulimit -l unlimited` before invocation.
     * If mlockall fails (typically EPERM or ENOMEM), we print
     * a diagnostic to stderr and continue running unlocked —
     * the program still works, just without the stall-elimination
     * guarantee.
     *
     * Env-var surface (rather than a `runtime { lock_memory:
     * true }` block on `main locus`) keeps the choice at deploy
     * time where it belongs — the operator decides per-host
     * based on RLIMIT_MEMLOCK availability, without recompiling
     * the program. The language-surface variant is reserved for
     * a later F.32 ship if the env-var surface proves
     * insufficient. */
    const char *lock_mem_env = getenv("LOTUS_LOCK_MEMORY");
    if (lock_mem_env && (lock_mem_env[0] == '1' || lock_mem_env[0] == 't' || lock_mem_env[0] == 'T')) {
        if (mlockall(MCL_CURRENT | MCL_FUTURE) != 0) {
            fprintf(stderr,
                    "lotus: LOTUS_LOCK_MEMORY=%s requested but mlockall "
                    "failed (errno=%d %s). Continuing without memory locking; "
                    "increase RLIMIT_MEMLOCK (`ulimit -l unlimited`) or grant "
                    "CAP_IPC_LOCK to fix.\n",
                    lock_mem_env, errno, strerror(errno));
        }
    }
}

/*
 * 2026-05-17 — stdout buffering discipline.
 *
 * libc fully-buffers stdout when it isn't a TTY (pipes, files,
 * subprocess captures). That's wrong for Hale's contract:
 * `println("READY"); accept_blocking_call();` should make
 * "READY\n" visible immediately, not on accept's return — pipe
 * consumers (test oracles, supervisors waiting for a READY
 * handshake, log tailers) hang forever otherwise.
 *
 * Switch stdout to line-buffered globally so `\n`-terminated
 * `println` flushes on the newline regardless of how stdout is
 * connected. Matches Python's `python -u` discipline + Go's
 * default. Called once from main's prelude.
 *
 * stderr is already line-buffered per POSIX; we don't touch it.
 *
 * C2 (pond/subprocess) addendum: also ignore SIGPIPE globally so a
 * write to a closed pipe (the canonical hazard when a subprocess
 * exits before the parent finishes draining its stdin pipe) returns
 * EPIPE instead of killing the parent. Affects every Hale
 * program — but the contract Hale offers is "writes to broken
 * pipes return EPIPE via the IoError channel" not "the OS kills
 * your program when a stream closes", so the global flip is the
 * right default. Cost: programs that need SIGPIPE-driven termination
 * (rare) lose it.
 */
void lotus_io_init(void) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    /* SIG_IGN return is "previous action" — discarding it is fine;
     * the only way this fails is signum out of range, which can't
     * happen for SIGPIPE. */
    signal(SIGPIPE, SIG_IGN);
}

int lotus_env_args_count(void) {
    return g_argc;
}

const char *lotus_env_arg(int i) {
    if (i < 0 || i >= g_argc || !g_argv || !g_argv[i]) {
        return g_empty_str;
    }
    return g_argv[i];
}

const char *lotus_env_var(const char *name) {
    if (!name) return g_empty_str;
    const char *v = getenv(name);
    return v ? v : g_empty_str;
}

int lotus_env_var_exists(const char *name) {
    if (!name) return 0;
    return getenv(name) != NULL ? 1 : 0;
}

/*
 * Standard input — `std::io::stdin::read_line` substrate.
 *
 * Reads one line from stdin via POSIX getline(3) and copies the
 * content (with trailing newline stripped) into the lazy global
 * payload arena so the returned String is pointer-stable for
 * the program's lifetime. The libc getline buffer is freed
 * after the copy.
 *
 * Returns "" (the static empty-string sentinel) on EOF or
 * read error. Empty input lines (`\n` with no other content)
 * also return "" — the EOF-vs-empty-line collision is
 * documented in spec/stdlib.md; programs that need to
 * distinguish drive the read through a sibling status getter
 * (see lotus_stdin_read_line_status below).
 */
static int g_stdin_last_status = 0;
/*  0 = success (line was read; possibly empty)
 * -1 = EOF (no bytes read before EOF)
 * -2 = IO error (errno set; getline returned -1 with non-EOF)
 * -3 = OOM in payload arena (alloc returned NULL after a read)
 */

const char *lotus_stdin_read_line(void) {
    char *line = NULL;
    size_t cap = 0;
    errno = 0;
    ssize_t n = getline(&line, &cap, stdin);
    if (n < 0) {
        free(line);
        if (feof(stdin)) {
            g_stdin_last_status = -1;
        } else {
            g_stdin_last_status = -2;
        }
        return g_empty_str;
    }
    /* Strip the trailing '\n' (and optional '\r' before it) so
     * callers don't have to. getline preserves the newline; we
     * normalize here once. */
    if (n > 0 && line[n - 1] == '\n') {
        n--;
        if (n > 0 && line[n - 1] == '\r') {
            n--;
        }
    }
    char *out = (char *)lotus_bus_payload_arena_alloc((size_t)n + 1, 1);
    if (!out) {
        free(line);
        g_stdin_last_status = -3;
        return g_empty_str;
    }
    if (n > 0) {
        memcpy(out, line, (size_t)n);
    }
    out[n] = '\0';
    free(line);
    g_stdin_last_status = 0;
    return out;
}

/* Returns the status of the most recent lotus_stdin_read_line
 * call: 0 success, -1 EOF, -2 IO error, -3 OOM. Lets callers
 * distinguish "empty input line" (status 0, len 0) from "EOF"
 * (status -1, len 0). */
int lotus_stdin_read_line_status(void) {
    return g_stdin_last_status;
}

/*
 * m78: minimal string parsing primitives.
 *
 * Atoi-style: returns 0 when the input doesn't look like an
 * integer. Callers that need to distinguish "0" from "bad
 * input" probe with the boolean sibling. Implemented via
 * strtoll so leading whitespace and a leading sign are
 * accepted, but trailing garbage rejects (the strict shape).
 *
 * v0 scope: signed 64-bit integers in base 10. Hex / octal /
 * underscores wait on a richer parsing library. The
 * sufficient case for "parse a port from argv" is base 10
 * with optional leading minus.
 */

int64_t lotus_str_parse_int(const char *s) {
    if (!s || !*s) return 0;
    char *end = NULL;
    errno = 0;
    long long v = strtoll(s, &end, 10);
    if (errno != 0 || !end || *end != '\0') {
        return 0;
    }
    return (int64_t)v;
}

int lotus_str_can_parse_int(const char *s) {
    if (!s || !*s) return 0;
    char *end = NULL;
    errno = 0;
    (void)strtoll(s, &end, 10);
    if (errno != 0 || !end || *end != '\0') {
        return 0;
    }
    return 1;
}

/*
 * v1.x-16: parse_float / can_parse_float.
 * Strict trailing-NUL parse — empty / non-numeric / partial-tail
 * inputs return 0.0 and 0 respectively. Matches the parse_int
 * contract: a "soft" check function lets callers gate on
 * parseability and the parser returns 0 on failure for surface
 * code that wants a defaulting shape.
 */
double lotus_str_parse_float(const char *s) {
    if (!s || !*s) return 0.0;
    char *end = NULL;
    errno = 0;
    double v = strtod(s, &end);
    if (errno != 0 || !end || *end != '\0') {
        return 0.0;
    }
    return v;
}

int lotus_str_can_parse_float(const char *s) {
    if (!s || !*s) return 0;
    char *end = NULL;
    errno = 0;
    (void)strtod(s, &end);
    if (errno != 0 || !end || *end != '\0') {
        return 0;
    }
    return 1;
}

/*
 * Parse a Decimal literal source spelling into an i128 mantissa
 * with implicit scale 9. Mirrors parse_decimal_to_i128_scale9 in
 * codegen.rs — accepts optional trailing 'd', optional leading
 * sign, '_' digit separators, and truncates at exactly 9 fractional
 * digits (pads with zeros below 9). The i128 result is split into
 * two i64 halves (hi:lo) for ABI portability — same convention
 * used by lotus_decimal_to_string for the print path.
 *
 * On malformed / overflowed input, writes 0,0 and the can-variant
 * returns 0. Callers that need to distinguish "0.0" from "bad input"
 * gate on lotus_str_can_parse_decimal first.
 */
static int parse_decimal_to_i128(const char *s, __int128 *out) {
    if (!s) return 0;
    size_t n = strlen(s);
    if (n == 0) return 0;
    if (s[n - 1] == 'd') n -= 1;
    if (n == 0) return 0;
    size_t i = 0;
    __int128 sign = 1;
    if (s[i] == '-') { sign = -1; i += 1; }
    else if (s[i] == '+') { i += 1; }
    if (i >= n) return 0;
    __int128 mantissa = 0;
    int frac_digits = 0;
    int seen_dot = 0;
    int seen_digit = 0;
    for (; i < n; i++) {
        char c = s[i];
        if (c >= '0' && c <= '9') {
            seen_digit = 1;
            if (!seen_dot || frac_digits < 9) {
                __int128 prev = mantissa;
                mantissa = mantissa * 10 + (__int128)(c - '0');
                /* overflow guard: re-divide and compare */
                if (mantissa / 10 != prev) return 0;
                if (seen_dot) frac_digits += 1;
            }
        } else if (c == '.' && !seen_dot) {
            seen_dot = 1;
        } else if (c == '_') {
            /* digit separator */
        } else {
            return 0;
        }
    }
    if (!seen_digit) return 0;
    while (frac_digits < 9) {
        __int128 prev = mantissa;
        mantissa = mantissa * 10;
        if (mantissa / 10 != prev) return 0;
        frac_digits += 1;
    }
    *out = sign * mantissa;
    return 1;
}

void lotus_str_parse_decimal(const char *s, int64_t *out_hi, int64_t *out_lo) {
    __int128 m = 0;
    (void)parse_decimal_to_i128(s, &m);
    /* On failure m stays 0; the codegen guards on can_parse first. */
    *out_lo = (int64_t)(uint64_t)m;
    *out_hi = (int64_t)(m >> 64);
}

int lotus_str_can_parse_decimal(const char *s) {
    __int128 m = 0;
    return parse_decimal_to_i128(s, &m);
}

/*
 * 2026-05-26 — range-aware variants of the str helpers. These
 * accept a String + (start, end_exclusive) and operate only on
 * the byte range [start, end_exclusive), without requiring the
 * substring to be NUL-terminated. Used by std::str::range_eq /
 * range_parse_int / range_parse_decimal to enable allocation-
 * free JSON walks (the fathom workload identified
 * `iter_find_string_field` returning an owned String per field
 * lookup as the dominant arena-pressure source).
 *
 * Bounds: `start` must be >= 0; `end_exclusive` must be <=
 * strlen(s); end_exclusive < start returns the "empty range"
 * result (eq false, parse error). The caller (typically Hale
 * stdlib) is responsible for the bounds — these helpers do not
 * walk past `end_exclusive` even on malformed numeric input,
 * so a substring containing trailing garbage that's outside
 * the range doesn't reject the parse spuriously.
 */
int lotus_str_range_eq(const char *s, int64_t start, int64_t end_exclusive,
                       const char *t)
{
    if (!s || !t) return 0;
    if (start < 0 || end_exclusive < start) return 0;
    size_t n = (size_t)(end_exclusive - start);
    size_t tn = strlen(t);
    if (n != tn) return 0;
    return memcmp(s + start, t, n) == 0 ? 1 : 0;
}

/* Range-bounded int parse. Same shape as lotus_str_parse_int but
 * with explicit [start, end_exclusive) bounds; returns 0 on
 * empty / malformed / out-of-range — gate via the can-variant
 * for the strict path. */
static int64_t parse_int_in_range(const char *s, int64_t start,
                                   int64_t end_exclusive, int *ok)
{
    *ok = 0;
    if (!s || start < 0 || end_exclusive <= start) return 0;
    size_t n = (size_t)(end_exclusive - start);
    /* strtoll wants NUL-terminated input. Copy into a small
     * stack buffer; bound it at 64 bytes (longer than any
     * representable int64 in decimal — 19 digits + sign + NUL =
     * 21). Refuse longer than 63 chars. */
    if (n >= 64) return 0;
    char buf[64];
    memcpy(buf, s + start, n);
    buf[n] = '\0';
    char *end = NULL;
    errno = 0;
    long long v = strtoll(buf, &end, 10);
    if (errno != 0 || !end || *end != '\0') return 0;
    *ok = 1;
    return (int64_t)v;
}

int64_t lotus_str_parse_int_range(const char *s, int64_t start,
                                   int64_t end_exclusive)
{
    int ok = 0;
    return parse_int_in_range(s, start, end_exclusive, &ok);
}

int lotus_str_can_parse_int_range(const char *s, int64_t start,
                                   int64_t end_exclusive)
{
    int ok = 0;
    (void)parse_int_in_range(s, start, end_exclusive, &ok);
    return ok;
}

/* Range-bounded Decimal parse. Reuses parse_decimal_to_i128's
 * digit-walk by copying the substring into a small bounded
 * buffer first; Decimal source spellings are short
 * (mantissa + optional sign + dot + optional 'd' suffix), so a
 * 128-byte stack buffer covers any realistic input. */
static int parse_decimal_to_i128_range(const char *s, int64_t start,
                                        int64_t end_exclusive,
                                        __int128 *out)
{
    if (!s || start < 0 || end_exclusive <= start) return 0;
    size_t n = (size_t)(end_exclusive - start);
    if (n >= 128) return 0;
    char buf[128];
    memcpy(buf, s + start, n);
    buf[n] = '\0';
    return parse_decimal_to_i128(buf, out);
}

void lotus_str_parse_decimal_range(const char *s, int64_t start,
                                    int64_t end_exclusive,
                                    int64_t *out_hi, int64_t *out_lo)
{
    __int128 m = 0;
    (void)parse_decimal_to_i128_range(s, start, end_exclusive, &m);
    *out_lo = (int64_t)(uint64_t)m;
    *out_hi = (int64_t)(m >> 64);
}

int lotus_str_can_parse_decimal_range(const char *s, int64_t start,
                                       int64_t end_exclusive)
{
    __int128 m = 0;
    return parse_decimal_to_i128_range(s, start, end_exclusive, &m);
}

/*
 * m58: deployment-config subject binding.
 *
 * Layered on top of the m57 AF_UNIX transport: a startup config
 * file maps each `bus subscribe` / publish subject to a transport
 * URL (currently only `unix://<path>`). Source stays transport-
 * agnostic per notes/open-questions #8 — the binding lives
 * entirely in deployment-config.
 *
 * Codegen emits one call to lotus_bus_load_config in main's
 * prelude:
 *
 *     lotus_bus_load_config(getenv("LOTUS_BUS_CONFIG"));
 *
 * If the env var is unset (or the file is unreadable),
 * lotus_bus_load_config no-ops and the binary behaves as a
 * single-process program — matches the m45-followup baseline so
 * existing examples are unaffected.
 *
 * Wire format: one entry per line, `subject=url:role`. Comments
 * begin with '#' and run to end-of-line. Whitespace is trimmed
 * around all three tokens. role is `listen` or `connect`. The
 * role is per-binary, per-subject — two binaries on the same
 * subject must declare opposite roles in their respective configs.
 *
 * v0.1 supports CONNECT-side dispatch only: a publisher with a
 * CONNECT-role binding fans out via lotus_transport_send during
 * lotus_bus_dispatch. LISTEN-side accept-and-spawn-reader-thread
 * is m59+ — at this milestone the listener role is exercised by
 * the m57 transport_driver harness so the full publisher pipeline
 * can be verified end-to-end without yet wiring receive-side
 * dispatch. v0.1 also supports exactly one peer per subject; the
 * fanout cardinality story (multi-peer per subject, multi-subject
 * per peer) is m60.
 */

/* Wave B (bus-transport redesign): an entry is one of three kinds.
 * UNIX = substrate-provided AF_UNIX transport; ADAPTER = user-
 * supplied protocol-layer locus (NATS, MQTT, raw-TCP-with-framing,
 * ...) whose `send` method receives outbound payloads;
 * UDP (2026-05-26) = unified IPv4 UDP transport that covers both
 * unicast and multicast — the destination address determines which
 * mode (224.0.0.0/4 → multicast, else unicast). */
#define LOTUS_BUS_REMOTE_KIND_UNIX    0
#define LOTUS_BUS_REMOTE_KIND_ADAPTER 1
#define LOTUS_BUS_REMOTE_KIND_UDP     2

typedef struct lotus_bus_remote_entry {
    char              *subject;       /* owned (strdup'd at register) */
    int                kind;          /* one of LOTUS_BUS_REMOTE_KIND_* */
    /* --- UNIX fields (valid when kind == UNIX) --- */
    lotus_transport_t *transport;     /* set in main for CONNECT,
                                         in reader-thread for LISTEN */
    int                role;
    /* m59: per-subject reader thread for LISTEN role. Set when the
     * pthread is spawned at register time; the thread loops on
     * lotus_transport_recv and dispatches to local subscribers via
     * lotus_bus_local_dispatch. CONNECT-role entries leave both
     * fields zero (no thread, transport opened on the main path). */
    pthread_t          reader_thread;
    int                has_reader_thread;
    /* --- ADAPTER fields (valid when kind == ADAPTER) --- */
    /* `adapter_self` is the adapter locus's self pointer; held
     * in the program-lifetime payload arena by codegen so it
     * outlives main. `adapter_send_fn` is the address of the
     * locus's `send(subject: String, bytes: Bytes)` method,
     * resolved at codegen time. The runtime invokes it directly
     * without going through the F.20 vtable. */
    void              *adapter_self;
    void             (*adapter_send_fn)(void *self,
                                        const char *subject,
                                        void *bytes);
    /* --- UDP fields (valid when kind == UDP) ---
     * 2026-05-26: unified IPv4 UDP transport. `udp_fd` is the
     * socket used for BOTH sendto (CONNECT role) and recvfrom
     * (LISTEN role). `udp_dest` is filled at register time for
     * CONNECT entries; LISTEN entries leave it zeroed (the
     * reader thread's recvfrom captures the source per-datagram
     * if a caller needs it). `udp_is_multicast` records whether
     * the destination falls in 224.0.0.0/4 — drives
     * IP_ADD_MEMBERSHIP on LISTEN side and IP_DROP_MEMBERSHIP
     * on destroy. */
    int                udp_fd;
    struct sockaddr_in udp_dest;
    int                udp_is_multicast;
    /* F.36 Slice 3 (2026-05-28): pluggable codec hooks. Set by
     * `lotus_bus_register_codec` when the binding entry carried
     * a `codec(L { ... })` clause. The encode hook converts a
     * value pointer (T's struct layout) to a Bytes blob; the
     * decode hook converts wire-bytes back to T. Both run on
     * whatever thread invokes them — the codec's purity is
     * enforced at typecheck (F.36 Slice 2), so no coordination
     * is in scope here.
     *
     * Encode/decode signatures match the codegen-emitted method
     * shape: encode takes (codec_self, value_ptr) returning a
     * Bytes-blob ptr; decode takes (codec_self, bytes_ptr)
     * returning a freshly-allocated value-of-T (heap-bearing
     * via the payload arena). When NULL, dispatch falls back to
     * the m70 default serializer / deserializer. */
    void              *codec_self;
    void             *(*codec_encode_fn)(void *self, void *value);
    void             *(*codec_decode_fn)(void *self, void *bytes);
} lotus_bus_remote_entry_t;

/* 2026-05-27 — array-of-pointers shape (was array-of-structs).
 * Each entry is its own malloc'd allocation; the global array
 * holds pointers to them and may realloc freely without
 * invalidating the per-entry addresses.
 *
 * Why this shape: the udp listen-side reader threads hold a
 * stable `entry` pointer captured at spawn time (see
 * lotus_bus_udp_reader_args_t.entry). With the array-of-structs
 * shape, a subsequent register_remote call that grew the array
 * could realloc-move the storage and dangle every previously-
 * spawned reader thread's `entry` pointer. Fathom hit this when
 * priceview's LOTUS_BUS_CONFIG mixed 4 listens + 2 connects:
 * the 5th entry's realloc invalidated the first 4 reader
 * threads' entry pointers, segfaulting silently on the first
 * inbound datagram. Single-role configs (listen-only or
 * connect-only) that fit in the initial cap (=4) never tripped
 * the realloc, which is why the crash matrix appeared
 * "listen+connect specific" rather than "more than 4 entries."
 *
 * Cost: one extra pointer dereference per fanout step; one
 * extra malloc per registration. Both are noise in the
 * shapes that touch this code (startup + low-rate publishes). */
static lotus_bus_remote_entry_t **g_bus_remote_entries = NULL;
static size_t g_bus_remote_count = 0;
static size_t g_bus_remote_cap   = 0;

static inline int lotus_bus_has_remote_entries(void) {
    return g_bus_remote_count > 0;
}

#define LOTUS_BUS_REMOTE_INITIAL_CAP 4

/* m59: queue pointer published by the codegen prelude (via
 * lotus_bus_set_queue) so reader threads can dispatch into the
 * cooperative-subscriber path without plumbing the queue through
 * the transport layer. NULL until the codegen prelude runs;
 * reader threads handle the NULL case by skipping cooperative
 * dispatch (pinned subscribers via mailbox always work). */
static lotus_bus_queue_t *g_bus_queue_for_remote = NULL;

void lotus_bus_set_queue(lotus_bus_queue_t *queue) {
    g_bus_queue_for_remote = queue;
}

/* m59: reader-thread args. Owns the path string so the thread
 * can outlive the lotus_bus_register_remote call. The entry
 * back-reference lets the thread publish its transport ptr to
 * the entry so lotus_bus_remote_destroy_all can find it. */
typedef struct lotus_bus_reader_args {
    char                     *path;       /* owned by the thread */
    lotus_bus_remote_entry_t *entry;
} lotus_bus_reader_args_t;

static void *lotus_bus_reader_thread_main(void *arg) {
    lotus_bus_reader_args_t *args = (lotus_bus_reader_args_t *)arg;
    /* Open the LISTEN transport HERE, on the reader thread, so
     * accept() blocks the reader thread instead of main's boot
     * path. m58 opened transports inline in register_remote which
     * meant a subscriber binary would hang at startup until the
     * publisher connected; m59 defers the accept off the boot
     * path so main proceeds and any local-subscribe registration
     * can complete before we wait for a peer. */
    lotus_transport_t *t = lotus_transport_create(
        args->path, LOTUS_TRANSPORT_LISTEN);
    if (!t) {
        free(args->path);
        free(args);
        return NULL;
    }
    /* Publish the transport pointer back to the entry so
     * lotus_bus_remote_destroy_all can shutdown(2) the connection
     * if a clean teardown is needed. (Race: between accept
     * returning and this store, destroy_all sees NULL and skips
     * the shutdown — that's fine because in well-formed test
     * scenarios destroy_all runs after the peer has closed,
     * which already drives recv to EOF.) */
    args->entry->transport = t;

    char wire_buf[LOTUS_PAYLOAD_MAX];
    char struct_buf[LOTUS_PAYLOAD_MAX];
    while (1) {
        ssize_t n = lotus_transport_recv(t, wire_buf, sizeof(wire_buf));
        if (n <= 0) break;     /* peer closed (0) or error (-1) */

        /* m60: deserialize wire bytes into struct-layout bytes
         * before handing them to local dispatch. Look up the
         * deserialize_fn from the FIRST local entry matching
         * this subject — by language constraint all entries on
         * the same subject share the payload type, so any one
         * works. Skip dispatch if the type-checker mismatches
         * or there are no local subscribers (the recv'd bytes
         * have nowhere to go locally; that's not an error in
         * relay-shaped programs). */
        lotus_deserialize_fn deserialize = NULL;
        for (size_t i = 0; i < g_bus_count; i++) {
            lotus_bus_entry_t *e = &g_bus_entries[i];
            if (!e->subject) continue;
            /* m94: wildcard locals (e.g. "log.**") need to match
             * concrete remote-bound subjects too, so use the same
             * pattern-matching as the dispatch path. By language
             * constraint, all subscribers on the same subject
             * share the payload type, so the deserialize_fn from
             * any matching entry is the right one. */
            if (!lotus_subject_match(e->subject, args->entry->subject)) continue;
            deserialize = e->deserialize;
            break;
        }
        if (!deserialize) continue;
        ssize_t struct_size = deserialize(
            wire_buf, (size_t)n, struct_buf, sizeof(struct_buf));
        if (struct_size <= 0) continue;
        lotus_bus_local_dispatch(g_bus_queue_for_remote,
                                 args->entry->subject,
                                 struct_buf, (size_t)struct_size);
    }

    lotus_transport_destroy(t);
    args->entry->transport = NULL;     /* prevent double-destroy */
    free(args->path);
    free(args);
    return NULL;
}

/* 2026-05-26: UDP bus-transport helpers. The scheme `udp://host:port`
 * covers both unicast and multicast — the host's address class
 * picks which: 224.0.0.0/4 → multicast (kernel routes via the
 * multicast tree on send; subscribers must IP_ADD_MEMBERSHIP at
 * bind time to receive), anything else → unicast (kernel routes
 * normally; subscribers just bind).
 *
 * The publisher's `sendto` call is identical for both — the kernel
 * inspects the destination and picks the route. The subscriber-
 * side bind is identical except multicast also joins the group.
 *
 * Wire frame: same as the existing AF_UNIX transport — one
 * datagram per message, opaque bytes that the bus router hands
 * to the registered deserialize_fn. UDP's native datagram
 * boundaries match SEQPACKET's, so no length prefix needed.
 * The receiver-side wire buffer is sized at LOTUS_PAYLOAD_MAX
 * (64 KB), which covers the UDP datagram max. The kernel-side
 * SO_RCVBUF defaults to the system value (~208 KB on Linux);
 * tune higher for bursty receivers via LOTUS_BUS_UDP_RCVBUF. */

/* 2026-05-27: dedupe-throttled stderr log for fanout `sendto`
 * failures. Without this the runtime swallows the error
 * silently; with it the first occurrence of each errno class
 * surfaces a one-line diagnostic. The most common cases for
 * UDP fanout are EMSGSIZE (payload exceeds path MTU + DF
 * set — see IP_MTU_DISCOVER), ENOBUFS (kernel send buffer
 * full — bump SO_SNDBUF), and EAGAIN (rare for blocking UDP
 * sockets; would indicate something pathological).
 *
 * Per-errno dedupe so a real fault doesn't flood the log;
 * tiny static table is fine — there are only a handful of
 * errno values sendto actually returns. Thread-safety:
 * mutex around the table; the contention is unmeasurable
 * because hitting this path means delivery is already
 * failing. */
static pthread_mutex_t g_udp_sendto_err_lock = PTHREAD_MUTEX_INITIALIZER;
static int g_udp_sendto_err_seen[8] = {0};
static int g_udp_sendto_err_count   = 0;

static void lotus_bus_udp_log_sendto_error(const char *subject, int err) {
    pthread_mutex_lock(&g_udp_sendto_err_lock);
    for (int i = 0; i < g_udp_sendto_err_count; i++) {
        if (g_udp_sendto_err_seen[i] == err) {
            pthread_mutex_unlock(&g_udp_sendto_err_lock);
            return;
        }
    }
    if (g_udp_sendto_err_count <
        (int)(sizeof(g_udp_sendto_err_seen)
              / sizeof(g_udp_sendto_err_seen[0]))) {
        g_udp_sendto_err_seen[g_udp_sendto_err_count++] = err;
    }
    pthread_mutex_unlock(&g_udp_sendto_err_lock);
    fprintf(stderr,
            "[bus] udp sendto failed (subject=\"%s\", errno=%d/%s) "
            "— logged once per errno; check path MTU "
            "(EMSGSIZE → reduce payload or enable jumbo / "
            "IP_MTU_DISCOVER=IP_PMTUDISC_DONT) or kernel send "
            "buffer (ENOBUFS → bump SO_SNDBUF)\n",
            subject ? subject : "?", err,
            strerror(err) ? strerror(err) : "?");
}

/* 2026-05-27: SO_RCVBUF size for UDP reader sockets. Env-
 * configurable so jumbo / bursty receivers can grow the kernel-
 * side receive queue without code changes. Returns 0 if unset
 * or invalid (caller skips setsockopt). */
static int lotus_bus_udp_rcvbuf_env(void) {
    const char *s = getenv("LOTUS_BUS_UDP_RCVBUF");
    if (!s || !*s) return 0;
    long n = strtol(s, NULL, 10);
    if (n <= 0 || n > INT_MAX) return 0;
    return (int)n;
}
static int lotus_addr_is_multicast(const struct in_addr *a) {
    uint32_t h = ntohl(a->s_addr);
    return (h >= 0xE0000000u) && (h < 0xF0000000u);
}

/* Parse "host:port" into a sockaddr_in. Returns 0 on success, -1
 * on malformed input (sets errno on the standard parses). Host
 * must be a dotted-quad IPv4; hostname resolution is the caller's
 * job (the bus router runs on the boot path and shouldn't block
 * on DNS — explicit IPs match the spec/runtime.md transport
 * surface contract). */
static int lotus_bus_parse_udp_addr(const char *spec,
                                     struct sockaddr_in *out)
{
    if (!spec || !out) { errno = EINVAL; return -1; }
    const char *colon = strrchr(spec, ':');
    if (!colon || colon == spec) { errno = EINVAL; return -1; }
    char host[64];
    size_t host_len = (size_t)(colon - spec);
    if (host_len >= sizeof(host)) { errno = EINVAL; return -1; }
    memcpy(host, spec, host_len);
    host[host_len] = '\0';
    char *end = NULL;
    long port = strtol(colon + 1, &end, 10);
    if (!end || *end != '\0' || port <= 0 || port > 65535) {
        errno = EINVAL;
        return -1;
    }
    memset(out, 0, sizeof(*out));
    out->sin_family = AF_INET;
    out->sin_port   = htons((uint16_t)port);
    if (inet_pton(AF_INET, host, &out->sin_addr) != 1) {
        errno = EINVAL;
        return -1;
    }
    return 0;
}

/* UDP LISTEN reader thread. Binds the socket (joins the multicast
 * group if applicable), then loops recvfrom + deserialize +
 * local_dispatch — same shape as the unix:// reader thread. */
typedef struct lotus_bus_udp_reader_args {
    char                     *host_port;  /* owned by thread */
    lotus_bus_remote_entry_t *entry;
} lotus_bus_udp_reader_args_t;

static void *lotus_bus_udp_reader_thread_main(void *arg) {
    lotus_bus_udp_reader_args_t *args = (lotus_bus_udp_reader_args_t *)arg;
    struct sockaddr_in dest;
    if (lotus_bus_parse_udp_addr(args->host_port, &dest) != 0) {
        fprintf(stderr,
                "lotus_bus udp reader: invalid host:port %s\n",
                args->host_port);
        free(args->host_port);
        free(args);
        return NULL;
    }
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (fd < 0) {
        perror("lotus_bus udp reader: socket");
        free(args->host_port);
        free(args);
        return NULL;
    }
    int one = 1;
    /* SO_REUSEADDR so multiple subscribers on the same multicast
     * group can bind the same port. Required for multicast;
     * harmless for unicast. */
    (void)setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    /* SO_REUSEPORT (Linux 3.9+) — required on modern kernels for
     * the multi-subscriber-same-host pattern when multiple
     * processes want to receive the same multicast group. Without
     * this, kernel-level load-balancing across the bound sockets
     * may not engage. Best-effort: some kernels lack the option,
     * we ignore the failure. */
#ifdef SO_REUSEPORT
    (void)setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &one, sizeof(one));
#endif
    /* SO_RCVBUF override (2026-05-27). Kernel default (~208 KB
     * on Linux) is fine for most receivers; bursty / jumbo /
     * high-fanout cases can lift this via LOTUS_BUS_UDP_RCVBUF
     * (bytes). Best-effort: kernel caps the value at
     * net.core.rmem_max — failure is silent because there's no
     * recovery path. */
    int rcvbuf = lotus_bus_udp_rcvbuf_env();
    if (rcvbuf > 0) {
        (void)setsockopt(fd, SOL_SOCKET, SO_RCVBUF,
                         &rcvbuf, sizeof(rcvbuf));
    }
    struct sockaddr_in bind_addr;
    memset(&bind_addr, 0, sizeof(bind_addr));
    bind_addr.sin_family = AF_INET;
    bind_addr.sin_port   = dest.sin_port;
    /* Bind to the configured address regardless of unicast /
     * multicast. The bind tuple is what Linux uses to filter
     * which incoming datagrams reach this socket; the
     * IP_ADD_MEMBERSHIP call below (multicast case) is a
     * separate concern controlling whether the kernel
     * forwards the group's traffic up the IP stack at all.
     *
     * Pre-2026-05-27 the multicast branch bound INADDR_ANY,
     * which works for a single receiver but crosstalks with
     * any other socket bound to the same port: every joined-
     * group packet on that port lands on every wildcard-bound
     * socket. Surfaced by fathom's priceview, which subscribes
     * to four distinct multicast groups sharing port 5000 —
     * each (group, port) is logically a distinct endpoint
     * carrying its own payload type, but the wildcard bind
     * collapsed them into a single delivery set so every
     * datagram fanned to all four handlers (kraken+coinbase
     * gauges converged to whichever venue's snapshot arrived
     * last). Matches the reference multicast-receiver shape
     * in Boost.Asio's `multicast::join_group` example and
     * NASDAQ ITCH-Recovery client samples. */
    bind_addr.sin_addr = dest.sin_addr;
    if (bind(fd, (struct sockaddr *)&bind_addr, sizeof(bind_addr)) < 0) {
        perror("lotus_bus udp reader: bind");
        close(fd);
        free(args->host_port);
        free(args);
        return NULL;
    }
    if (lotus_addr_is_multicast(&dest.sin_addr)) {
        struct ip_mreq mreq;
        memset(&mreq, 0, sizeof(mreq));
        mreq.imr_multiaddr     = dest.sin_addr;
        mreq.imr_interface.s_addr = htonl(INADDR_ANY);
        if (setsockopt(fd, IPPROTO_IP, IP_ADD_MEMBERSHIP,
                       &mreq, sizeof(mreq)) < 0) {
            perror("lotus_bus udp reader: IP_ADD_MEMBERSHIP");
            close(fd);
            free(args->host_port);
            free(args);
            return NULL;
        }
        args->entry->udp_is_multicast = 1;
    }
    /* Publish the fd back to the entry so destroy_all can close
     * + leave-group on teardown. */
    args->entry->udp_fd   = fd;
    args->entry->udp_dest = dest;

    char wire_buf[LOTUS_PAYLOAD_MAX];
    char struct_buf[LOTUS_PAYLOAD_MAX];
    while (1) {
        ssize_t n = recvfrom(fd, wire_buf, sizeof(wire_buf), 0, NULL, NULL);
        if (n < 0) {
            if (errno == EINTR) continue;
            break;  /* EBADF / ENOTCONN / shutdown-induced */
        }
        if (n == 0) {
            /* Two cases:
             *   - Legitimate zero-length datagram. Not meaningful
             *     for the bus router; the wire frame can never be
             *     empty (it's the serialized form of the user's
             *     type, which has at least the header bytes).
             *   - Socket has been shut down by destroy_all
             *     (shutdown(SHUT_RDWR)).
             * Either way, exit the loop. */
            break;
        }
        lotus_deserialize_fn deserialize = NULL;
        for (size_t i = 0; i < g_bus_count; i++) {
            lotus_bus_entry_t *e = &g_bus_entries[i];
            if (!e->subject) continue;
            if (!lotus_subject_match(e->subject, args->entry->subject)) continue;
            deserialize = e->deserialize;
            break;
        }
        if (!deserialize) {
            /* 2026-05-28: the silent-skip-on-no-deserializer path.
             * If LOTUS_BUS_LOG_DESERIALIZE_DROP=1, emit one line
             * naming the subject + payload size so bring-up bisects
             * have something to grep for instead of "no log
             * messages at all". Three udp:// bring-ups this week
             * burned hours on this drop class. See the
             * `handoff-compiler-refdata-dispatch-silent-...` brief. */
            if (lotus_bus_log_deserialize_drop_enabled()) {
                dprintf(2,
                        "lotus_bus udp reader: drop on `%s` "
                        "(no deserializer registered) — %zd-byte payload\n",
                        args->entry->subject, n);
            }
            continue;
        }
        ssize_t struct_size = deserialize(
            wire_buf, (size_t)n, struct_buf, sizeof(struct_buf));
        if (struct_size <= 0) {
            if (lotus_bus_log_deserialize_drop_enabled()) {
                dprintf(2,
                        "lotus_bus udp reader: drop on `%s` "
                        "(deserialize returned %zd) — %zd-byte wire payload\n",
                        args->entry->subject, struct_size, n);
            }
            continue;
        }
        lotus_bus_local_dispatch(g_bus_queue_for_remote,
                                 args->entry->subject,
                                 struct_buf, (size_t)struct_size);
    }
    free(args->host_port);
    free(args);
    return NULL;
}

void lotus_bus_register_remote(const char *subject,
                               const char *url,
                               int role) {
    if (!subject || !url) {
        fprintf(stderr,
                "lotus_bus_register_remote: null subject or url\n");
        return;
    }
    /* Recognized schemes:
     *   unix://<path>          AF_UNIX SEQPACKET (m58)
     *   udp://<ipv4>:<port>    IPv4 UDP, unicast or multicast
     *                          (multicast detected from address
     *                          class) — added 2026-05-26.
     * User-supplied protocol-layer transports come in via
     * lotus_bus_register_remote_adapter (Wave B). */
    static const char unix_scheme[] = "unix://";
    static const char udp_scheme[]  = "udp://";
    size_t unix_len = sizeof(unix_scheme) - 1;
    size_t udp_len  = sizeof(udp_scheme) - 1;
    int is_udp = 0;
    const char *path = NULL;
    if (strncmp(url, unix_scheme, unix_len) == 0) {
        path = url + unix_len;
    } else if (strncmp(url, udp_scheme, udp_len) == 0) {
        is_udp = 1;
        path   = url + udp_len;
    } else {
        fprintf(stderr,
                "lotus_bus_register_remote: unsupported URL scheme "
                "(recognize unix:// and udp://): %s\n",
                url);
        return;
    }
    if (*path == '\0') {
        fprintf(stderr,
                "lotus_bus_register_remote: empty path in %s\n", url);
        return;
    }

    /* Grow the slot-array (of pointers, post-2026-05-27) so we
     * have room to stash the new entry's pointer. The
     * realloc here can move the array of pointers, but the
     * individual entries it points to are stable malloc'd
     * allocations — no dangling reader-thread `args->entry`
     * pointers from prior registrations. */
    if (g_bus_remote_count == g_bus_remote_cap) {
        size_t new_cap = g_bus_remote_cap == 0
            ? LOTUS_BUS_REMOTE_INITIAL_CAP
            : g_bus_remote_cap * 2;
        lotus_bus_remote_entry_t **grown = (lotus_bus_remote_entry_t **)
            realloc(g_bus_remote_entries,
                    new_cap * sizeof(lotus_bus_remote_entry_t *));
        if (!grown) return;
        g_bus_remote_entries = grown;
        g_bus_remote_cap     = new_cap;
    }

    char *subject_copy = strdup(subject);
    if (!subject_copy) return;

    lotus_bus_remote_entry_t *e = (lotus_bus_remote_entry_t *)
        malloc(sizeof(lotus_bus_remote_entry_t));
    if (!e) { free(subject_copy); return; }
    g_bus_remote_entries[g_bus_remote_count++] = e;
    e->subject           = subject_copy;
    e->kind              = is_udp
        ? LOTUS_BUS_REMOTE_KIND_UDP
        : LOTUS_BUS_REMOTE_KIND_UNIX;
    e->transport         = NULL;
    e->role              = role;
    e->has_reader_thread = 0;
    e->adapter_self      = NULL;
    e->adapter_send_fn   = NULL;
    e->udp_fd            = -1;
    memset(&e->udp_dest, 0, sizeof(e->udp_dest));
    e->udp_is_multicast  = 0;
    /* F.36 Slice 3: codec fields default NULL (m70 path). The
     * register_codec call below zero-or-overwrites them. */
    e->codec_self        = NULL;
    e->codec_encode_fn   = NULL;
    e->codec_decode_fn   = NULL;

    if (is_udp) {
        if (role == LOTUS_TRANSPORT_LISTEN) {
            /* LISTEN: spawn reader thread that owns the bind +
             * multicast-group-join + recvfrom loop. Same shape
             * as the unix:// reader thread but UDP semantics. */
            lotus_bus_udp_reader_args_t *args =
                (lotus_bus_udp_reader_args_t *)malloc(sizeof(*args));
            if (!args) return;
            args->host_port = strdup(path);
            args->entry     = e;
            if (!args->host_port) {
                free(args);
                return;
            }
            /* Reader thread dispatches handlers (which open scratch
             * subregions) concurrently with main → subregion latch. */
            lotus_mark_multithreaded();
            if (pthread_create(&e->reader_thread, NULL,
                               lotus_bus_udp_reader_thread_main, args) != 0)
            {
                perror("lotus_bus_register_remote: udp pthread_create");
                free(args->host_port);
                free(args);
                return;
            }
            e->has_reader_thread = 1;
        } else {
            /* CONNECT: open a UDP socket bound to an ephemeral
             * local port; resolve the destination addr at
             * register time. Per-publish dispatch does
             * sendto(fd, payload, udp_dest). */
            if (lotus_bus_parse_udp_addr(path, &e->udp_dest) != 0) {
                fprintf(stderr,
                        "lotus_bus_register_remote: invalid udp:// "
                        "host:port %s\n", path);
                return;
            }
            int fd = socket(AF_INET, SOCK_DGRAM, 0);
            if (fd < 0) {
                perror("lotus_bus_register_remote: udp socket");
                return;
            }
            /* Bind to an ephemeral local port so the kernel picks
             * a source addr for our sendto's. INADDR_ANY lets the
             * kernel select the route-appropriate interface. */
            struct sockaddr_in any;
            memset(&any, 0, sizeof(any));
            any.sin_family      = AF_INET;
            any.sin_addr.s_addr = htonl(INADDR_ANY);
            any.sin_port        = 0;
            if (bind(fd, (struct sockaddr *)&any, sizeof(any)) < 0) {
                perror("lotus_bus_register_remote: udp bind");
                close(fd);
                return;
            }
            e->udp_fd           = fd;
            e->udp_is_multicast = lotus_addr_is_multicast(&e->udp_dest.sin_addr);
        }
        return;
    }

    if (role == LOTUS_TRANSPORT_LISTEN) {
        /* m59: spawn a reader thread that owns this subject's
         * recv loop. The thread opens the LISTEN transport on
         * its own stack so accept() doesn't block the main
         * thread. */
        lotus_bus_reader_args_t *args =
            (lotus_bus_reader_args_t *)malloc(sizeof(*args));
        if (!args) return;
        args->path  = strdup(path);
        args->entry = e;
        if (!args->path) {
            free(args);
            return;
        }
        /* Reader thread dispatches handlers (which open scratch
         * subregions) concurrently with main → subregion latch. */
        lotus_mark_multithreaded();
        if (pthread_create(&e->reader_thread, NULL,
                           lotus_bus_reader_thread_main, args) != 0) {
            perror("lotus_bus_register_remote: pthread_create");
            free(args->path);
            free(args);
            return;
        }
        e->has_reader_thread = 1;
    } else {
        /* CONNECT: open inline so the connect-with-retry happens
         * on the boot path. The first publish on this subject
         * fans out through the resulting transport. */
        e->transport = lotus_transport_create(path, role);
        /* On failure lotus_transport_create already perror'd; the
         * entry stays in the table with transport=NULL so fanout
         * skips it and destroy_all is a no-op for this slot. */
    }
}

/* Wave B: register an adapter binding. The adapter locus has
 * already been instantiated by codegen with program-lifetime
 * allocation, and its `send(subject, bytes)` method's fn pointer
 * has been resolved. Outbound fanout to this subject invokes
 * `send_fn(self_data, subject_c_str, bytes_struct)` with a Bytes
 * value built from the local payload via lotus_bytes_from_buf
 * against the lazy global payload arena.
 *
 * Adapter entries don't open a transport or spawn a reader thread
 * — the adapter locus itself owns its protocol lifecycle through
 * its own birth/dissolve methods. destroy_all is a no-op for
 * adapter slots beyond freeing the subject string.
 */
void lotus_bus_register_remote_adapter(
    const char *subject,
    void *self_data,
    void (*send_fn)(void *self,
                    const char *subject,
                    void *bytes))
{
    if (!subject || !self_data || !send_fn) {
        fprintf(stderr,
                "lotus_bus_register_remote_adapter: null subject, "
                "self_data, or send_fn\n");
        return;
    }
    if (g_bus_remote_count == g_bus_remote_cap) {
        size_t new_cap = g_bus_remote_cap == 0
            ? LOTUS_BUS_REMOTE_INITIAL_CAP
            : g_bus_remote_cap * 2;
        lotus_bus_remote_entry_t **grown = (lotus_bus_remote_entry_t **)
            realloc(g_bus_remote_entries,
                    new_cap * sizeof(lotus_bus_remote_entry_t *));
        if (!grown) return;
        g_bus_remote_entries = grown;
        g_bus_remote_cap     = new_cap;
    }
    char *subject_copy = strdup(subject);
    if (!subject_copy) return;
    lotus_bus_remote_entry_t *e = (lotus_bus_remote_entry_t *)
        malloc(sizeof(lotus_bus_remote_entry_t));
    if (!e) { free(subject_copy); return; }
    g_bus_remote_entries[g_bus_remote_count++] = e;
    e->subject           = subject_copy;
    e->kind              = LOTUS_BUS_REMOTE_KIND_ADAPTER;
    e->transport         = NULL;
    e->role              = 0;
    e->has_reader_thread = 0;
    e->adapter_self      = self_data;
    e->adapter_send_fn   = send_fn;
    e->udp_fd            = -1;
    memset(&e->udp_dest, 0, sizeof(e->udp_dest));
    e->udp_is_multicast  = 0;
    e->codec_self        = NULL;
    e->codec_encode_fn   = NULL;
    e->codec_decode_fn   = NULL;
}

/* F.36 Slice 3 (2026-05-28): attach a pluggable codec to an
 * already-registered remote binding. Looks up the entry by
 * subject + patches the codec fields. The codec's encode is
 * called on the publish-side dispatch path (before transport
 * send); decode is called on the receive-side reader thread
 * (instead of the m70 __deserialize_T). Both run on whatever
 * thread invokes them; the codec's purity is enforced at
 * typecheck (F.36 Slice 2), so no coordination is in scope.
 *
 * Idempotent on re-register; the codegen emits one call per
 * binding entry that carries a codec clause. */
void lotus_bus_register_codec(
    const char *subject,
    void *codec_self,
    void *(*encode_fn)(void *self, void *value),
    void *(*decode_fn)(void *self, void *bytes))
{
    if (!subject || !codec_self || !encode_fn || !decode_fn) {
        fprintf(stderr,
                "lotus_bus_register_codec: null subject, codec_self, "
                "or method pointer\n");
        return;
    }
    for (size_t i = 0; i < g_bus_remote_count; i++) {
        lotus_bus_remote_entry_t *e = g_bus_remote_entries[i];
        if (!e->subject) continue;
        if (strcmp(e->subject, subject) != 0) continue;
        e->codec_self      = codec_self;
        e->codec_encode_fn = encode_fn;
        e->codec_decode_fn = decode_fn;
        return;
    }
    fprintf(stderr,
            "lotus_bus_register_codec: no remote binding for `%s` — "
            "codec attachment ignored\n",
            subject);
}

/* Trim leading + trailing whitespace in-place. Returns a pointer
 * into the same buffer; the caller still owns the allocation. */
static char *lotus__bus_strip(char *s) {
    while (*s == ' ' || *s == '\t') s++;
    char *end = s + strlen(s);
    while (end > s) {
        char c = end[-1];
        if (c != ' ' && c != '\t' && c != '\n' && c != '\r') break;
        end--;
    }
    *end = '\0';
    return s;
}

void lotus_bus_load_config(const char *path) {
    if (!path) return;
    FILE *fp = fopen(path, "r");
    if (!fp) {
        fprintf(stderr,
                "lotus_bus_load_config: cannot open %s: %s\n",
                path, strerror(errno));
        return;
    }
    char line[1024];
    int lineno = 0;
    while (fgets(line, sizeof(line), fp)) {
        lineno++;
        /* Strip end-of-line comments. */
        char *hash = strchr(line, '#');
        if (hash) *hash = '\0';
        char *trimmed = lotus__bus_strip(line);
        if (*trimmed == '\0') continue;

        char *eq = strchr(trimmed, '=');
        if (!eq) {
            fprintf(stderr,
                    "lotus_bus_load_config: %s:%d: missing '=' in '%s'\n",
                    path, lineno, trimmed);
            continue;
        }
        *eq = '\0';
        char *subject = lotus__bus_strip(trimmed);
        char *rest    = lotus__bus_strip(eq + 1);

        /* Split URL and role on the LAST ':'. URLs like
         * unix:///tmp/foo.sock contain a ':' inside the scheme,
         * so strrchr (last colon) reliably locates the role
         * suffix. */
        char *colon = strrchr(rest, ':');
        if (!colon || colon == rest) {
            fprintf(stderr,
                    "lotus_bus_load_config: %s:%d: missing ':role' "
                    "suffix on '%s'\n",
                    path, lineno, rest);
            continue;
        }
        *colon = '\0';
        char *url      = lotus__bus_strip(rest);
        char *role_str = lotus__bus_strip(colon + 1);

        int role_val;
        if (strcmp(role_str, "listen") == 0) {
            role_val = LOTUS_TRANSPORT_LISTEN;
        } else if (strcmp(role_str, "connect") == 0) {
            role_val = LOTUS_TRANSPORT_CONNECT;
        } else {
            fprintf(stderr,
                    "lotus_bus_load_config: %s:%d: unknown role "
                    "'%s' (expected 'listen' or 'connect')\n",
                    path, lineno, role_str);
            continue;
        }
        lotus_bus_register_remote(subject, url, role_val);
    }
    fclose(fp);
}

/* Forward-declared at the top of the bus router section so
 * lotus_bus_dispatch can fan out to remote subscribers without
 * caring about table layout. */
void lotus_bus_remote_fanout(const char *subject,
                             const void *payload,
                             size_t payload_size) {
    if (!subject) return;
    for (size_t i = 0; i < g_bus_remote_count; i++) {
        lotus_bus_remote_entry_t *e = g_bus_remote_entries[i];
        if (!e->subject) continue;
        if (strcmp(e->subject, subject) != 0) continue;
        if (e->kind == LOTUS_BUS_REMOTE_KIND_ADAPTER) {
            /* Wave B: package the wire bytes as an Hale-level
             * Bytes value (program-lifetime, lives in the payload
             * arena), then dispatch through the adapter locus's
             * `send` method. The adapter's body owns framing /
             * delivery — the bus only guarantees one whole message
             * per call. */
            if (!e->adapter_self || !e->adapter_send_fn) continue;
            lotus_arena_t *parena = lotus_bus_payload_arena_get();
            if (!parena) continue;
            void *bytes_val = lotus_bytes_from_buf(
                parena, payload, (int64_t)payload_size);
            if (!bytes_val) continue;
            e->adapter_send_fn(e->adapter_self, e->subject, bytes_val);
            continue;
        }
        if (e->kind == LOTUS_BUS_REMOTE_KIND_UDP) {
            /* CONNECT-side UDP fanout. sendto routes the
             * datagram according to the destination's address
             * class (unicast/multicast). The kernel doesn't
             * report delivery; sendto returning > 0 only
             * confirms the datagram entered the local IP stack —
             * that's the contract publishers expect from a UDP
             * transport. LISTEN-side UDP entries have udp_fd set
             * but role != CONNECT; their fd is owned by the
             * reader thread for recvfrom. */
            if (e->role != LOTUS_TRANSPORT_CONNECT) continue;
            if (e->udp_fd < 0) continue;
            ssize_t sent = sendto(e->udp_fd, payload, payload_size, 0,
                                  (struct sockaddr *)&e->udp_dest,
                                  sizeof(e->udp_dest));
            if (sent < 0) {
                lotus_bus_udp_log_sendto_error(e->subject, errno);
            }
            continue;
        }
        if (!e->transport) continue;
        /* CONNECT role only fans out at this milestone. LISTEN
         * role transports exist on the receive side and are
         * driven by the (future) reader thread, not by publish-
         * site dispatch. */
        if (e->role != LOTUS_TRANSPORT_CONNECT) continue;
        (void)lotus_transport_send(e->transport, payload, payload_size);
        /* Errors are logged inside lotus_transport_send; we don't
         * abort dispatch on transport failure — local subscribers
         * already received their copy. */
    }
}

/* m105 body — forward-declared near lotus_bus_local_dispatch.
 * See the doc-comment up there for the design rationale. Lives
 * here because the function body references g_bus_queue_for_remote
 * (declared just above) and the per-subject deserialize_fn from
 * g_bus_entries. */
/* Phase 3 keyed variant (2026-05-25). Mirrors
 * lotus_bus_dispatch_wire's per-subscriber-arena routing but
 * applies the routing-key filter at each entry. Same Task-9
 * arena-correctness story: deserialize INTO each subscriber's
 * own __arena via the TLS-routed allocator, so payload pointers
 * are bounded by the subscriber's lifecycle.
 *
 * v0.1: kind=1 entries fire on key match; kind=0 entries fire
 * on every keyed publish (audit-all sinks); kind=2 (catch-
 * unmatched fallback) is reserved for v0.2 and dispatched in
 * the no-specific-match pass at the end. */
void lotus_bus_dispatch_wire_keyed(const char *subject,
                                    const void *wire_bytes,
                                    size_t wire_size,
                                    uint64_t key_lo,
                                    uint64_t key_hi) {
    if (!subject || !wire_bytes || wire_size == 0) return;
    char *struct_buf = g_tls_bus_struct_buf;   /* off the coro stack */
    lotus_arena_t *prev_tls = lotus_current_caller_arena;
    int matched_specific = 0;
    size_t specific_subs_on_subject = 0;
    size_t unkeyed_subs_on_subject = 0;
    for (size_t i = 0; i < g_bus_count; i++) {
        lotus_bus_entry_t *e = &g_bus_entries[i];
        if (!e->subject) continue;
        if (!lotus_subject_match(e->subject, subject)) continue;
        if (!e->deserialize) continue;
        if (e->key_filter_kind == 1) {
            specific_subs_on_subject++;
            if (e->key_lo != key_lo || e->key_hi != key_hi) continue;
            matched_specific = 1;
        } else if (e->key_filter_kind == 0) {
            unkeyed_subs_on_subject++;
        } else {
            /* kind == 2: skip in the specific pass. */
            continue;
        }
        lotus_arena_t *sub_arena =
            e->self_ptr ? *(lotus_arena_t **)e->self_ptr : NULL;
        lotus_current_caller_arena = sub_arena;
        ssize_t struct_size = e->deserialize(
            wire_bytes, wire_size, struct_buf, LOTUS_PAYLOAD_MAX);
        if (struct_size <= 0) continue;
        if (e->mailbox) {
            lotus_mailbox_post(
                e->mailbox, e->handler, e->self_ptr,
                struct_buf, (size_t)struct_size);
        } else if (e->coop_pool) {
            /* F.31 Phase 4 (2026-05-28): mirror lotus_bus_local_dispatch's
             * coop_pool branch. Without this, subscribers on non-main
             * cooperative pools (e.g. cooperative(pool=ws_workers))
             * receive via the global queue and run on the publisher's
             * drain thread instead of their pool's worker, silently
             * violating the single-threaded-method invariant. */
            lotus_coop_pool_post(e->coop_pool, e->handler, e->self_ptr,
                                 struct_buf, (size_t)struct_size);
        } else if (g_bus_queue_for_remote) {
            lotus_bus_queue_enqueue(
                g_bus_queue_for_remote, e->handler, e->self_ptr,
                struct_buf, (size_t)struct_size);
        }
    }
    if (!matched_specific) {
        for (size_t i = 0; i < g_bus_count; i++) {
            lotus_bus_entry_t *e = &g_bus_entries[i];
            if (!e->subject) continue;
            if (!lotus_subject_match(e->subject, subject)) continue;
            if (!e->deserialize) continue;
            if (e->key_filter_kind != 2) continue;
            lotus_arena_t *sub_arena =
                e->self_ptr ? *(lotus_arena_t **)e->self_ptr : NULL;
            lotus_current_caller_arena = sub_arena;
            ssize_t struct_size = e->deserialize(
                wire_bytes, wire_size, struct_buf, LOTUS_PAYLOAD_MAX);
            if (struct_size <= 0) continue;
            if (e->mailbox) {
                lotus_mailbox_post(
                    e->mailbox, e->handler, e->self_ptr,
                    struct_buf, (size_t)struct_size);
            } else if (e->coop_pool) {
                lotus_coop_pool_post(e->coop_pool, e->handler,
                                     e->self_ptr, struct_buf,
                                     (size_t)struct_size);
            } else if (g_bus_queue_for_remote) {
                lotus_bus_queue_enqueue(
                    g_bus_queue_for_remote, e->handler, e->self_ptr,
                    struct_buf, (size_t)struct_size);
            }
        }
        if (lotus_bus_log_unmatched_enabled()) {
            fprintf(stderr,
                    "[bus] subject=\"%s\" key_lo=%" PRIu64
                    " key_hi=%" PRIu64
                    " no_specific_match (%zu specific subs on subject; "
                    "%zu unkeyed)\n",
                    subject, key_lo, key_hi,
                    specific_subs_on_subject,
                    unkeyed_subs_on_subject);
        }
    }
    lotus_current_caller_arena = prev_tls;
}

void lotus_bus_dispatch_wire(const char *subject,
                             const void *wire_bytes,
                             size_t wire_size) {
    if (!subject || !wire_bytes || wire_size == 0) return;
    /* Phase-3 Task 9 (2026-05-20): per-subscriber arena routing.
     * Previously this deserialize-once-then-fanout shape parked
     * the deserialized String/Bytes pointers in the program-
     * lifetime g_bus_payload_arena (leaky for long-running
     * cross-process consumers). Now we iterate matching
     * subscribers and deserialize INTO EACH SUBSCRIBER'S OWN
     * arena via the TLS-routed allocator (Task 8 helpers); the
     * payload pointers in the enqueued struct_buf alias the
     * subscriber's own __arena, bounded by the subscriber's
     * lifecycle. Cost: deserialize is called once per matching
     * subscriber instead of once total — a real cost for high-
     * fan-out subjects, but the structural correctness win is
     * load-bearing for production daemons (HYPERLOOP-MDGW).
     *
     * The m20 spec's "subscriber's arena outlives the payload
     * pointer" guarantee now holds end-to-end: the pointer was
     * allocated in the subscriber's arena to begin with, so it
     * stays valid until the subscriber dissolves. No more
     * program-lifetime deposit. */
    char *struct_buf = g_tls_bus_struct_buf;   /* off the coro stack */
    lotus_arena_t *prev_tls = lotus_current_caller_arena;
    size_t matched = 0;
    size_t delivered = 0;
    for (size_t i = 0; i < g_bus_count; i++) {
        lotus_bus_entry_t *e = &g_bus_entries[i];
        if (!e->subject) continue;
        if (!lotus_subject_match(e->subject, subject)) continue;
        if (!e->deserialize) {
            if (lotus_bus_log_drop_enabled()) {
                fprintf(stderr,
                        "[bus] publish dropped at entry %zu: subject=\"%s\" "
                        "matched but deserialize_fn is NULL\n",
                        i, subject);
            }
            continue;
        }
        /* Phase 3 (2026-05-25): mirror lotus_bus_local_dispatch —
         * unkeyed publishes (which is what this entry point
         * services; the keyed variant is lotus_bus_dispatch_wire_keyed)
         * must NOT fire entries that registered a key filter.
         * Otherwise an unkeyed `<-` to a keyed subject would
         * deliver to every kind=1 subscriber regardless of key,
         * bypassing the routing-key contract. */
        if (e->key_filter_kind != 0) continue;
        matched++;
        /* Load the subscriber's __arena via the m20 fixed-offset
         * GEP: slot 0 of every locus struct is `__arena: ptr`. */
        lotus_arena_t *sub_arena =
            e->self_ptr
                ? *(lotus_arena_t **)e->self_ptr
                : NULL;
        lotus_current_caller_arena = sub_arena;
        ssize_t struct_size = e->deserialize(
            wire_bytes, wire_size, struct_buf, LOTUS_PAYLOAD_MAX);
        if (struct_size <= 0) {
            if (lotus_bus_log_drop_enabled()) {
                fprintf(stderr,
                        "[bus] publish dropped at entry %zu: subject=\"%s\" "
                        "deserialize returned %zd (wire_size=%zu)\n",
                        i, subject, struct_size, wire_size);
            }
            continue;
        }
        if (e->mailbox) {
            lotus_mailbox_post(
                e->mailbox, e->handler, e->self_ptr,
                struct_buf, (size_t)struct_size);
            delivered++;
        } else if (e->coop_pool) {
            /* F.31 Phase 4 (2026-05-28): same fix as the keyed
             * variant above — wire-dispatch was missing the
             * coop_pool branch that lotus_bus_local_dispatch ships
             * with, so subscribers on non-main cooperative pools
             * silently routed to the global queue. */
            lotus_coop_pool_post(e->coop_pool, e->handler, e->self_ptr,
                                 struct_buf, (size_t)struct_size);
            delivered++;
        } else if (g_bus_queue_for_remote) {
            lotus_bus_queue_enqueue(
                g_bus_queue_for_remote, e->handler, e->self_ptr,
                struct_buf, (size_t)struct_size);
            delivered++;
        } else if (lotus_bus_log_drop_enabled()) {
            fprintf(stderr,
                    "[bus] publish dropped at entry %zu: subject=\"%s\" "
                    "no post target (mailbox/coop_pool/queue all NULL)\n",
                    i, subject);
        }
    }
    if (matched == 0 && lotus_bus_log_drop_enabled()) {
        fprintf(stderr,
                "[bus] publish dropped: no local subscribers for "
                "subject=\"%s\" via wire path (g_bus_count=%zu, "
                "wire_size=%zu)\n",
                subject, g_bus_count, wire_size);
    }
    (void)delivered;
    lotus_current_caller_arena = prev_tls;
}

/* m70: lazy global "payload arena" for String byte storage in
 * cross-process bus deserialization. The reader thread fills a
 * stack-local struct_buf, dispatches via lotus_bus_local_dispatch
 * (which copies struct_buf bytes into a queue cell), and after
 * drain copies the cell bytes into the subscriber's arena. Any
 * String pointer in struct_buf must outlive that whole chain —
 * the subscriber's arena isn't accessible at deserialize time
 * (we don't yet know which subscriber will fire; a subject can
 * have multiple), so we allocate from a long-lived shared arena
 * instead. Lifetime is the program; destroyed in
 * lotus_bus_remote_destroy_all. Memory grows unbounded —
 * acceptable for v1 (subscribers run for bounded duration). The
 * pthread mutex serializes allocator access since reader threads
 * call this concurrently. */
static lotus_arena_t   *g_bus_payload_arena       = NULL;
static pthread_mutex_t  g_bus_payload_arena_mutex = PTHREAD_MUTEX_INITIALIZER;

/* Phase-3 safety net (2026-05-19): bound g_bus_payload_arena to a
 * fixed byte budget so a leaking long-running program crashes
 * loudly via the cap diagnostic instead of an OOM kill. The cap
 * is set at the only place the arena is created (this helper);
 * all 35+ lazy-init sites in this file go through it. Default
 * 64 MiB; overridable via LOTUS_BUS_PAYLOAD_ARENA_CAP env var (in
 * bytes) for capacity-planning experiments. The cap event is
 * dprintf'd to stderr exactly once per process. Subsequent
 * allocations against the capped arena return NULL; existing
 * callers (snapshot/finish via alloc_fail_sentinel,
 * lotus_bytes_create returning NULL through empty_global,
 * recv_bytes returning empty Bytes) already surface NULL as
 * "empty / failure" so the cap converts an OOM to a degraded
 * service mode rather than a crash. The proper fix
 * (per-subscriber arena routing for m70 wire-dispatch, plus
 * __caller_arena threading for the stdlib primitives that land
 * here) lives elsewhere; this is the floor. */
#define LOTUS_BUS_PAYLOAD_ARENA_DEFAULT_CAP_BYTES \
    ((size_t)64 * 1024 * 1024)

static lotus_arena_t *lotus_bus_payload_arena_create_capped(void) {
    lotus_arena_t *a = lotus_arena_create_labeled("g_bus_payload_arena");
    if (!a) return NULL;
    size_t cap = LOTUS_BUS_PAYLOAD_ARENA_DEFAULT_CAP_BYTES;
    const char *env = getenv("LOTUS_BUS_PAYLOAD_ARENA_CAP");
    if (env && env[0]) {
        char *end = NULL;
        unsigned long long v = strtoull(env, &end, 10);
        if (end != env && v > 0 && v <= SIZE_MAX) {
            cap = (size_t)v;
        }
    }
    a->chunk_byte_cap = cap;
    a->cap_diag_name  = "g_bus_payload_arena";
    return a;
}

/* Phase-3 stdlib __caller_arena threading (2026-05-19). User-
 * callable stdlib primitives that previously allocated their
 * result into g_bus_payload_arena now route through the
 * thread-local `lotus_current_caller_arena` instead. The codegen
 * lowering sets the TLS pointer before each call site to
 * whichever arena makes sense for the calling context
 * (current_self's locus arena from a method body, __caller_arena
 * from a free fn, lotus.arena.global from main). The primitive
 * reads via lotus_caller_arena_or_global() — falls back to the
 * capped g_bus_payload_arena when no TLS is set (interpreter,
 * non-Hale C entry, etc.).
 *
 * The TLS indirection lets us migrate the primitive surface
 * without changing every C signature. The fallback preserves
 * backward compat: any caller (in C or interpreter land) that
 * forgets to set the TLS still gets a valid arena, just the
 * leaky one. */
__thread lotus_arena_t *lotus_current_caller_arena = NULL;

void lotus_set_caller_arena(lotus_arena_t *a) {
    lotus_current_caller_arena = a;
}

lotus_arena_t *lotus_caller_arena_or_global(void) {
    if (lotus_current_caller_arena) return lotus_current_caller_arena;
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_bus_payload_arena_create_capped();
    }
    lotus_arena_t *p = g_bus_payload_arena;
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return p;
}

/* Migration helper: same lock-init-alloc pattern that nearly every
 * stdlib primitive in this file repeats, factored into a single
 * fn so the migration becomes "replace the open-coded block with
 * one call." Routes through TLS when set (fast, no mutex; arena
 * is per-thread), falls back to the capped g_bus_payload_arena
 * with the mutex when no TLS. Returns NULL on either alloc
 * failure (cap or malloc); callers' existing NULL handling
 * surfaces it. */
void *lotus_caller_or_global_bytes_create(int64_t len) {
    if (lotus_current_caller_arena) {
        return lotus_bytes_create(lotus_current_caller_arena, len);
    }
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_bus_payload_arena_create_capped();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *blob = lotus_bytes_create(g_bus_payload_arena, len);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return blob;
}

void *lotus_bus_payload_arena_alloc(size_t size, size_t align) {
    /* Phase-3: route through the caller_arena TLS when set so
     * stdlib primitives that go through this helper (str_lower /
     * str_upper / pad_left / etc.) get caller-scoped allocation
     * automatically. Falls back to the capped g_bus_payload_arena
     * when no TLS is set (interpreter, non-Hale C entry,
     * pre-init code paths). The fallback path keeps the mutex so
     * concurrent reader threads using m70 wire dispatch remain
     * thread-safe; the TLS path skips the mutex (per-thread
     * arena = single accessor). */
    if (lotus_current_caller_arena) {
        return lotus_arena_alloc(
            lotus_current_caller_arena, size, align);
    }
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_bus_payload_arena_create_capped();
        if (!g_bus_payload_arena) {
            pthread_mutex_unlock(&g_bus_payload_arena_mutex);
            return NULL;
        }
    }
    void *p = lotus_arena_alloc(g_bus_payload_arena, size, align);
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return p;
}

/* Wave B: handle-only accessor. Lazy-initializes the bus payload
 * arena on first call (same machinery as lotus_bus_payload_arena_alloc)
 * and returns the pointer. The bus adapter fanout path uses this
 * to hand a stable arena to lotus_bytes_from_buf so the Bytes
 * value lives past the dispatch call frame. */
lotus_arena_t *lotus_bus_payload_arena_get(void) {
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (!g_bus_payload_arena) {
        g_bus_payload_arena = lotus_bus_payload_arena_create_capped();
    }
    lotus_arena_t *p = g_bus_payload_arena;
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
    return p;
}

/*
 * m89: read_bytes wrapper that anchors the resulting Bytes
 * blob in the lazy global payload arena (same lifetime
 * mechanism as read_file's String). Doing it this way keeps
 * the Bytes value valid for the entire program — a fn that
 * returns Bytes can rely on the pointer staying live past
 * the call site without m49-style deep-copy plumbing.
 */
void *lotus_fs_read_bytes_global(const char *path) {
    /* lotus_fs_read_bytes allocates internally via
     * lotus_arena_alloc; we hold the mutex around it because
     * the global arena is shared across reader threads. */
    void *result = lotus_fs_read_bytes(lotus_caller_arena_or_global(), path);
    return result;
}

/*
 * C4 (pond/crypto follow-up): cryptographically-strong random
 * bytes. Returns a fresh Bytes blob of length `n` anchored in
 * the bus payload arena, mirroring `lotus_fs_read_bytes_global`'s
 * lifetime story.
 *
 * Implementation order (Linux):
 *   1. `getrandom(buf, n, 0)` — modern syscall, retries on EINTR,
 *      handles short returns by looping.
 *   2. If `getrandom` is unavailable (ENOSYS) or `<sys/random.h>`
 *      isn't visible at build time, fall through to reading
 *      `/dev/urandom` until `n` bytes are filled.
 *
 * Argument shape:
 *   - `n <= 0`         → returns a length-0 Bytes blob, no error.
 *   - `n > GETRANDOM_MAX (8192)` → sets errno=EINVAL and returns
 *     NULL so the codegen-side wrapper synthesizes an IoError
 *     with kind="invalid". The cap is a per-call ergonomic
 *     limit, not the kernel's GRND_MAX (33,554,431) — agents
 *     pulling more than 8 KiB at once are almost certainly
 *     doing something wrong (key material is 16-64 bytes;
 *     session tokens are 16-32). They can loop if they really
 *     want more.
 *   - Any read error from the underlying source surfaces as
 *     NULL + errno from the failing call.
 */
#define LOTUS_GETRANDOM_PER_CALL_MAX 8192

void *lotus_os_getrandom(int64_t n) {
    /* n <= 0 → caller wants empty (no error). */
    if (n <= 0) {
        return lotus_caller_or_global_bytes_create(0);
    }
    if (n > LOTUS_GETRANDOM_PER_CALL_MAX) {
        errno = EINVAL;
        return NULL;
    }
    void *blob = lotus_caller_or_global_bytes_create(n);
    if (!blob) {
        errno = ENOMEM;
        return NULL;
    }
    unsigned char *body = (unsigned char *)lotus_bytes_data(blob);
    size_t left = (size_t)n;
    unsigned char *p = body;

    /* Step 1: try getrandom(2). On ENOSYS, fall through. */
#if defined(__linux__) || defined(__GLIBC__)
    while (left > 0) {
        ssize_t r = getrandom(p, left, 0);
        if (r > 0) {
            p    += (size_t)r;
            left -= (size_t)r;
            continue;
        }
        if (r < 0 && errno == EINTR) continue;
        if (r < 0 && errno == ENOSYS) {
            /* Kernel too old (pre-3.17). Reset progress so the
             * urandom fallback rewrites the entire buffer. */
            p    = body;
            left = (size_t)n;
            break;
        }
        /* Any other error: surface it. */
        return NULL;
    }
    if (left == 0) {
        return blob;
    }
#endif

    /* Step 2: /dev/urandom fallback. Used on platforms without
     * the syscall and on Linux kernels too old to expose it. */
    int fd = open("/dev/urandom", O_RDONLY);
    if (fd < 0) {
        return NULL;
    }
    while (left > 0) {
        ssize_t r = read(fd, p, left);
        if (r > 0) {
            p    += (size_t)r;
            left -= (size_t)r;
            continue;
        }
        if (r < 0 && errno == EINTR) continue;
        /* r == 0 from /dev/urandom is "impossible" but treat as
         * EIO so the caller sees a useful kind. */
        if (r == 0) errno = EIO;
        close(fd);
        return NULL;
    }
    close(fd);
    return blob;
}

/*
 * ====================================================================
 * C2 — subprocess primitives.
 *
 * Two surfaces:
 *   1. `lotus_process_run` — synchronous fork+exec+wait. The 80%
 *      shell-out case: capture stdout/stderr/exit-code and hand
 *      the whole package back as a ProcessOutput struct.
 *   2. `lotus_process_spawn` + `_wait` + `_kill_escalate` +
 *      `_pipe_read_nonblocking` + `_pipe_write` — async lifecycle
 *      for long-running children (`pip install`, a TUI subprocess,
 *      a build job).
 *
 * Argv shape (both surfaces): newline-separated String. argv[0] is
 * the executable, argv[1..] are args. Trailing newline allowed but
 * not required. Hale's array surface is statically-sized, so the
 * newline-blob is the v1 ergonomic compromise (mirrors cli.hl's
 * argv_keys: String pattern). Internal split-on-newline produces
 * the exec(2) argv array; the buffer is malloc'd, freed in the
 * caller frame after exec returns.
 *
 * Design notes:
 *   - SIGPIPE is ignored globally (see lotus_io_init); writes to a
 *     closed pipe surface EPIPE through errno instead of killing
 *     the parent.
 *   - Child gets its own process group (`setpgid(0, 0)` in child)
 *     so a parent crash doesn't strand orphans on shared session
 *     resources, and a `kill_escalate` against the pid can target
 *     the whole pgid later if we want to. setpgid over prctl(
 *     PR_SET_PDEATHSIG) because the former is POSIX (macOS / BSD
 *     work the same way) and the latter is Linux-only — auto-reap-
 *     on-parent-death is also too aggressive: a controlled Hale
 *     dissolve already covers the orderly-shutdown path, and the
 *     hard-parent-crash case is handled at a higher level (systemd
 *     reaper, docker cgroup, etc.).
 *   - All fds are closed on every path. Subprocess code leaks fds
 *     more easily than anything else; every error-handling branch
 *     here explicitly drops the pipes.
 *   - kill_escalate is TERM → wait 100ms (polling via waitpid
 *     WNOHANG) → KILL → blocking waitpid to reap.
 * ====================================================================
 */

/* Per-call cap on argv blob size — defensive against pathological
 * inputs. Linux's ARG_MAX is typically 2MB; we cap lower so a
 * runaway concat in user code surfaces here, not after the kernel
 * rejects with E2BIG on exec. */
#define LOTUS_PROCESS_ARGV_MAX 65536

/* Internal: split `blob` (newline-separated) into a malloc'd argv
 * array with a trailing NULL slot. Returns 0 on success; on failure
 * (oversized, empty, OOM) sets errno and returns -1. On success,
 * the caller owns *out_argv (a single malloc — the entries point
 * into a sibling malloc'd buffer that's also returned via *out_buf
 * so the caller can free both). */
static int lotus_process_split_argv(
    const char *blob,
    char ***out_argv,
    char **out_buf,
    int *out_count
) {
    if (!blob) {
        errno = EINVAL;
        return -1;
    }
    size_t blob_len = strlen(blob);
    if (blob_len == 0) {
        errno = EINVAL;
        return -1;
    }
    if (blob_len > LOTUS_PROCESS_ARGV_MAX) {
        errno = E2BIG;
        return -1;
    }
    char *buf = (char *)malloc(blob_len + 1);
    if (!buf) {
        errno = ENOMEM;
        return -1;
    }
    memcpy(buf, blob, blob_len + 1);
    int count = 0;
    for (size_t i = 0; i < blob_len; i++) {
        if (buf[i] == '\n') count++;
    }
    /* If the last character isn't a newline, the tail is its own
     * arg. */
    if (blob_len > 0 && buf[blob_len - 1] != '\n') {
        count++;
    }
    if (count == 0) {
        free(buf);
        errno = EINVAL;
        return -1;
    }
    char **argv = (char **)malloc(sizeof(char *) * (size_t)(count + 1));
    if (!argv) {
        free(buf);
        errno = ENOMEM;
        return -1;
    }
    int idx = 0;
    char *start = buf;
    for (size_t i = 0; i < blob_len; i++) {
        if (buf[i] == '\n') {
            buf[i] = '\0';
            argv[idx++] = start;
            start = buf + i + 1;
        }
    }
    if (start < buf + blob_len) {
        argv[idx++] = start;
    }
    argv[idx] = NULL;
    *out_argv = argv;
    *out_buf = buf;
    *out_count = count;
    return 0;
}

/* C2 surface 1: synchronous run. fork → setpgid → exec in the
 * child; parent reads stdout + stderr to EOF (interleaved via
 * poll() so the child can write either stream in any order
 * without deadlocking), waits for exit.
 *
 * Out-params (all must be non-NULL):
 *   - *out_code   = exit code (-1 if killed by signal)
 *   - *out_signal = signal number (0 if normal exit)
 *   - *out_stdout = captured stdout (NUL-terminated, arena-anchored)
 *   - *out_stderr = captured stderr (NUL-terminated, arena-anchored)
 *
 * Returns 0 on success (fork+exec ok and waited), errno on
 * failure. ENOENT if the executable isn't found, EACCES if it's
 * not executable, ENOMEM on allocation failure, etc.
 */
/* std::process::rss_bytes (2026-05-21): the calling process's
 * peak resident-set size in bytes. Wraps getrusage(RUSAGE_SELF).
 *
 * Linux reports ru_maxrss in KiB (the kernel's `struct rusage`
 * comment says "maximum resident set size (in kilobytes)"; man
 * getrusage confirms `ru_maxrss` is "in kilobytes" on Linux).
 * BSD / macOS report it in bytes. We're targeting Linux for v1
 * so we multiply by 1024 unconditionally. If we ever care about
 * BSD parity, this is one #ifdef away.
 *
 * The "peak" framing (high-water mark, not current RSS) is what
 * `getrusage` actually exposes — there's no syscall for current
 * RSS that doesn't go through /proc. For observability the peak
 * is usually what matters (alarm thresholds key off worst-case);
 * a future `current_rss_bytes()` could parse /proc/self/statm
 * once the read_file fix lands.
 *
 * On failure (getrusage rejects, vanishingly rare) returns 0 so
 * the caller doesn't get a negative value through the i64 ABI. */
int64_t lotus_process_rss_bytes(void) {
    struct rusage ru;
    if (getrusage(RUSAGE_SELF, &ru) != 0) {
        return 0;
    }
    return (int64_t)ru.ru_maxrss * 1024;
}

int lotus_process_run(
    const char *argv_blob,
    int32_t *out_code,
    int32_t *out_signal,
    const char **out_stdout_str,
    const char **out_stderr_str
) {
    if (!out_code || !out_signal || !out_stdout_str || !out_stderr_str) {
        return EINVAL;
    }
    *out_code = -1;
    *out_signal = 0;
    *out_stdout_str = "";
    *out_stderr_str = "";

    char **argv = NULL;
    char *argv_buf = NULL;
    int argc = 0;
    if (lotus_process_split_argv(argv_blob, &argv, &argv_buf, &argc) < 0) {
        return errno ? errno : EINVAL;
    }

    int out_pipe[2] = { -1, -1 };
    int err_pipe[2] = { -1, -1 };
    if (pipe(out_pipe) < 0) {
        int saved = errno;
        free(argv); free(argv_buf);
        return saved;
    }
    if (pipe(err_pipe) < 0) {
        int saved = errno;
        close(out_pipe[0]); close(out_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }

    pid_t pid = fork();
    if (pid < 0) {
        int saved = errno;
        close(out_pipe[0]); close(out_pipe[1]);
        close(err_pipe[0]); close(err_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }
    if (pid == 0) {
        /* CHILD. setpgid first so a kill against this pid can
         * target the whole group later. Errors from setpgid are
         * non-fatal at exec time — proceed regardless. */
        setpgid(0, 0);
        if (dup2(out_pipe[1], STDOUT_FILENO) < 0) _exit(127);
        if (dup2(err_pipe[1], STDERR_FILENO) < 0) _exit(127);
        close(out_pipe[0]);
        close(out_pipe[1]);
        close(err_pipe[0]);
        close(err_pipe[1]);
        /* stdin from /dev/null so the child doesn't hang on read.
         * Best-effort — if /dev/null can't be opened, leave stdin
         * inherited from the parent. */
        int devnull = open("/dev/null", O_RDONLY);
        if (devnull >= 0) {
            dup2(devnull, STDIN_FILENO);
            close(devnull);
        }
        execvp(argv[0], argv);
        _exit(127);
    }

    /* PARENT. Close the write ends; we only read. */
    close(out_pipe[1]); out_pipe[1] = -1;
    close(err_pipe[1]); err_pipe[1] = -1;

    /* Drain both pipes via poll() so the child can write to
     * stdout and stderr in any order without filling either pipe
     * buffer and blocking. Naive "drain stdout to EOF then
     * stderr" deadlocks when the child writes >PIPE_BUF (~64KB)
     * to stderr without the parent reading. Cap each stream at
     * 16 MiB to bound parent memory against runaway children. */
    struct pollfd pfds[2];
    pfds[0].fd = out_pipe[0]; pfds[0].events = POLLIN;
    pfds[1].fd = err_pipe[0]; pfds[1].events = POLLIN;
    const size_t cap_bytes = 16 * 1024 * 1024;
    size_t out_cap = 4096, out_len = 0;
    size_t err_cap = 4096, err_len = 0;
    char *out_buf = (char *)malloc(out_cap);
    char *err_buf = (char *)malloc(err_cap);
    int drain_err = 0;
    if (!out_buf || !err_buf) {
        drain_err = ENOMEM;
    }
    int out_open = 1, err_open = 1;
    while (!drain_err && (out_open || err_open)) {
        pfds[0].events = out_open ? POLLIN : 0;
        pfds[1].events = err_open ? POLLIN : 0;
        int pn = poll(pfds, 2, -1);
        if (pn < 0) {
            if (errno == EINTR) continue;
            drain_err = errno;
            break;
        }
        if (out_open && (pfds[0].revents & (POLLIN | POLLHUP | POLLERR))) {
            if (out_len + 1 >= out_cap && out_cap < cap_bytes) {
                size_t nc = out_cap * 2;
                if (nc > cap_bytes) nc = cap_bytes + 1;
                char *nb = (char *)realloc(out_buf, nc);
                if (!nb) { drain_err = ENOMEM; break; }
                out_buf = nb; out_cap = nc;
            }
            ssize_t r = read(out_pipe[0], out_buf + out_len,
                             out_cap - out_len - 1);
            if (r > 0) {
                out_len += (size_t)r;
                if (out_len >= cap_bytes) out_open = 0;
            } else if (r == 0) {
                out_open = 0;
            } else if (errno != EINTR) {
                drain_err = errno;
                break;
            }
        }
        if (err_open && (pfds[1].revents & (POLLIN | POLLHUP | POLLERR))) {
            if (err_len + 1 >= err_cap && err_cap < cap_bytes) {
                size_t nc = err_cap * 2;
                if (nc > cap_bytes) nc = cap_bytes + 1;
                char *nb = (char *)realloc(err_buf, nc);
                if (!nb) { drain_err = ENOMEM; break; }
                err_buf = nb; err_cap = nc;
            }
            ssize_t r = read(err_pipe[0], err_buf + err_len,
                             err_cap - err_len - 1);
            if (r > 0) {
                err_len += (size_t)r;
                if (err_len >= cap_bytes) err_open = 0;
            } else if (r == 0) {
                err_open = 0;
            } else if (errno != EINTR) {
                drain_err = errno;
                break;
            }
        }
    }
    close(out_pipe[0]); out_pipe[0] = -1;
    close(err_pipe[0]); err_pipe[0] = -1;

    /* Reap. waitpid retries on EINTR. */
    int status = 0;
    for (;;) {
        pid_t w = waitpid(pid, &status, 0);
        if (w == pid) break;
        if (w < 0 && errno == EINTR) continue;
        if (!drain_err) drain_err = errno;
        break;
    }

    if (drain_err) {
        free(out_buf); free(err_buf);
        free(argv); free(argv_buf);
        return drain_err;
    }

    out_buf[out_len] = '\0';
    err_buf[err_len] = '\0';
    /* Anchor in the caller's arena (via TLS) or the capped global
     * fallback so the pointers outlive this call frame. */
    char *out_anchored = (char *)lotus_bus_payload_arena_alloc(
        out_len + 1, 1);
    char *err_anchored = (char *)lotus_bus_payload_arena_alloc(
        err_len + 1, 1);
    if (!out_anchored || !err_anchored) {
        free(out_buf); free(err_buf);
        free(argv); free(argv_buf);
        return ENOMEM;
    }
    memcpy(out_anchored, out_buf, out_len + 1);
    memcpy(err_anchored, err_buf, err_len + 1);
    free(out_buf); free(err_buf);
    free(argv); free(argv_buf);

    *out_stdout_str = out_anchored;
    *out_stderr_str = err_anchored;

    /* Decode exit status. */
    if (WIFEXITED(status)) {
        *out_code = WEXITSTATUS(status);
        *out_signal = 0;
    } else if (WIFSIGNALED(status)) {
        *out_code = -1;
        *out_signal = WTERMSIG(status);
    } else {
        *out_code = -1;
        *out_signal = 0;
    }
    /* Distinguish exec failure (127) so the agent sees "not_found"
     * when execvp failed in the child. We do this here because the
     * child's _exit(127) is the only signal we have — we don't
     * route the exec errno across the fork boundary in v1. If
     * stderr is empty, this was almost certainly a child-side
     * exec failure. If the child wrote to stderr, it ran and
     * chose 127 — we trust that and don't override. */
    if (WIFEXITED(status) && WEXITSTATUS(status) == 127 && err_len == 0) {
        errno = ENOENT;
        return ENOENT;
    }
    return 0;
}

/* C2 surface 2a: async spawn. fork → setpgid → exec in the child;
 * parent gets back the pid + three pipe fds (stdin write, stdout
 * read, stderr read).
 *
 * Returns 0 on success, errno on failure. All out-params are
 * populated only on success.
 */
int lotus_process_spawn(
    const char *argv_blob,
    int32_t *out_pid,
    int32_t *out_stdin_fd,
    int32_t *out_stdout_fd,
    int32_t *out_stderr_fd
) {
    if (!out_pid || !out_stdin_fd || !out_stdout_fd || !out_stderr_fd) {
        return EINVAL;
    }
    char **argv = NULL;
    char *argv_buf = NULL;
    int argc = 0;
    if (lotus_process_split_argv(argv_blob, &argv, &argv_buf, &argc) < 0) {
        return errno ? errno : EINVAL;
    }

    int in_pipe[2]  = { -1, -1 };
    int out_pipe[2] = { -1, -1 };
    int err_pipe[2] = { -1, -1 };
    if (pipe(in_pipe) < 0) {
        int saved = errno;
        free(argv); free(argv_buf);
        return saved;
    }
    if (pipe(out_pipe) < 0) {
        int saved = errno;
        close(in_pipe[0]); close(in_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }
    if (pipe(err_pipe) < 0) {
        int saved = errno;
        close(in_pipe[0]); close(in_pipe[1]);
        close(out_pipe[0]); close(out_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }

    pid_t pid = fork();
    if (pid < 0) {
        int saved = errno;
        close(in_pipe[0]); close(in_pipe[1]);
        close(out_pipe[0]); close(out_pipe[1]);
        close(err_pipe[0]); close(err_pipe[1]);
        free(argv); free(argv_buf);
        return saved;
    }
    if (pid == 0) {
        setpgid(0, 0);
        if (dup2(in_pipe[0], STDIN_FILENO) < 0) _exit(127);
        if (dup2(out_pipe[1], STDOUT_FILENO) < 0) _exit(127);
        if (dup2(err_pipe[1], STDERR_FILENO) < 0) _exit(127);
        close(in_pipe[0]);  close(in_pipe[1]);
        close(out_pipe[0]); close(out_pipe[1]);
        close(err_pipe[0]); close(err_pipe[1]);
        execvp(argv[0], argv);
        _exit(127);
    }

    /* PARENT. Close the child-side ends, keep our own. */
    close(in_pipe[0]);  in_pipe[0] = -1;
    close(out_pipe[1]); out_pipe[1] = -1;
    close(err_pipe[1]); err_pipe[1] = -1;

    /* Mark the parent-side pipe fds non-blocking so reads return
     * EAGAIN promptly instead of blocking. stdin write stays
     * blocking — the caller controls when to write, and a write
     * EAGAIN would confuse the common case. SIGPIPE is ignored
     * globally so a write after the child exits returns EPIPE
     * via the IoError channel. */
    int flags;
    flags = fcntl(out_pipe[0], F_GETFL, 0);
    if (flags >= 0) fcntl(out_pipe[0], F_SETFL, flags | O_NONBLOCK);
    flags = fcntl(err_pipe[0], F_GETFL, 0);
    if (flags >= 0) fcntl(err_pipe[0], F_SETFL, flags | O_NONBLOCK);

    free(argv); free(argv_buf);

    *out_pid = (int32_t)pid;
    *out_stdin_fd = (int32_t)in_pipe[1];
    *out_stdout_fd = (int32_t)out_pipe[0];
    *out_stderr_fd = (int32_t)err_pipe[0];
    return 0;
}

/* C2 surface 2b: blocking wait. Reaps the child, decodes the exit
 * status. Returns 0 on success, errno on failure. */
int lotus_process_wait(
    int32_t pid,
    int32_t *out_code,
    int32_t *out_signal
) {
    if (!out_code || !out_signal) return EINVAL;
    *out_code = -1;
    *out_signal = 0;
    int status = 0;
    for (;;) {
        pid_t w = waitpid((pid_t)pid, &status, 0);
        if (w == (pid_t)pid) break;
        if (w < 0 && errno == EINTR) continue;
        return errno ? errno : ECHILD;
    }
    if (WIFEXITED(status)) {
        *out_code = WEXITSTATUS(status);
        *out_signal = 0;
    } else if (WIFSIGNALED(status)) {
        *out_code = -1;
        *out_signal = WTERMSIG(status);
    }
    return 0;
}

/* C2 surface 2c: TERM → wait 100ms → KILL → reap. Returns 0 on
 * success (the process has been reaped or was already gone),
 * errno on failure. Idempotent against already-reaped children
 * (ESRCH from kill is treated as success since the goal is "this
 * pid is no longer running").
 *
 * The 100ms window is the SIGTERM grace period. Long enough that
 * a well-behaved child (one that handles SIGTERM by flushing +
 * exiting) finishes; short enough that a wedged child gets the
 * KILL hammer promptly. Polls via waitpid(WNOHANG) at 5ms
 * intervals so we exit early as soon as the child reaps.
 */
int lotus_process_kill_escalate(int32_t pid) {
    if (pid <= 0) return EINVAL;
    if (kill((pid_t)pid, SIGTERM) < 0) {
        if (errno != ESRCH) {
            return errno;
        }
    }
    const int poll_interval_us = 5000;
    const int total_us = 100 * 1000;
    int elapsed = 0;
    int status = 0;
    while (elapsed < total_us) {
        pid_t w = waitpid((pid_t)pid, &status, WNOHANG);
        if (w == (pid_t)pid) return 0;
        if (w < 0) {
            if (errno == EINTR) continue;
            if (errno == ECHILD) return 0;
            return errno;
        }
        struct timespec ts;
        ts.tv_sec = 0;
        ts.tv_nsec = (long)poll_interval_us * 1000L;
        nanosleep(&ts, NULL);
        elapsed += poll_interval_us;
    }
    if (kill((pid_t)pid, SIGKILL) < 0) {
        if (errno != ESRCH) return errno;
    }
    for (;;) {
        pid_t w = waitpid((pid_t)pid, &status, 0);
        if (w == (pid_t)pid) return 0;
        if (w < 0) {
            if (errno == EINTR) continue;
            if (errno == ECHILD) return 0;
            return errno;
        }
    }
}

/* C2 surface 2d: non-blocking read from a pipe fd opened by
 * lotus_process_spawn. Returns an arena-anchored NUL-terminated
 * string containing up to 64 KiB of available bytes.
 *
 * Return shapes:
 *   - non-empty string: bytes were available; copied into the
 *     bus_payload_arena.
 *   - empty string (""): EAGAIN / EWOULDBLOCK (no data available)
 *     OR EOF (child closed its write end). Use lotus_process_wait
 *     to distinguish.
 *   - NULL: hard error (EBADF, EIO, etc.) — errno set so the
 *     codegen-side wrapper synthesizes an IoError.
 */
const char *lotus_process_pipe_read_nonblocking(int32_t fd) {
    static const char empty[1] = { 0 };
    if (fd < 0) {
        errno = EBADF;
        return NULL;
    }
    char buf[65536];
    ssize_t r = read((int)fd, buf, sizeof(buf) - 1);
    if (r < 0) {
        if (errno == EAGAIN
#if defined(EWOULDBLOCK) && (EWOULDBLOCK != EAGAIN)
            || errno == EWOULDBLOCK
#endif
            || errno == EINTR) {
            return empty;
        }
        return NULL;
    }
    if (r == 0) {
        /* EOF — surface as empty. */
        return empty;
    }
    buf[r] = '\0';
    char *out = (char *)lotus_bus_payload_arena_alloc((size_t)r + 1, 1);
    if (!out) {
        errno = ENOMEM;
        return NULL;
    }
    memcpy(out, buf, (size_t)r + 1);
    return out;
}

/* C2 surface 2e: write a string to a pipe fd opened by
 * lotus_process_spawn. Returns bytes written on success, -1 on
 * error (errno set). Writes the full strlen of `s` — embedded NULs
 * truncate. SIGPIPE is ignored globally so a write after the child
 * closed its read end surfaces as EPIPE through errno, not a
 * signal. */
int64_t lotus_process_pipe_write(int32_t fd, const char *s) {
    if (fd < 0) {
        errno = EBADF;
        return -1;
    }
    if (!s) {
        errno = EINVAL;
        return -1;
    }
    size_t total = strlen(s);
    size_t left = total;
    const char *p = s;
    while (left > 0) {
        ssize_t w = write((int)fd, p, left);
        if (w > 0) {
            p += (size_t)w;
            left -= (size_t)w;
            continue;
        }
        if (w < 0 && errno == EINTR) continue;
        return -1;
    }
    return (int64_t)total;
}

/*
 * m90: list_dir wrapper anchoring the resulting String in
 * the global payload arena. Same lifetime motivation as
 * read_bytes_global / read_file: callers can stash the
 * pointer past the call site without m49-style deep-copy
 * plumbing.
 */
const char *lotus_fs_list_dir_global(const char *path) {
    static const char empty[1] = { 0 };
    const char *result = lotus_fs_list_dir(lotus_caller_arena_or_global(), path);
    return result;
}

/*
 * Extension lookup wrapper. Resolves the basename's last dot
 * (see lotus_fs_extension_locate) and copies the dot-prefixed
 * slice into the program-lifetime payload arena so the returned
 * String outlives the call frame — same convention as
 * read_file / list_dir / read_bytes. Returns the stable empty
 * string when there is no extension.
 */
const char *lotus_fs_extension_global(const char *path) {
    static const char empty[1] = { 0 };
    const char *ext = lotus_fs_extension_locate(path);
    if (!ext) return empty;
    char *out = lotus_str_clone(lotus_caller_arena_or_global(), ext);
    return out ? out : empty;
}

/*
 * Phase 2g: allocate a zero-length Bytes blob in the global
 * payload arena. Used as the "empty / error" return shape for
 * recv_bytes and the bytes_* helpers so callers always get a
 * well-formed blob (length=0 visible via lotus_bytes_len) rather
 * than NULL. Each call allocates fresh — callers downstream may
 * write to the buffer (lotus_bytes_data + 0 is in-bounds for
 * len-0 blobs but only via subsequent grow paths), and aliasing
 * via a singleton would surface mutation across unrelated callers.
 */
static void *lotus_bytes_empty_global(void) {
    void *empty = lotus_caller_or_global_bytes_create(0);
    return empty;
}

/* F.27 alloc-fail sentinel for BytesBuilder snapshot()/finish()
 * (2026-05-19). Stable static pointer with `[i64 0][i64 0]`
 * layout — eight bytes of length-prefix (read as 0 by
 * lotus_bytes_len) plus padding so lotus_bytes_data's `+8`
 * derive lands in valid memory. Returned from the C primitives
 * failure paths instead of lotus_bytes_empty_global, so the
 * locus method body's lotus_bytes_is_alloc_fail check
 * discriminates fail-empty from success-empty (the latter still
 * allocates fresh via lotus_bytes_create even for len=0). */
static const int64_t g_bytes_alloc_fail_sentinel[2] = { 0, 0 };

static void *lotus_bytes_alloc_fail_sentinel(void) {
    return (void *)g_bytes_alloc_fail_sentinel;
}

int64_t lotus_bytes_is_alloc_fail(const void *blob) {
    return blob == (const void *)g_bytes_alloc_fail_sentinel ? 1 : 0;
}

/*
 * Phase 2g: binary-safe TCP recv. Mirrors lotus_tcp_recv_str's
 * allocation + read(2) shape but builds a Bytes blob (length
 * prefix + body) instead of a NUL-terminated string, so embedded
 * NUL bytes survive intact. The blob is anchored in the lazy
 * global payload arena, matching the lifetime convention of
 * lotus_fs_read_bytes_global — callers can stash the pointer
 * past the call site without m49 deep-copy plumbing.
 *
 * Returns a Bytes blob with length 0 on fd/cap errors or EOF;
 * the caller distinguishes "empty" from "error" via the explicit
 * length, since the truncate-on-NUL hazard that motivated this
 * primitive is exactly the case where length-on-the-wire matters.
 */
void *lotus_tcp_recv_bytes(int fd, int max_bytes) {
    if (fd < 0 || max_bytes <= 0) {
        return lotus_bytes_empty_global();
    }
    /* F.35 Slice 3: same async_io park-on-EAGAIN dance as
     * accept_one. Off-pool callers (pinned threads, main) keep
     * the classic blocking read. */
    int async = lotus_io_on_async_io_pool();
    if (async) {
        lotus_io_set_nonblock(fd);
    }
    /* Allocate the body at the cap, read into it, then patch the
     * length prefix down to the actual bytes read. lotus_bytes_create
     * sets prefix=cap initially; partial reads (the common case)
     * need the prefix corrected so callers see the true length. */
    void *blob = lotus_caller_or_global_bytes_create((int64_t)max_bytes);
    if (!blob) {
        return lotus_bytes_empty_global();
    }
    char *body = (char *)lotus_bytes_data(blob);
    ssize_t n;
    for (;;) {
        n = read(fd, body, (size_t)max_bytes);
        if (n >= 0) break;
        if (errno == EINTR) continue;
        if (async && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            if (lotus_coop_park_on_fd(fd, EPOLLIN) == 0) continue;
        }
        /* read error: hand back an empty Bytes so downstream code
         * sees length=0 and can detect "nothing read". The reserved
         * arena memory leaks until program exit (matches recv_str's
         * convention). */
        return lotus_bytes_empty_global();
    }
    /* Patch the length prefix down to the actual bytes read. */
    *(int64_t *)blob = (int64_t)n;
    return blob;
}

/*
 * Phase 2g: Bytes → String conversion. Allocates a (len+1)-byte
 * buffer in the global payload arena, memcpys the Bytes body
 * into it, and NUL-terminates. Embedded NUL bytes survive in
 * the buffer but the resulting String's strlen-based view will
 * truncate at the first one — callers who need NUL-safe handling
 * should stay in Bytes. The conversion is for the common case
 * of "received bytes I'm pretty sure are UTF-8 / ASCII and want
 * to treat as a String".
 */
const char *lotus_str_from_bytes(const void *b) {
    static const char empty[1] = { 0 };
    if (!b) return empty;
    int64_t len = lotus_bytes_len(b);
    if (len <= 0) return empty;
    lotus_arena_t *arena = lotus_caller_arena_or_global();
    if (!arena) return empty;
    char *buf = (char *)lotus_arena_alloc(arena, (size_t)len + 1, 1);
    if (!buf) return empty;
    memcpy(buf, (const char *)b + sizeof(int64_t), (size_t)len);
    buf[(size_t)len] = '\0';
    return buf;
}

/*
 * Phase 2g: String → Bytes conversion. strlen the source string,
 * allocate a Bytes blob of that length in the global payload
 * arena, memcpy the body. Symmetric inverse of lotus_str_from_bytes.
 * Useful for handing String data to send_bytes when the payload
 * is text but the protocol surface demands the binary-safe call.
 */
void *lotus_bytes_from_str(const char *s) {
    if (!s) {
        return lotus_bytes_empty_global();
    }
    int64_t len = (int64_t)strlen(s);
    lotus_arena_t *arena = lotus_caller_arena_or_global();
    if (!arena) return lotus_bytes_empty_global();
    void *blob = lotus_bytes_create(arena, len);
    if (!blob) {
        return lotus_bytes_empty_global();
    }
    memcpy(lotus_bytes_data(blob), s, (size_t)len);
    return blob;
}

/*
 * Phase 2g: byte-as-Int accessor — returns the i-th byte's
 * unsigned value (0..255) sign-extended into an Int (i64). Used
 * by binary protocol parsers (WebSocket frame headers, framing
 * length fields, etc.) that need to peek at a single byte. Out
 * of range (i < 0 or i >= len) returns -1 — bytes never go
 * negative on read, so -1 is a clean sentinel.
 */
int64_t lotus_bytes_at(const void *b, int64_t i) {
    if (!b) return -1;
    int64_t len = lotus_bytes_len(b);
    if (i < 0 || i >= len) return -1;
    const unsigned char *body =
        (const unsigned char *)b + sizeof(int64_t);
    return (int64_t)body[i];
}

/*
 * Phase 2g: Bytes slice — returns a fresh Bytes blob containing
 * the half-open range [lo, hi). Out-of-range bounds clamp to the
 * source length; hi <= lo yields an empty blob. The result is a
 * copy (not a view) so it composes with deep-copy-shaped lifetime
 * conventions; anchored in the global payload arena.
 */
void *lotus_bytes_slice(const void *b, int64_t lo, int64_t hi) {
    if (!b) return lotus_bytes_empty_global();
    int64_t len = lotus_bytes_len(b);
    if (lo < 0) lo = 0;
    if (hi > len) hi = len;
    if (hi <= lo) return lotus_bytes_empty_global();
    int64_t out_len = hi - lo;
    lotus_arena_t *arena = lotus_caller_arena_or_global();
    if (!arena) return lotus_bytes_empty_global();
    void *blob = lotus_bytes_create(arena, out_len);
    if (!blob) return lotus_bytes_empty_global();
    memcpy(
        lotus_bytes_data(blob),
        (const char *)b + sizeof(int64_t) + lo,
        (size_t)out_len);
    return blob;
}

/*
 * ws-echo `bytes-construction-from-ints`: build a one-byte Bytes
 * blob from an Int (low 8 bits). Companion to the recv side
 * for outbound binary protocols (WebSocket frame headers,
 * length-encoded prefixes, etc.). Anchored in the program-
 * lifetime payload arena so the returned pointer matches the
 * lifetime conventions of recv_bytes / bytes_slice. The Int
 * argument is taken mod 256 — callers that pre-mask explicitly
 * are no-ops; callers passing larger ints lose the high bits
 * silently, matching how `b << 8` truncates.
 */
void *lotus_bytes_from_int(int64_t v) {
    lotus_arena_t *arena = lotus_caller_arena_or_global();
    if (!arena) return lotus_bytes_empty_global();
    void *blob = lotus_bytes_create(arena, 1);
    if (!blob) return lotus_bytes_empty_global();
    unsigned char *body = (unsigned char *)lotus_bytes_data(blob);
    body[0] = (unsigned char)(v & 0xFF);
    return blob;
}

/*
 * ws-echo `bytes-construction-from-ints`: concatenate two Bytes
 * blobs into a fresh one. Composes with from_int to assemble
 * arbitrary outbound payloads (recursive: from_int + concat builds
 * any byte sequence). Either argument may be NULL/empty; the
 * result mirrors the non-empty side (or is empty if both are).
 */
void *lotus_bytes_concat(const void *a, const void *b) {
    int64_t la = a ? lotus_bytes_len(a) : 0;
    int64_t lb = b ? lotus_bytes_len(b) : 0;
    int64_t total = la + lb;
    lotus_arena_t *arena = lotus_caller_arena_or_global();
    if (!arena) return lotus_bytes_empty_global();
    void *blob = lotus_bytes_create(arena, total);
    if (!blob) return lotus_bytes_empty_global();
    char *body = (char *)lotus_bytes_data(blob);
    if (la > 0) {
        memcpy(body, (const char *)a + sizeof(int64_t), (size_t)la);
    }
    if (lb > 0) {
        memcpy(body + la, (const char *)b + sizeof(int64_t), (size_t)lb);
    }
    return blob;
}

/*
 * ws-echo `sha1-base64-missing`: SHA-1 of a Bytes blob,
 * returning the 20-byte digest as Bytes. Stand-alone
 * implementation per RFC 3174 to avoid pulling in OpenSSL
 * just for the WebSocket handshake. Single-shot API: no
 * streaming Update/Final pair; callers that need streaming
 * can build it on top.
 */
static uint32_t sha1_rotl(uint32_t v, int n) {
    return (v << n) | (v >> (32 - n));
}

void *lotus_crypto_sha1(const void *b) {
    int64_t len = b ? lotus_bytes_len(b) : 0;
    const unsigned char *msg =
        b ? (const unsigned char *)b + sizeof(int64_t) : NULL;

    uint32_t h0 = 0x67452301u;
    uint32_t h1 = 0xEFCDAB89u;
    uint32_t h2 = 0x98BADCFEu;
    uint32_t h3 = 0x10325476u;
    uint32_t h4 = 0xC3D2E1F0u;

    /* Build padded message: original + 0x80 + zeros + 8-byte big-endian
     * length (in bits). Total length is multiple of 64. */
    uint64_t bit_len = (uint64_t)len * 8u;
    int64_t padded_len = len + 1;   /* original + 0x80 */
    /* pad zeros until padded_len % 64 == 56 */
    int64_t mod = padded_len % 64;
    int64_t pad_zeros = (mod <= 56) ? (56 - mod) : (56 + 64 - mod);
    padded_len += pad_zeros + 8;     /* +8 for length field */

    unsigned char *buf = (unsigned char *)malloc((size_t)padded_len);
    if (!buf) return lotus_bytes_empty_global();
    if (len > 0) memcpy(buf, msg, (size_t)len);
    buf[len] = 0x80;
    for (int64_t i = len + 1; i < padded_len - 8; i++) buf[i] = 0;
    for (int i = 0; i < 8; i++) {
        buf[padded_len - 1 - i] = (unsigned char)(bit_len >> (i * 8));
    }

    for (int64_t off = 0; off < padded_len; off += 64) {
        uint32_t w[80];
        for (int i = 0; i < 16; i++) {
            w[i] = ((uint32_t)buf[off + i * 4 + 0] << 24)
                 | ((uint32_t)buf[off + i * 4 + 1] << 16)
                 | ((uint32_t)buf[off + i * 4 + 2] << 8)
                 | ((uint32_t)buf[off + i * 4 + 3]);
        }
        for (int i = 16; i < 80; i++) {
            w[i] = sha1_rotl(w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16], 1);
        }
        uint32_t a = h0, ba = h1, c = h2, d = h3, e = h4;
        for (int i = 0; i < 80; i++) {
            uint32_t f, k;
            if (i < 20)      { f = (ba & c) | (~ba & d);         k = 0x5A827999u; }
            else if (i < 40) { f = ba ^ c ^ d;                    k = 0x6ED9EBA1u; }
            else if (i < 60) { f = (ba & c) | (ba & d) | (c & d); k = 0x8F1BBCDCu; }
            else             { f = ba ^ c ^ d;                    k = 0xCA62C1D6u; }
            uint32_t temp = sha1_rotl(a, 5) + f + e + k + w[i];
            e = d;
            d = c;
            c = sha1_rotl(ba, 30);
            ba = a;
            a = temp;
        }
        h0 += a; h1 += ba; h2 += c; h3 += d; h4 += e;
    }
    free(buf);

    void *blob = lotus_caller_or_global_bytes_create(20);
    if (!blob) return lotus_bytes_empty_global();
    unsigned char *dgst = (unsigned char *)lotus_bytes_data(blob);
    uint32_t hs[5] = { h0, h1, h2, h3, h4 };
    for (int i = 0; i < 5; i++) {
        dgst[i * 4 + 0] = (unsigned char)(hs[i] >> 24);
        dgst[i * 4 + 1] = (unsigned char)(hs[i] >> 16);
        dgst[i * 4 + 2] = (unsigned char)(hs[i] >> 8);
        dgst[i * 4 + 3] = (unsigned char)(hs[i]);
    }
    return blob;
}

/*
 * C3 (pond follow-up): SHA-256 per FIPS 180-4 of a Bytes blob,
 * returning the 32-byte digest as Bytes. Stand-alone — no
 * libcrypto / OpenSSL dependency. Single-shot API; callers that
 * need streaming can build it on top. Mirrors the lotus_crypto_sha1
 * surface (anchored in the bus payload arena).
 */
static const uint32_t lotus_sha256_k[64] = {
    0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u,
    0x3956c25bu, 0x59f111f1u, 0x923f82a4u, 0xab1c5ed5u,
    0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u,
    0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u,
    0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu,
    0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
    0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u,
    0xc6e00bf3u, 0xd5a79147u, 0x06ca6351u, 0x14292967u,
    0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u,
    0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u,
    0xa2bfe8a1u, 0xa81a664bu, 0xc24b8b70u, 0xc76c51a3u,
    0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
    0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u,
    0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu, 0x682e6ff3u,
    0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u,
    0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u,
};

static uint32_t lotus_sha256_rotr(uint32_t v, int n) {
    return (v >> n) | (v << (32 - n));
}

/* Core SHA-256 compute: hash `msg[0..len]` into `out[0..32]`.
 * `out` must point to a writable 32-byte buffer. Pure function;
 * no arena allocation here so HMAC can reuse it. */
static void lotus_sha256_compute(const unsigned char *msg,
                                  int64_t len,
                                  unsigned char out[32]) {
    uint32_t h0 = 0x6a09e667u;
    uint32_t h1 = 0xbb67ae85u;
    uint32_t h2 = 0x3c6ef372u;
    uint32_t h3 = 0xa54ff53au;
    uint32_t h4 = 0x510e527fu;
    uint32_t h5 = 0x9b05688cu;
    uint32_t h6 = 0x1f83d9abu;
    uint32_t h7 = 0x5be0cd19u;

    /* Padded message: original + 0x80 + zeros + 8-byte BE bit-length.
     * Total length a multiple of 64. */
    uint64_t bit_len = (uint64_t)len * 8u;
    int64_t padded_len = len + 1;
    int64_t mod = padded_len % 64;
    int64_t pad_zeros = (mod <= 56) ? (56 - mod) : (56 + 64 - mod);
    padded_len += pad_zeros + 8;

    unsigned char *buf = (unsigned char *)malloc((size_t)padded_len);
    if (!buf) {
        /* Out-of-memory: fall back to all-zero digest. Same shape
         * as the sha1 path's empty-bytes guard — recoverable in
         * principle by callers checking len. */
        memset(out, 0, 32);
        return;
    }
    if (len > 0) memcpy(buf, msg, (size_t)len);
    buf[len] = 0x80;
    for (int64_t i = len + 1; i < padded_len - 8; i++) buf[i] = 0;
    for (int i = 0; i < 8; i++) {
        buf[padded_len - 1 - i] = (unsigned char)(bit_len >> (i * 8));
    }

    for (int64_t off = 0; off < padded_len; off += 64) {
        uint32_t w[64];
        for (int i = 0; i < 16; i++) {
            w[i] = ((uint32_t)buf[off + i * 4 + 0] << 24)
                 | ((uint32_t)buf[off + i * 4 + 1] << 16)
                 | ((uint32_t)buf[off + i * 4 + 2] << 8)
                 | ((uint32_t)buf[off + i * 4 + 3]);
        }
        for (int i = 16; i < 64; i++) {
            uint32_t s0 = lotus_sha256_rotr(w[i - 15], 7)
                       ^ lotus_sha256_rotr(w[i - 15], 18)
                       ^ (w[i - 15] >> 3);
            uint32_t s1 = lotus_sha256_rotr(w[i - 2], 17)
                       ^ lotus_sha256_rotr(w[i - 2], 19)
                       ^ (w[i - 2] >> 10);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }

        uint32_t a = h0, b = h1, c = h2, d = h3;
        uint32_t e = h4, f = h5, g = h6, hh = h7;
        for (int i = 0; i < 64; i++) {
            uint32_t S1 = lotus_sha256_rotr(e, 6)
                       ^ lotus_sha256_rotr(e, 11)
                       ^ lotus_sha256_rotr(e, 25);
            uint32_t ch = (e & f) ^ ((~e) & g);
            uint32_t temp1 = hh + S1 + ch + lotus_sha256_k[i] + w[i];
            uint32_t S0 = lotus_sha256_rotr(a, 2)
                       ^ lotus_sha256_rotr(a, 13)
                       ^ lotus_sha256_rotr(a, 22);
            uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
            uint32_t temp2 = S0 + maj;
            hh = g;
            g = f;
            f = e;
            e = d + temp1;
            d = c;
            c = b;
            b = a;
            a = temp1 + temp2;
        }
        h0 += a; h1 += b; h2 += c; h3 += d;
        h4 += e; h5 += f; h6 += g; h7 += hh;
    }
    free(buf);

    uint32_t hs[8] = { h0, h1, h2, h3, h4, h5, h6, h7 };
    for (int i = 0; i < 8; i++) {
        out[i * 4 + 0] = (unsigned char)(hs[i] >> 24);
        out[i * 4 + 1] = (unsigned char)(hs[i] >> 16);
        out[i * 4 + 2] = (unsigned char)(hs[i] >> 8);
        out[i * 4 + 3] = (unsigned char)(hs[i]);
    }
}

void *lotus_crypto_sha256(const void *b) {
    int64_t len = b ? lotus_bytes_len(b) : 0;
    const unsigned char *msg =
        b ? (const unsigned char *)b + sizeof(int64_t) : NULL;

    unsigned char digest[32];
    lotus_sha256_compute(msg, len, digest);

    void *blob = lotus_caller_or_global_bytes_create(32);
    if (!blob) return lotus_bytes_empty_global();
    memcpy(lotus_bytes_data(blob), digest, 32);
    return blob;
}

/*
 * C3 (pond follow-up): HMAC-SHA256 per RFC 2104.
 *   HMAC(K, m) = H((K' xor opad) || H((K' xor ipad) || m))
 * where K' = K padded to the block size (64 bytes for SHA-256):
 * zero-extended if |K| <= B, else H(K) zero-extended.
 *
 * Returns the 32-byte tag as Bytes anchored in the payload arena.
 */
void *lotus_crypto_hmac_sha256(const void *key_b, const void *msg_b) {
    const int B = 64; /* SHA-256 block size */

    int64_t klen = key_b ? lotus_bytes_len(key_b) : 0;
    const unsigned char *kraw =
        key_b ? (const unsigned char *)key_b + sizeof(int64_t) : NULL;
    int64_t mlen = msg_b ? lotus_bytes_len(msg_b) : 0;
    const unsigned char *mraw =
        msg_b ? (const unsigned char *)msg_b + sizeof(int64_t) : NULL;

    /* K' — key normalized to B bytes. */
    unsigned char kprime[64];
    memset(kprime, 0, B);
    if (klen > B) {
        unsigned char khash[32];
        lotus_sha256_compute(kraw, klen, khash);
        memcpy(kprime, khash, 32);
    } else if (klen > 0) {
        memcpy(kprime, kraw, (size_t)klen);
    }

    /* Inner: H((K' xor ipad) || m) */
    int64_t inner_len = (int64_t)B + mlen;
    unsigned char *inner_buf = (unsigned char *)malloc((size_t)inner_len);
    if (!inner_buf) return lotus_bytes_empty_global();
    for (int i = 0; i < B; i++) inner_buf[i] = kprime[i] ^ 0x36;
    if (mlen > 0) memcpy(inner_buf + B, mraw, (size_t)mlen);
    unsigned char inner_hash[32];
    lotus_sha256_compute(inner_buf, inner_len, inner_hash);
    free(inner_buf);

    /* Outer: H((K' xor opad) || inner_hash) */
    unsigned char outer_buf[64 + 32];
    for (int i = 0; i < B; i++) outer_buf[i] = kprime[i] ^ 0x5C;
    memcpy(outer_buf + B, inner_hash, 32);
    unsigned char tag[32];
    lotus_sha256_compute(outer_buf, B + 32, tag);

    void *blob = lotus_caller_or_global_bytes_create(32);
    if (!blob) return lotus_bytes_empty_global();
    memcpy(lotus_bytes_data(blob), tag, 32);
    return blob;
}

/*
 * 2026-05-27 — CRC32 (IEEE 802.3 reversed polynomial,
 * `0xEDB88320`). Init `0xFFFFFFFF`, final XOR
 * `0xFFFFFFFF` — the standard variant that zlib's
 * `crc32()` and Python's `binascii.crc32` return.
 * Returns the 4-byte checksum as i64 (caller can cast /
 * compare as needed). Stand-alone bit-at-a-time impl; no
 * lookup table to keep the binary section small. For
 * market-data-shape inputs (a few hundred bytes per
 * call, hundreds of calls per second) this is fast
 * enough; users on bulk paths can wrap a table-driven
 * variant on top of the same Bytes shape if needed.
 *
 * Verified against published test vectors:
 *     crc32("")          = 0x00000000
 *     crc32("a")         = 0xE8B7BE43
 *     crc32("abc")       = 0x352441C2
 *     crc32("123456789") = 0xCBF43926
 */
int64_t lotus_crypto_crc32(const void *b) {
    int64_t len = b ? lotus_bytes_len(b) : 0;
    const unsigned char *msg =
        b ? (const unsigned char *)b + sizeof(int64_t) : NULL;
    uint32_t crc = 0xFFFFFFFFu;
    for (int64_t i = 0; i < len; i++) {
        crc ^= (uint32_t)msg[i];
        for (int j = 0; j < 8; j++) {
            uint32_t mask = (uint32_t)(0u - (crc & 1u));
            crc = (crc >> 1) ^ (0xEDB88320u & mask);
        }
    }
    return (int64_t)(uint32_t)(crc ^ 0xFFFFFFFFu);
}

/*
 * ws-echo `sha1-base64-missing`: Base64 encode a Bytes blob,
 * returning a NUL-terminated String (standard alphabet,
 * with `=` padding to a multiple of 4). Anchored in the
 * payload arena.
 */
static const char b64_alpha[] =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

const char *lotus_text_base64_encode(const void *b) {
    int64_t len = b ? lotus_bytes_len(b) : 0;
    const unsigned char *src =
        b ? (const unsigned char *)b + sizeof(int64_t) : NULL;
    int64_t out_len = ((len + 2) / 3) * 4;

    char *out = (char *)lotus_bus_payload_arena_alloc((size_t)(out_len + 1), 1);
    if (!out) return "";

    int64_t i = 0, j = 0;
    while (i + 3 <= len) {
        uint32_t v = ((uint32_t)src[i] << 16)
                   | ((uint32_t)src[i + 1] << 8)
                   |  (uint32_t)src[i + 2];
        out[j + 0] = b64_alpha[(v >> 18) & 0x3F];
        out[j + 1] = b64_alpha[(v >> 12) & 0x3F];
        out[j + 2] = b64_alpha[(v >> 6) & 0x3F];
        out[j + 3] = b64_alpha[v & 0x3F];
        i += 3;
        j += 4;
    }
    int64_t rem = len - i;
    if (rem == 1) {
        uint32_t v = (uint32_t)src[i] << 16;
        out[j + 0] = b64_alpha[(v >> 18) & 0x3F];
        out[j + 1] = b64_alpha[(v >> 12) & 0x3F];
        out[j + 2] = '=';
        out[j + 3] = '=';
        j += 4;
    } else if (rem == 2) {
        uint32_t v = ((uint32_t)src[i] << 16) | ((uint32_t)src[i + 1] << 8);
        out[j + 0] = b64_alpha[(v >> 18) & 0x3F];
        out[j + 1] = b64_alpha[(v >> 12) & 0x3F];
        out[j + 2] = b64_alpha[(v >> 6) & 0x3F];
        out[j + 3] = '=';
        j += 4;
    }
    out[j] = '\0';
    return out;
}

/*
 * v1.x-16: base64::decode. Inverse of lotus_text_base64_encode.
 * Returns a Bytes blob anchored in the bus payload arena.
 * Whitespace inside the input is ignored (RFC 4648 §3.3 — many
 * MIME-style encoders insert line breaks). Strictly rejects any
 * non-alphabet, non-whitespace, non-padding character by
 * returning a zero-length Bytes blob. Returns the empty blob for
 * empty / NULL input as well — callers should treat that as
 * either "empty source" or "decode failed".
 */
static int b64_decode_char(int c) {
    if (c >= 'A' && c <= 'Z') return c - 'A';
    if (c >= 'a' && c <= 'z') return c - 'a' + 26;
    if (c >= '0' && c <= '9') return c - '0' + 52;
    if (c == '+') return 62;
    if (c == '/') return 63;
    return -1;
}

void *lotus_text_base64_decode(const char *s) {

    if (!s) {
        return lotus_caller_or_global_bytes_create(0);
    }

    /* Count alphabet chars only (skip whitespace). Padding counts
     * toward the group-of-4 alignment check. */
    size_t alpha_count = 0;
    size_t pad_count = 0;
    for (const char *p = s; *p; p++) {
        unsigned char c = (unsigned char)*p;
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r') continue;
        if (c == '=') { pad_count++; continue; }
        if (b64_decode_char(c) < 0) {
            return lotus_caller_or_global_bytes_create(0);
        }
        alpha_count++;
    }
    /* Total chars including padding must be a multiple of 4. */
    if ((alpha_count + pad_count) % 4 != 0) {
        return lotus_caller_or_global_bytes_create(0);
    }
    /* At most 2 padding chars. */
    if (pad_count > 2) {
        return lotus_caller_or_global_bytes_create(0);
    }
    /* Decoded length: each 4 input chars yield 3 bytes, minus padding. */
    int64_t total_chars = (int64_t)(alpha_count + pad_count);
    int64_t out_len = (total_chars / 4) * 3 - (int64_t)pad_count;
    if (out_len < 0) out_len = 0;

    void *blob = lotus_caller_or_global_bytes_create(out_len);
    if (!blob || out_len == 0) {
        return blob;
    }
    unsigned char *out = (unsigned char *)lotus_bytes_data(blob);

    uint32_t buf = 0;
    int bits = 0;
    int64_t j = 0;
    for (const char *p = s; *p; p++) {
        unsigned char c = (unsigned char)*p;
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r') continue;
        if (c == '=') break;
        int v = b64_decode_char(c);
        buf = (buf << 6) | (uint32_t)v;
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            if (j < out_len) {
                out[j++] = (unsigned char)((buf >> bits) & 0xFF);
            }
        }
    }
    return blob;
}

/*
 * ws-echo `random-seed-missing`: minimal RNG surface. xorshift64*
 * seeded from monotonic time (cheap, library-internal use only
 * — NOT cryptographic). Suitable for nonces, retry jitter, test
 * shuffles. Single shared state guarded by a mutex; v1 doesn't
 * try to be thread-safe-fast.
 */
static uint64_t g_rand_state = 0;
static pthread_mutex_t g_rand_mutex = PTHREAD_MUTEX_INITIALIZER;

void lotus_rand_seed_from_time(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    uint64_t s = (uint64_t)ts.tv_sec * 1000000000ULL
               + (uint64_t)ts.tv_nsec;
    if (s == 0) s = 0x9E3779B97F4A7C15ULL;     /* avoid 0 */
    pthread_mutex_lock(&g_rand_mutex);
    g_rand_state = s;
    pthread_mutex_unlock(&g_rand_mutex);
}

int64_t lotus_rand_next_int(int64_t max) {
    pthread_mutex_lock(&g_rand_mutex);
    if (g_rand_state == 0) {
        /* Auto-seed on first use so callers that forget the
         * explicit seed still get distinct values per process. */
        struct timespec ts;
        clock_gettime(CLOCK_MONOTONIC, &ts);
        g_rand_state = (uint64_t)ts.tv_sec * 1000000000ULL
                     + (uint64_t)ts.tv_nsec;
        if (g_rand_state == 0) g_rand_state = 0x9E3779B97F4A7C15ULL;
    }
    /* xorshift64* */
    uint64_t x = g_rand_state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    g_rand_state = x;
    uint64_t mixed = x * 0x2545F4914F6CDD1DULL;
    pthread_mutex_unlock(&g_rand_mutex);
    if (max <= 0) return 0;
    return (int64_t)(mixed % (uint64_t)max);
}

/*
 * C7 (pond follow-up): wall-clock seconds since the Unix epoch.
 * Backs `std::time::now() -> Int`. CLOCK_REALTIME is reserved for
 * observation only (NTP slewing / leap seconds can warp the
 * value); CLOCK_MONOTONIC stays the basis for scheduling. Returns
 * tv_sec verbatim — sub-second precision lives in the future
 * `std::time::now_ns` if a consumer surfaces it.
 */
int64_t lotus_time_now_seconds(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (int64_t)ts.tv_sec;
}

/*
 * Construct a Time value from epoch seconds. v0 Time at codegen
 * level is a NUL-terminated ISO 8601 string in UTC (matches the
 * compile-time backtick-literal form). Allocates a 24-byte buffer
 * in the caller arena and fills it via gmtime_r + strftime.
 * Returns "" on any failure (gmtime returns NULL, strftime
 * truncates, arena alloc fails).
 */
const char *lotus_time_from_unix(int64_t n) {
    static const char empty[1] = { 0 };
    lotus_arena_t *arena = lotus_caller_arena_or_global();
    if (!arena) return empty;
    char *buf = (char *)lotus_arena_alloc(arena, 24, 1);
    if (!buf) return empty;
    time_t t = (time_t)n;
    struct tm tm;
    if (!gmtime_r(&t, &tm)) {
        buf[0] = '\0';
        return buf;
    }
    size_t k = strftime(buf, 24, "%Y-%m-%dT%H:%M:%SZ", &tm);
    if (k == 0) {
        buf[0] = '\0';
    }
    return buf;
}

/*
 * Phase 2e: index-API surface over the existing
 * lotus_fs_list_dir_global() cache. Returning a real `[String]`
 * waits on dynamic-array codegen support; meanwhile the
 * count + at pair drops every list_dir caller's iteration loop
 * from the manual `index_of("\n") + slice + advance` pattern to
 * a clean while-loop bounded by count. Both walk the cached
 * newline-joined blob, so amortised cost is linear in total
 * bytes once across both calls (no re-stat per entry).
 *
 * Filenames with embedded `\n` are still ill-defined at this
 * substrate — same limitation as list_dir itself (POSIX permits
 * `\n` in path segments; v0 documents the limitation and chooses
 * the simpler newline-joined cache).
 */
int64_t lotus_fs_list_dir_count(const char *path) {
    if (!path) return 0;
    const char *blob = lotus_fs_list_dir_global(path);
    if (!blob || !*blob) return 0;
    /* The cache shape is `entry\nentry\n...\n`. Count the newlines;
     * the last entry always carries a trailing newline (see
     * lotus_fs_list_dir's emit loop). */
    int64_t n = 0;
    for (const char *p = blob; *p; p++) {
        if (*p == '\n') n++;
    }
    return n;
}

const char *lotus_fs_list_dir_at(const char *path, int64_t idx) {
    static const char empty[1] = { 0 };
    if (!path || idx < 0) return empty;
    const char *blob = lotus_fs_list_dir_global(path);
    if (!blob || !*blob) return empty;
    /* Walk to the start of the idx-th entry. */
    const char *p = blob;
    for (int64_t k = 0; k < idx; k++) {
        const char *nl = strchr(p, '\n');
        if (!nl) return empty;
        p = nl + 1;
        if (!*p) return empty;
    }
    /* p points at the start of the idx-th entry. Find its
     * terminating newline and copy the slice into the global
     * payload arena so the returned String outlives the call. */
    const char *end = strchr(p, '\n');
    if (!end) return empty;
    size_t len = (size_t)(end - p);
    char *out = (char *)lotus_bus_payload_arena_alloc(len + 1, 1);
    if (!out) return empty;
    memcpy(out, p, len);
    out[len] = '\0';
    return out;
}

/*
 * C9 (pond/agent/sandbox): race-free tempfile-path allocator.
 * Assembles `prefix + "XXXXXX" + suffix` into a writable buffer,
 * calls mkstemps(3) to atomically open+create the file (mode
 * 0600) with the XXXXXX template substituted, immediately closes
 * the fd, and returns the resulting path string anchored in the
 * lazy global payload arena. NULL on error (errno set; EINVAL on
 * NULL args, ENOMEM on alloc failure, anything mkstemps can set
 * on its own — typically ENOENT/EACCES on the prefix dir).
 *
 * The caller owns cleanup — they wanted a path, not an fd —
 * matching the pond friction-log ask and the standard mktemp(3)
 * shape. There IS a window between our close() and the caller's
 * reopen (an attacker with write-access to the parent dir could
 * unlink + replace), but the pond contract is "race-free path
 * allocation" rather than "race-free path lifecycle" — that's
 * the standard mktemp shape and the friction-log ask explicitly
 * names this contract. Callers needing a held-open fd should
 * grow a sibling `mkstemp_fd` primitive later.
 */
const char *lotus_fs_mktemp(const char *prefix, const char *suffix) {
    if (!prefix || !suffix) {
        errno = EINVAL;
        return NULL;
    }
    size_t plen = strlen(prefix);
    size_t slen = strlen(suffix);
    /* prefix + "XXXXXX" + suffix + NUL */
    size_t total = plen + 6 + slen + 1;
    char *tmpl = (char *)malloc(total);
    if (!tmpl) {
        errno = ENOMEM;
        return NULL;
    }
    memcpy(tmpl, prefix, plen);
    memcpy(tmpl + plen, "XXXXXX", 6);
    memcpy(tmpl + plen + 6, suffix, slen);
    tmpl[total - 1] = '\0';
    int fd = mkstemps(tmpl, (int)slen);
    if (fd < 0) {
        int saved = errno;
        free(tmpl);
        errno = saved;
        return NULL;
    }
    close(fd);
    /* Anchor the assembled path in the caller's arena (TLS) or
     * the capped global fallback, then drop the malloc buffer. */
    char *out = lotus_str_clone(lotus_caller_arena_or_global(), tmpl);
    free(tmpl);
    if (!out) {
        errno = ENOMEM;
        return NULL;
    }
    return out;
}

void lotus_bus_remote_destroy_all(void) {
    for (size_t i = 0; i < g_bus_remote_count; i++) {
        lotus_bus_remote_entry_t *e = g_bus_remote_entries[i];

        /* Wave B: adapter entries own their protocol lifecycle
         * through the adapter locus's own dissolve method, which
         * fires via the regular locus dissolve cascade at program
         * exit. No transport to destroy or reader thread to join
         * here — only the strdup'd subject string to free. */
        if (e->kind == LOTUS_BUS_REMOTE_KIND_ADAPTER) {
            if (e->subject) free(e->subject);
            free(e);
            continue;
        }

        /* 2026-05-26 — UDP entries. Two-step shutdown:
         *   1. shutdown(SHUT_RDWR) unblocks a recvfrom-blocked
         *      reader thread (close alone doesn't on Linux —
         *      the file's refcount is still held by the blocked
         *      thread, so the recvfrom doesn't see EBADF until
         *      the next call).
         *   2. pthread_join after the reader's recvfrom returns
         *      n == 0 (its exit signal).
         *   3. close releases the fd.
         * IP_DROP_MEMBERSHIP is unnecessary pre-close (kernel
         * drops on close). */
        if (e->kind == LOTUS_BUS_REMOTE_KIND_UDP) {
            if (e->udp_fd >= 0) {
                shutdown(e->udp_fd, SHUT_RDWR);
            }
            if (e->has_reader_thread) {
                pthread_join(e->reader_thread, NULL);
            }
            if (e->udp_fd >= 0) {
                close(e->udp_fd);
                e->udp_fd = -1;
            }
            if (e->subject) free(e->subject);
            free(e);
            continue;
        }

        /* m59: for LISTEN role, the reader thread owns the
         * transport's lifecycle. Best-effort shutdown(conn_fd)
         * to unblock recv if the peer hasn't closed yet, then
         * join. The thread destroys the transport itself before
         * exiting, so we don't double-destroy here. */
        if (e->has_reader_thread) {
            if (e->transport && e->transport->conn_fd >= 0) {
                /* SHUT_RDWR turns subsequent recvs into
                 * immediate EOF. Ignore errors — if the peer has
                 * already closed (the common case in a clean
                 * teardown), the fd may already be half-shut. */
                shutdown(e->transport->conn_fd, SHUT_RDWR);
            }
            pthread_join(e->reader_thread, NULL);
            /* Reader thread has already nulled e->transport on
             * its way out, but if it failed before storing
             * (transport_create returned NULL), the field is
             * already NULL — so the CONNECT-style destroy below
             * is a no-op for this slot. */
        }
        if (e->transport) {
            lotus_transport_destroy(e->transport);
        }
        if (e->subject) {
            free(e->subject);
        }
        free(e);
    }
    if (g_bus_remote_entries) free(g_bus_remote_entries);
    g_bus_remote_entries = NULL;
    g_bus_remote_count   = 0;
    g_bus_remote_cap     = 0;

    /* m70: tear down the lazy payload arena (used by deserialize
     * to allocate String byte storage that survives the reader-
     * thread → dispatch → handler chain). Created on first use
     * via lotus_bus_payload_arena_alloc; destroyed here at
     * program shutdown alongside the rest of the bus tables. */
    pthread_mutex_lock(&g_bus_payload_arena_mutex);
    if (g_bus_payload_arena) {
        lotus_arena_destroy(g_bus_payload_arena);
        g_bus_payload_arena = NULL;
    }
    pthread_mutex_unlock(&g_bus_payload_arena_mutex);
}

/*
 * v1.x: ASCII case folding. `lower(s)` / `upper(s)` allocate a
 * new NUL-terminated string in the bus payload arena (same
 * lifetime class as parse_int etc.) and copy the input byte-by-
 * byte with the standard ASCII case shift. Non-ASCII bytes pass
 * through unchanged (utf-8 case folding is intentionally NOT
 * attempted at v1 — locale-correct folding requires Unicode
 * tables far heavier than the runtime currently carries).
 */
const char *lotus_str_lower(const char *s) {
    if (!s) return "";
    size_t n = strlen(s);
    char *out = (char *)lotus_bus_payload_arena_alloc(n + 1, 1);
    if (!out) return "";
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        out[i] = (c >= 'A' && c <= 'Z') ? (char)(c + 32) : (char)c;
    }
    out[n] = '\0';
    return out;
}

const char *lotus_str_trim(const char *s) {
    if (!s) return "";
    /* Whitespace per RFC 7230 / common usage: space, tab, \r, \n. */
    size_t n = strlen(s);
    size_t lo = 0;
    while (lo < n) {
        unsigned char c = (unsigned char)s[lo];
        if (c == ' ' || c == '\t' || c == '\r' || c == '\n') {
            lo++;
        } else {
            break;
        }
    }
    size_t hi = n;
    while (hi > lo) {
        unsigned char c = (unsigned char)s[hi - 1];
        if (c == ' ' || c == '\t' || c == '\r' || c == '\n') {
            hi--;
        } else {
            break;
        }
    }
    size_t out_len = hi - lo;
    char *out = (char *)lotus_bus_payload_arena_alloc(out_len + 1, 1);
    if (!out) return "";
    if (out_len > 0) {
        memcpy(out, s + lo, out_len);
    }
    out[out_len] = '\0';
    return out;
}

/*
 * 2026-05-17 — substring extraction. Returns s[lo..hi) clamped
 * to the input's byte range; negative lo / hi past the end /
 * inverted bounds all collapse to "". Operates on raw bytes
 * (same shape as bytes::slice + str::from_bytes composed), so
 * non-ASCII multi-byte sequences are split at byte boundaries
 * — slice high-byte ASCII / Bytes via std::bytes::slice if you
 * need codepoint discipline. Result lives in the global payload
 * arena.
 */
const char *lotus_str_substring(const char *s, int64_t lo, int64_t hi) {
    if (!s) return "";
    int64_t n = (int64_t)strlen(s);
    if (lo < 0) lo = 0;
    if (hi > n) hi = n;
    if (lo >= hi) return "";
    size_t out_len = (size_t)(hi - lo);
    char *out = (char *)lotus_bus_payload_arena_alloc(out_len + 1, 1);
    if (!out) return "";
    memcpy(out, s + lo, out_len);
    out[out_len] = '\0';
    return out;
}

/*
 * Replace every occurrence of `needle` with `replacement` in `s`.
 * Naive O(n*m) scan. Empty needle returns `s` unchanged (replacing
 * "" infinitely is undefined). Overlap is greedy-forward — each
 * match advances by `needle_len`, not 1.
 */
const char *lotus_str_replace(const char *s, const char *needle,
                              const char *replacement) {
    if (!s) return "";
    if (!needle || !*needle) {
        /* No-op for empty needle. */
        size_t n = strlen(s);
        char *out = (char *)lotus_bus_payload_arena_alloc(n + 1, 1);
        if (!out) return "";
        memcpy(out, s, n);
        out[n] = '\0';
        return out;
    }
    if (!replacement) replacement = "";
    size_t s_len   = strlen(s);
    size_t need    = strlen(needle);
    size_t rep_len = strlen(replacement);

    /* Count occurrences first to right-size the output. */
    size_t count = 0;
    for (size_t i = 0; i + need <= s_len; ) {
        if (memcmp(s + i, needle, need) == 0) {
            count++;
            i += need;
        } else {
            i++;
        }
    }
    size_t out_len;
    if (rep_len >= need) {
        out_len = s_len + count * (rep_len - need);
    } else {
        out_len = s_len - count * (need - rep_len);
    }

    char *out = (char *)lotus_bus_payload_arena_alloc(out_len + 1, 1);
    if (!out) return "";

    size_t j = 0;
    for (size_t i = 0; i < s_len; ) {
        if (i + need <= s_len && memcmp(s + i, needle, need) == 0) {
            memcpy(out + j, replacement, rep_len);
            j += rep_len;
            i += need;
        } else {
            out[j++] = s[i++];
        }
    }
    out[out_len] = '\0';
    return out;
}

/*
 * Repeat `s` n times, concatenated. Negative or zero n returns
 * the empty string. NULL s is treated as "". Result is anchored
 * in the bus payload arena.
 */
const char *lotus_str_repeat(const char *s, int64_t n) {
    if (!s || n <= 0) {
        return "";
    }
    size_t sl = strlen(s);
    if (sl == 0) return "";
    size_t total = sl * (size_t)n;
    char *out = (char *)lotus_bus_payload_arena_alloc(total + 1, 1);
    if (!out) return "";
    for (int64_t i = 0; i < n; i++) {
        memcpy(out + i * sl, s, sl);
    }
    out[total] = '\0';
    return out;
}

/*
 * Pad `s` on the LEFT with `pad` until total length is `width`.
 * If `s` is already >= width, returns `s` unchanged (no truncation).
 * `pad` must be a single ASCII char (uses first byte). Common
 * shape for right-aligning numbers in column output.
 */
const char *lotus_str_pad_left(const char *s, int64_t width, const char *pad) {
    if (!s) s = "";
    size_t sl = strlen(s);
    if ((int64_t)sl >= width) {
        /* Already wide enough — return unchanged (arena-copy so the
         * caller doesn't need to distinguish own-vs-borrow). */
        size_t n = sl;
        char *out = (char *)lotus_bus_payload_arena_alloc(n + 1, 1);
        if (!out) return "";
        memcpy(out, s, n);
        out[n] = '\0';
        return out;
    }
    char ch = (pad && *pad) ? *pad : ' ';
    size_t pad_count = (size_t)width - sl;
    size_t total = (size_t)width;
    char *out = (char *)lotus_bus_payload_arena_alloc(total + 1, 1);
    if (!out) return "";
    memset(out, ch, pad_count);
    memcpy(out + pad_count, s, sl);
    out[total] = '\0';
    return out;
}

/*
 * Pad `s` on the RIGHT with `pad` until total length is `width`.
 * Same shape as pad_left but the pad bytes go on the trailing side.
 * Common for left-aligning columns in table output.
 */
const char *lotus_str_pad_right(const char *s, int64_t width, const char *pad) {
    if (!s) s = "";
    size_t sl = strlen(s);
    size_t total = ((int64_t)sl >= width) ? sl : (size_t)width;
    char *out = (char *)lotus_bus_payload_arena_alloc(total + 1, 1);
    if (!out) return "";
    memcpy(out, s, sl);
    if (total > sl) {
        char ch = (pad && *pad) ? *pad : ' ';
        memset(out + sl, ch, total - sl);
    }
    out[total] = '\0';
    return out;
}

const char *lotus_str_upper(const char *s) {
    if (!s) return "";
    size_t n = strlen(s);
    char *out = (char *)lotus_bus_payload_arena_alloc(n + 1, 1);
    if (!out) return "";
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        out[i] = (c >= 'a' && c <= 'z') ? (char)(c - 32) : (char)c;
    }
    out[n] = '\0';
    return out;
}

/*
 * v1.x-15: string-builder primitive. Resolves the reader-list_item-
 * quadratic-concat friction: long-running string accumulation can
 * now run in amortized O(N) total cost via doubling realloc, rather
 * than the O(N²) shape that `buf = buf + chunk` collapsed to under
 * Hale's arena-anchored immutable Strings.
 *
 * The builder is a single contiguous malloc'd buffer with a
 * length and capacity. append() doubles cap as needed. finish()
 * allocates the final NUL-terminated string in the bus payload
 * arena (so it stays live for the rest of the program), copies
 * the buffer into it, frees the builder, and returns the string.
 *
 * Leaks the builder if finish() is never called — acceptable for
 * v1 since the surface fences this off: every builder_new()
 * dominates a builder_finish() in practice, and the worst-case
 * "user forgot to finish" is bounded by the working-set size of
 * one accumulator scope.
 */
typedef struct lotus_str_builder {
    size_t cap;
    size_t len;
    char  *buf;
} lotus_str_builder_t;

void *lotus_str_builder_new(void) {
    lotus_str_builder_t *b = (lotus_str_builder_t *)
        malloc(sizeof(lotus_str_builder_t));
    if (!b) return NULL;
    b->cap = 64;
    b->len = 0;
    b->buf = (char *)malloc(b->cap);
    if (!b->buf) {
        free(b);
        return NULL;
    }
    b->buf[0] = '\0';
    return b;
}

void lotus_str_builder_append(void *handle, const char *s) {
    if (!handle || !s) return;
    lotus_str_builder_t *b = (lotus_str_builder_t *)handle;
    size_t add = strlen(s);
    if (add == 0) return;
    size_t need = b->len + add;
    if (need + 1 > b->cap) {
        size_t new_cap = b->cap ? b->cap : 64;
        while (new_cap < need + 1) {
            new_cap *= 2;
            /* Guard against overflow at unreasonable sizes. */
            if (new_cap < b->cap) {
                /* Saturate: allocate exactly what we need. */
                new_cap = need + 1;
                break;
            }
        }
        char *nb = (char *)realloc(b->buf, new_cap);
        if (!nb) return;
        b->buf = nb;
        b->cap = new_cap;
    }
    memcpy(b->buf + b->len, s, add);
    b->len = need;
    b->buf[b->len] = '\0';
}

int64_t lotus_str_builder_len(const void *handle) {
    if (!handle) return 0;
    const lotus_str_builder_t *b = (const lotus_str_builder_t *)handle;
    return (int64_t)b->len;
}

const char *lotus_str_builder_finish(void *handle) {
    if (!handle) return "";
    lotus_str_builder_t *b = (lotus_str_builder_t *)handle;
    /* Allocate via the TLS-routed helper. Don't hold the
     * payload-arena mutex around it (helper takes the same mutex
     * on the fallback path → self-deadlock). */
    char *out = (char *)lotus_bus_payload_arena_alloc(b->len + 1, 1);
    if (!out) {
        free(b->buf);
        free(b);
        return "";
    }
    memcpy(out, b->buf, b->len);
    out[b->len] = '\0';
    free(b->buf);
    free(b);
    return out;
}

/*
 * C10 (pond follow-up): binary-safe builder mirroring
 * lotus_str_builder_* but using the Bytes ABI on both sides.
 *
 * Append: takes a Bytes blob (reads `[i64 len]` prefix so embedded
 * NULs survive). Finish: emits a freshly allocated `[i64 len]
 * [u8 data[len]]` Bytes blob anchored in the bus payload arena.
 *
 * Memory layout (2026-05-19, Phase-2 (1)). Diverges from
 * lotus_str_builder_t to support zero-copy `view()`:
 *
 *   malloc'd region: [int64_t len][u8 data[cap]]
 *                                 ^
 *                                 buf
 *
 * The data region is preceded inline by a Bytes-ABI length prefix.
 * `view()` returns `buf - 8` — a pointer that lotus_bytes_len /
 * lotus_bytes_at can dereference directly with no copy. Append /
 * shift_front / clear all update the prefix in sync with the data
 * mutation. The header `{cap, buf}` is small; the prefix lives
 * inline with the data so the view points at one contiguous block.
 *
 * Trade-off vs the prior `{cap, len, buf*}` shape: `len` now lives
 * at `*(int64_t*)(buf - 8)` instead of as a header field. Every
 * mutation reads/writes through one extra pointer indirection.
 * Negligible overhead at our scale; the structural win is the
 * zero-allocation `view()` that unblocks ~70 KB/s of the residual
 * pond/websocket leak.
 *
 * Pond consumers: pond/http/client + pond/agent/llm were
 * accumulating message bodies through std::str::builder_* +
 * std::bytes::from_string — lossy on chunks containing NUL.
 * std::bytes::builder_* is the single-step binary-safe path.
 */
typedef struct lotus_bytes_builder {
    size_t cap;     /* capacity of the data area (excludes the
                       8-byte length prefix AND the trailing NUL
                       reservation). */
    char *buf;      /* points at data area; *(int64_t*)(buf - 8)
                       is the live length (Bytes ABI). The full
                       malloc'd region is `buf - 8 .. buf + cap + 1`
                       — one byte of NUL reserve past `cap` so
                       `text_view()` can return `buf` as a C
                       string. The invariant `buf[len] == '\0'` is
                       maintained by every mutating op. */
    /* F.30b: monotonic mutation counter. Bumped by every mutating
     * op (append, append_slice, shift_front, clear, finish).
     * view() and text_view() snapshot this into the returned
     * lotus_view_t's `epoch` field; read-site unpacking compares
     * against this live value and panics on mismatch. Catches
     * "mutated between view() and read" misuse loudly at the
     * read site. Only `+= 1` from 0, so the static sentinel
     * (`-1`) on view structs is never produced by a real
     * builder. */
    int64_t mutation_epoch;
} lotus_bytes_builder_t;

/* F.30b view-ABI compaction (2026-05-22 PM): the view is now a
 * 16-byte by-value struct. Was {data, builder, stamped_epoch}
 * (24B) heap-allocated per view() call — that arena_alloc was the
 * dominant residual chunk-allocation trigger in long-running
 * recv loops (~134 view()/sec in the a websocket peeler workload).
 *
 * Now: `src` is overloaded — either the builder pointer (when
 * `epoch >= 0`, i.e. a real builder view) or the static-lifetime
 * data pointer (when `epoch == LOTUS_VIEW_EPOCH_STATIC`, used for
 * literal defaults via lotus_view_from_static_data and the
 * null-handle path of builder_view / builder_text_view).
 *
 * Read-site helpers recompute the underlying data pointer from
 * `((lotus_bytes_builder_t*)v.src)->buf` at unpack time. mutation
 * counters are bumped only by `+= 1` from 0, so non-negative
 * values are always "real builder"; the sentinel `-1` is unreachable
 * by natural increment.
 *
 * The struct's `{void*, int64_t}` layout fits the SysV AMD64
 * ABI's ≤16-byte two-INTEGER-eightbytes return: both helper return
 * values land in `rax`/`rdx`, and by-value args land in two arg
 * registers. No memory traffic for views in the hot path. */
#define LOTUS_VIEW_EPOCH_STATIC ((int64_t)-1)

typedef struct lotus_view {
    void   *src;
    int64_t epoch;
} lotus_view_t;

/* Read the inline length prefix at `buf - 8`. */
static inline int64_t lotus_bb_len(const lotus_bytes_builder_t *b) {
    return *(const int64_t *)(b->buf - sizeof(int64_t));
}

/* Write the inline length prefix at `buf - 8`. */
static inline void lotus_bb_set_len(lotus_bytes_builder_t *b, int64_t n) {
    *(int64_t *)(b->buf - sizeof(int64_t)) = n;
}

void *lotus_bytes_builder_new(int64_t initial_cap) {
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)
        malloc(sizeof(lotus_bytes_builder_t));
    if (!b) return NULL;
    b->cap = initial_cap > 0 ? (size_t)initial_cap : 64;
    /* `+1` for trailing NUL reserve so text_view() can return
     * `buf` as a C string without copying — the invariant
     * `buf[len] == '\0'` is seeded here and maintained by every
     * mutating op below. */
    char *region = (char *)malloc(sizeof(int64_t) + b->cap + 1);
    if (!region) {
        free(b);
        return NULL;
    }
    b->buf = region + sizeof(int64_t);
    lotus_bb_set_len(b, 0);
    b->buf[0] = '\0';
    b->mutation_epoch = 0;
    return b;
}

/* F.30b (2026-05-20): noreturn panic helper invoked by the view-
 * unpack helpers when the stamped epoch on a view doesn't match
 * the live mutation_epoch on its source builder. Indicates the
 * source builder was mutated between view() and this read —
 * programmer error, not a recoverable runtime condition. Mirrors
 * lotus_root_panic's shape (stderr + non-zero exit). */
__attribute__((noreturn))
void lotus_view_stale_panic(const char *kind,
                            int64_t stamped, int64_t current) {
    fprintf(stderr,
            "violation: %s read after source BytesBuilder mutated "
            "(stamped_epoch=%lld, current=%lld). The view's source "
            "buffer changed between view() and read; the aliased "
            "data is no longer the same bytes you observed at "
            "view-creation time. v1 has no borrow checker, so this "
            "is enforced at runtime — restructure to either "
            "snapshot() before the mutation or take view() after "
            "the mutation completes.\n",
            kind, (long long)stamped, (long long)current);
    fflush(stderr);
    _exit(1);
}

/* F.30b read-site unpacking helpers. Codegen emits a call to one
 * of these at every BytesView/StringView read-coercion site. The
 * helper compares the view's epoch against the source builder's
 * live mutation_epoch and panics on mismatch; on the OK path
 * returns the underlying data pointer (Bytes-shaped for view(),
 * C-string for text_view()). View struct is passed by value (two
 * INTEGER eightbytes in arg registers per SysV ABI). */
void *lotus_bytes_view_data(lotus_view_t v) {
    if (v.epoch == LOTUS_VIEW_EPOCH_STATIC) {
        /* Static-lifetime view (built via lotus_view_from_static_data
         * or the null-handle path of builder_view). `src` is the
         * underlying data pointer; no epoch check. */
        return v.src ? v.src : lotus_bytes_empty_global();
    }
    if (!v.src) return lotus_bytes_empty_global();
    const lotus_bytes_builder_t *b = (const lotus_bytes_builder_t *)v.src;
    int64_t current = b->mutation_epoch;
    if (current != v.epoch) {
        lotus_view_stale_panic("BytesView", v.epoch, current);
    }
    /* Recompute data pointer at read time — `buf - 8` is the
     * `[i64 len][u8 data]` Bytes ABI shape. */
    return b->buf - sizeof(int64_t);
}

const char *lotus_str_view_data(lotus_view_t v) {
    static const char empty[1] = { 0 };
    if (v.epoch == LOTUS_VIEW_EPOCH_STATIC) {
        return v.src ? (const char *)v.src : empty;
    }
    if (!v.src) return empty;
    const lotus_bytes_builder_t *b = (const lotus_bytes_builder_t *)v.src;
    int64_t current = b->mutation_epoch;
    if (current != v.epoch) {
        lotus_view_stale_panic("StringView", v.epoch, current);
    }
    /* Recompute at read time — `buf[len] == '\0'` invariant
     * makes `buf` directly a C-string. */
    return (const char *)b->buf;
}

/* F.30b (5b): wrap a static-lifetime String/Bytes pointer in a
 * view struct for storage-site default coercion. The epoch
 * sentinel signals lotus_*_view_data that there's no epoch
 * check to run; `src` carries the data pointer directly. Used
 * at struct/locus field-init sites where the declared type is
 * StringView/BytesView and the initializer is a String/Bytes
 * literal (stored in the global string table, program-lifetime).
 *
 * Post-PM compaction: no arena allocation — the view is returned
 * by value. The pre-PM shape lotus_arena_alloc'd 16 bytes per
 * call; that allocation is gone. */
lotus_view_t lotus_view_from_static_data(void *data) {
    lotus_view_t v;
    v.src = data;
    v.epoch = LOTUS_VIEW_EPOCH_STATIC;
    return v;
}

/* Returns 1 on success, 0 on hard failure (null handle or realloc
 * NULL). Empty / null chunk is a no-op success. The success
 * indicator is what the BytesBuilder locus's `append` method
 * checks before deciding to `violate alloc_failed` (F.27 routing
 * for fatal alloc fail). Pre-2026-05-19 this returned void and
 * silently no-op'd on realloc fail; that silent corruption is
 * exactly what F.27 promotes to a structural violation. */
int64_t lotus_bytes_builder_append(void *handle, const void *chunk_blob) {
    if (!handle) return 0;
    if (!chunk_blob) return 1;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    /* Bytes ABI: `[i64 len][u8 data[len]]`. Read the explicit
     * length prefix — strlen would truncate at the first NUL. */
    int64_t add_signed = lotus_bytes_len(chunk_blob);
    if (add_signed <= 0) return 1;
    size_t add = (size_t)add_signed;
    int64_t cur_len = lotus_bb_len(b);
    size_t need = (size_t)cur_len + add;
    if (need > b->cap) {
        size_t new_cap = b->cap ? b->cap : 64;
        while (new_cap < need) {
            new_cap *= 2;
            if (new_cap < b->cap) {
                /* Overflow guard — saturate at the exact need. */
                new_cap = need;
                break;
            }
        }
        /* The malloc'd region is `[i64 len][data][NUL]`, so realloc
         * the full region (buf - 8) including the trailing NUL
         * reserve. The new region's len-prefix carries over via
         * realloc's copy of the head bytes. */
        char *new_region = (char *)realloc(b->buf - sizeof(int64_t),
                                           sizeof(int64_t) + new_cap + 1);
        if (!new_region) return 0;
        b->buf = new_region + sizeof(int64_t);
        b->cap = new_cap;
    }
    /* Pull the body bytes out of the Bytes blob (past the
     * length prefix) and append verbatim — NULs included. */
    const char *src = (const char *)chunk_blob + sizeof(int64_t);
    memcpy(b->buf + cur_len, src, add);
    lotus_bb_set_len(b, (int64_t)need);
    b->buf[need] = '\0';
    b->mutation_epoch += 1;
    return 1;
}

int64_t lotus_bytes_builder_len(const void *handle) {
    if (!handle) return 0;
    const lotus_bytes_builder_t *b = (const lotus_bytes_builder_t *)handle;
    return lotus_bb_len(b);
}

/* Phase-2 (1): non-owning Bytes view into the builder's `[i64 len]
 * [u8 data]` region. Lifetime: valid until the next append /
 * shift_front / clear / finish on the source builder. The pond
 * recv loop relies on this for `parse_frame` to read rx_buf via
 * std::bytes::at / len with zero copy into g_bus_payload_arena —
 * the dominant residual leak after Phase-1.
 *
 * The returned pointer aliases storage owned by the builder; the
 * caller must not retain it across a mutation. v1 has no borrow
 * checker, so the rule is documented-and-trusted — same shape as
 * the rest of the locus's lifetime story. */
/* F.30b view ABI: returns a 16-byte view struct by value (no
 * arena allocation in the hot path — that was the dominant
 * residual chunk-allocation trigger in long-running recv loops).
 * `src` is the builder pointer; the data pointer (`buf - 8`,
 * Bytes-shaped) is recomputed at read time by
 * lotus_bytes_view_data. `epoch` snapshots the builder's
 * mutation counter so read-site staleness checks see the
 * version the caller observed. */
lotus_view_t lotus_bytes_builder_view(void *handle) {
    lotus_view_t v;
    if (!handle) {
        /* Null-handle path: reuse the static sentinel so the
         * read site sees the empty-Bytes global and doesn't
         * dereference src as a builder. */
        v.src = lotus_bytes_empty_global();
        v.epoch = LOTUS_VIEW_EPOCH_STATIC;
        return v;
    }
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    v.src = b;
    v.epoch = b->mutation_epoch;
    return v;
}

/* Phase-3 Site 2: non-owning String view aliasing the builder's
 * buffer. The builder maintains `buf[len] == '\0'` after every
 * mutation so the recomputed read-time pointer (`b->buf`) is a
 * valid C string for strlen / printf / the lotus_str_* surface.
 * F.30b view-ABI compaction: 16-byte struct returned by value,
 * data pointer recomputed at unpack time by lotus_str_view_data. */
lotus_view_t lotus_bytes_builder_text_view(void *handle) {
    static const char empty[1] = { 0 };
    lotus_view_t v;
    if (!handle) {
        v.src = (void *)empty;
        v.epoch = LOTUS_VIEW_EPOCH_STATIC;
        return v;
    }
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    v.src = b;
    v.epoch = b->mutation_epoch;
    return v;
}

void *lotus_bytes_builder_finish(void *handle) {
    if (!handle) return lotus_bytes_alloc_fail_sentinel();
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    int64_t cur_len = lotus_bb_len(b);
    /* Allocate in the caller's arena (TLS) or the capped global
     * fallback. The helper does its own locking; do NOT hold the
     * payload-arena mutex around it (helper would self-deadlock on
     * the fallback path). */
    void *blob = lotus_caller_or_global_bytes_create(cur_len);
    if (!blob) {
        free(b->buf - sizeof(int64_t));
        free(b);
        return lotus_bytes_alloc_fail_sentinel();
    }
    if (cur_len > 0) {
        memcpy(lotus_bytes_data(blob), b->buf, (size_t)cur_len);
    }
    free(b->buf - sizeof(int64_t));
    free(b);
    return blob;
}

/*
 * Phase-0 in-place Bytes ops for long-lived recv-loop accumulators
 * (pond/websocket FRICTION § "per-frame Bytes allocations
 * accumulate"). The existing builder API was finish-once-and-done;
 * the WS recv loop needs a buffer that grows, drains from the
 * front, snapshots out a view, and disposes without producing a
 * final Bytes blob. These four ops extend the builder lifecycle:
 *
 *   builder_shift_front(b, n) — drop first n bytes via memmove,
 *                               capacity preserved.
 *   builder_clear(b)          — len=0, capacity preserved.
 *   builder_snapshot(b)       — copy current [0..len) into a
 *                               fresh Bytes blob in the bus
 *                               payload arena. Builder unchanged.
 *   builder_free(b)           — dispose the malloc-backed buffer
 *                               with no materialization. The
 *                               "leak unless finish" hazard the
 *                               old comment described is closed
 *                               for long-lived holders that
 *                               never call finish.
 */
void lotus_bytes_builder_shift_front(void *handle, int64_t n) {
    if (!handle) return;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    if (n <= 0) return;
    int64_t cur_len = lotus_bb_len(b);
    size_t drop = (size_t)n;
    if (drop >= (size_t)cur_len) {
        lotus_bb_set_len(b, 0);
        b->buf[0] = '\0';
        b->mutation_epoch += 1;
        return;
    }
    /* memmove handles overlap; src+drop > dst by construction. */
    memmove(b->buf, b->buf + drop, (size_t)cur_len - drop);
    int64_t new_len = cur_len - (int64_t)drop;
    lotus_bb_set_len(b, new_len);
    b->buf[new_len] = '\0';
    b->mutation_epoch += 1;
}

void lotus_bytes_builder_clear(void *handle) {
    if (!handle) return;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    lotus_bb_set_len(b, 0);
    b->buf[0] = '\0';
    b->mutation_epoch += 1;
}

void *lotus_bytes_builder_snapshot(void *handle) {
    if (!handle) return lotus_bytes_alloc_fail_sentinel();
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    int64_t cur_len = lotus_bb_len(b);
    void *blob = lotus_caller_or_global_bytes_create(cur_len);
    if (!blob) {
        return lotus_bytes_alloc_fail_sentinel();
    }
    if (cur_len > 0) {
        memcpy(lotus_bytes_data(blob), b->buf, (size_t)cur_len);
    }
    return blob;
}

void lotus_bytes_builder_free(void *handle) {
    if (!handle) return;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    free(b->buf - sizeof(int64_t));
    free(b);
}

/* Phase-3 Site 1 (2026-05-19): copy src[lo..hi) directly into the
 * builder's tail. Eliminates the `slice(b, lo, hi) + append(slice)`
 * pair that allocates a fresh Bytes wrapper in g_bus_payload_arena
 * per call — the dominant residual in pond/websocket's unmask_into
 * fast path.
 *
 * Return contract (2026-05-20 — OOB split, downstream FRICTION #5b
 * follow-up):
 *   1 ok
 *   0 alloc-fail (null handle OR realloc NULL)
 *  -1 out-of-range indices (lo < 0 / hi < lo / hi > src_len, or
 *                            non-empty range on a null src_blob)
 *
 * The Hale-side wrapper branches on the three states and
 * violates `alloc_failed` (with captures: initial_cap) or
 * `index_oob` (with captures: lo, hi) as appropriate. The earlier
 * shape collapsed both into 0 and routed everything through
 * `violate alloc_failed`, which misled downstream production
 * on_failure handlers — they read `captures.initial_cap` and
 * concluded "memory exhausted" when the real cause was a caller-
 * supplied bad index. */
int64_t lotus_bytes_builder_append_slice(void *handle,
                                         const void *src_blob,
                                         int64_t lo,
                                         int64_t hi) {
    if (!handle) return 0;
    /* Null src + non-empty range is structurally OOB (the caller
     * asserts `[lo, hi)` exists in a blob with no body). Null src +
     * empty range is a no-op success — matches the prior shape. */
    if (!src_blob) return (lo == 0 && hi == 0) ? 1 : -1;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    int64_t src_len = lotus_bytes_len(src_blob);
    if (lo < 0 || hi < lo || hi > src_len) return -1;
    int64_t add = hi - lo;
    if (add == 0) return 1;
    int64_t cur_len = lotus_bb_len(b);
    size_t need = (size_t)cur_len + (size_t)add;
    if (need > b->cap) {
        size_t new_cap = b->cap ? b->cap : 64;
        while (new_cap < need) {
            new_cap *= 2;
            if (new_cap < b->cap) {
                new_cap = need;
                break;
            }
        }
        char *new_region = (char *)realloc(b->buf - sizeof(int64_t),
                                           sizeof(int64_t) + new_cap + 1);
        if (!new_region) return 0;
        b->buf = new_region + sizeof(int64_t);
        b->cap = new_cap;
    }
    const char *src = (const char *)src_blob + sizeof(int64_t) + lo;
    memcpy(b->buf + cur_len, src, (size_t)add);
    lotus_bb_set_len(b, (int64_t)need);
    b->buf[need] = '\0';
    b->mutation_epoch += 1;
    return 1;
}

/*
 * Phase 1: caller-provided destination at the syscall layer. The
 * `lotus_*_recv_bytes` shapes allocate a fresh `[i64 len][body]`
 * blob in g_bus_payload_arena per call — the leak source flagged
 * by pond/websocket's recv loop (~480 KB/s on a high-rate JSON
 * feed). recv_into reads directly into the caller's builder
 * buffer. Grows the builder if its remaining headroom (cap - len)
 * is smaller than max_bytes; the builder's len is bumped by the
 * count read. Return semantics:
 *
 *   > 0  bytes appended to the builder
 *   = 0  peer closed cleanly (TCP) / zero-length datagram (UDP)
 *   < 0  fatal error; builder is unchanged
 *
 * Mirrors POSIX read(2) — partial reads are normal, the caller
 * loops or yields. EINTR retried internally. No allocation in
 * g_bus_payload_arena.
 *
 * The reserve / advance pair is exposed for lotus_tls.c (separate
 * translation unit) to implement lotus_tls_recv_into without
 * needing lotus_bytes_builder_t's layout.
 */
void *lotus_bytes_builder_reserve(void *handle, int64_t n) {
    if (!handle || n <= 0) return NULL;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    int64_t cur_len = lotus_bb_len(b);
    size_t need = (size_t)cur_len + (size_t)n;
    if (need > b->cap) {
        size_t new_cap = b->cap ? b->cap : 64;
        while (new_cap < need) {
            new_cap *= 2;
            if (new_cap < b->cap) {
                new_cap = need;
                break;
            }
        }
        char *new_region = (char *)realloc(b->buf - sizeof(int64_t),
                                           sizeof(int64_t) + new_cap);
        if (!new_region) return NULL;
        b->buf = new_region + sizeof(int64_t);
        b->cap = new_cap;
    }
    return b->buf + cur_len;
}

void lotus_bytes_builder_advance(void *handle, int64_t n) {
    if (!handle || n <= 0) return;
    lotus_bytes_builder_t *b = (lotus_bytes_builder_t *)handle;
    lotus_bb_set_len(b, lotus_bb_len(b) + n);
    b->mutation_epoch += 1;
}

int64_t lotus_tcp_recv_into(int fd, void *builder, int64_t max_bytes) {
    if (fd < 0 || max_bytes <= 0) return -1;
    char *tail = (char *)lotus_bytes_builder_reserve(builder, max_bytes);
    if (!tail) return -1;
    ssize_t n;
    for (;;) {
        n = read(fd, tail, (size_t)max_bytes);
        if (n >= 0) break;
        if (errno == EINTR) continue;
        return -1;
    }
    lotus_bytes_builder_advance(builder, (int64_t)n);
    return (int64_t)n;
}

int64_t lotus_udp_recv_into(int fd, void *builder, int64_t max_bytes) {
    if (fd < 0 || max_bytes <= 0) return -1;
    char *tail = (char *)lotus_bytes_builder_reserve(builder, max_bytes);
    if (!tail) return -1;
    /* Datagram boundaries preserved: a single recvfrom delivers
     * at most one datagram. EINTR retried; other errors fatal. */
    ssize_t n;
    for (;;) {
        n = recvfrom(fd, tail, (size_t)max_bytes, 0, NULL, NULL);
        if (n >= 0) break;
        if (errno == EINTR) continue;
        return -1;
    }
    lotus_bytes_builder_advance(builder, (int64_t)n);
    return (int64_t)n;
}

/*
 * v1.x-FORM-2 PR6: root-locus value-error panic.
 *
 * Called by codegen when an `or raise` is reached past every
 * enclosing fallible(E) frame — i.e., the value error has
 * escaped the implicit main locus's body. Today: report to
 * stderr and exit(1), reusing the same shape the closure-
 * violation bare-handler fallback uses. Architecturally the
 * seat for a future routing-through-main-locus-on_failure
 * extension; the typename arg is the discriminator a future
 * dispatch would key on, and the payload ptr / size are
 * carried opaquely now so that extension doesn't need an ABI
 * bump.
 */
void lotus_root_panic(
    const void *payload,
    size_t payload_size,
    const char *payload_typename
) {
    (void)payload;
    (void)payload_size;
    const char *tn = payload_typename ? payload_typename : "<unknown>";
    dprintf(2, "Hale panic: unhandled %s escaping main locus\n", tn);
    exit(1);
}

/*
 * C8 (pond follow-up): IEEE 754 sentinel / classification helpers.
 * Back `std::math::{nan, is_nan, inf}`. `std::math::tanh` does NOT
 * have a wrapper here — it resolves through a direct LLVM extern
 * (mirroring `sqrt` / `exp` / `log` / `floor` / `ceil` / `pow`) so
 * binaries that don't actually call `tanh` aren't burdened with
 * an unresolved libm reference (test helper binaries — bus_config,
 * transport, etc. — link `lotus_arena.c` without `-lm`, so any
 * libm symbol referenced from this file at compile time becomes
 * an unconditional load-bearing dependency).
 *
 * `nan` / `inf` / `is_nan` are SAFE here: they reference only the
 * `<math.h>` macros `NAN` / `INFINITY` (compile-time constants)
 * and the canonical `f != f` test, none of which touch libm at
 * link time.
 *
 * NaN-printing is platform-dependent (`nan` / `NaN` / `-nan` via
 * printf %g); agents test for NaN via `is_nan(x)`, not by
 * comparing the printed value. Driven by pond/ml/neural
 * (hand-rolled tanh from exp) and pond/math/matrix (synthesizes
 * `nan_sentinel()` as `0.0/0.0` and `is_nan(f)` as `f != f`).
 */
double lotus_math_nan(void) {
    return (double)NAN;
}

double lotus_math_inf(void) {
    return (double)INFINITY;
}

/* Canonical IEEE 754 NaN test: a quiet NaN is the only value
 * that is not equal to itself. Returns 1 if `f` is NaN, 0
 * otherwise. Lowers as i1 on the LLVM side via the truncation
 * pattern lotus_fs_file_exists uses for its 0/1 -> Bool. */
int lotus_math_is_nan(double f) {
    return f != f ? 1 : 0;
}
