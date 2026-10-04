/* GenMC model of the lotus FAILURE CASCADE: construction-time delivery
 * (the hold, the settle, the deferred reclaim, the cross-thread await)
 * and a run's retention against its child's reclaim: a queued run's
 * cancel against the pool worker's admission, and a started run's hold,
 * which the reclaim waits for.
 * F.40 phase 3, L5 (spec/runtime.md § Lifecycle obligations, decision
 * lines 1 and 19; notes/f40-lifecycle-inventory.md rows R19, R19a).
 *
 * ===================================================================
 * PROOF BOUNDARY
 * ===================================================================
 *
 * WHAT IT MIRRORS. A hand transcription of these functions in
 * crates/hale-codegen/runtime/lotus_arena.c, with their
 * synchronization kept exactly (the one `g_params_open_lock` mutex,
 * the `__ATOMIC_RELEASE` / `__ATOMIC_ACQUIRE` counters beside it, the
 * one `g_run_tickets_lock` mutex and its live count):
 *   - lotus_params_open / lotus_params_settle (the open table, the
 *     settle's in-order delivery loop with the node linked while its
 *     handler runs, DELIVERED + unlink + broadcast-or-free after it,
 *     then the child's resume or deferred reclaim);
 *   - lotus_failure_hold (the open-count fast path, the open-table
 *     search under the lock, the violation copied into the node);
 *   - lotus_failure_defer_reclaim, lotus_held_latest_for,
 *     lotus_held_unlink;
 *   - lotus_failure_await (opener's thread: never waits; another
 *     thread: registers as a waiter and waits for DELIVERED, the last
 *     waiter frees the node);
 *   - the compiled `__reclaim_<L>` fast path (a MONOTONIC load of
 *     `lotus_held_failure_count`, then defer_reclaim) and the reclaim
 *     bracket of emit_locus_arena_destroy (the `__arena` latch,
 *     queued cancellation, the started-run wait, then arena release);
 *   - the run tickets and holds: lotus_run_ticket_take (in the run
 *     post), lotus_run_admit (the worker's dispatch: the ticket becomes
 *     the run's hold, still linked), lotus_run_hold_release (the run
 *     returned), lotus_run_cell_drop_canceled (replay's ordering gate,
 *     which looks before it admits), lotus_run_cancel_queued
 *     (live-count fast path, the walk that cancels the queued tickets
 *     and counts the started ones other than the caller's own, then
 *     lotus_run_hold_wait); the worker's `t_run_running` around the run;
 *   - the cell's transport: the cooperative pool's Vyukov MPSC ring
 *     (`lotus_mpsc_ring_t`, the same ring the pinned mailbox uses),
 *     transcribed as coop_pool_model.c / mailbox_model.c have it. Its
 *     release/acquire handoff is what carries the instantiating
 *     thread's params-open to the worker.
 *
 * REDUCTIONS (not the production code):
 *   - The compiler's initial lotus_run_cancel_only and the physical
 *     callback's lotus_run_cancel_queued are collapsed to one cancel
 *     followed by its wait. There is no handler on the reclaiming
 *     thread here. The handler-boundary retirement queue, active-release
 *     guards, owner links and retained descendant trees are NOT modeled;
 *     deadline, trace-order and ASan regressions exercise those paths.
 *     The shared reclaim entry's per-instance atomic claim is covered
 *     separately by reclaim_claim_model.c, which races two entrants.
 *   - `pthread_self()` / `pthread_equal()` are a model thread id passed
 *     in: the opener test in lotus_failure_await compares ids, and the
 *     thread-local `t_run_running` (the caller's own run, which its
 *     reclaim does not wait for) is an array indexed by that id.
 *   - The open table and the ticket table are fixed-size (no realloc
 *     growth; the growth runs under the same lock and adds no surface),
 *     and the ticket table is one bucket (the hash only spreads it).
 *   - The violation is two ints, copied field by field (production
 *     memcpy's `err_size` bytes); a ring cell carries only the run's
 *     handler, its child and its ticket (no payload: a run post has a
 *     zero-size payload).
 *   - The pool's wake handshake is not re-modeled (coop_pool_model.c
 *     checks it); the worker spins on the ring.
 *   - THE CONDITION VARIABLE IS NOT MODELED (GenMC has none). The
 *     await's `pthread_cond_wait(&g_held_delivered, ...)` is reduced to
 *     its safety core: read a broadcast epoch under the lock, unlock,
 *     spin until a later broadcast, relock, re-check the predicate;
 *     the settle's `pthread_cond_broadcast` bumps the epoch under the
 *     lock. The re-check under the relocked mutex is what orders, as in
 *     pthread_cond_wait. The reclaim's wait for run holds
 *     (`pthread_cond_timedwait(&g_run_holds_cv, ...)`, a millisecond at
 *     a time) is reduced the same way, and the hold's release bumps its
 *     epoch. Between its timed waits production services the reclaiming
 *     thread's queue (the main bus queue, the pinned mailbox, or, on an
 *     async pool's coroutine, a timer park instead of the condvar); no
 *     cell travels back to the reclaiming thread here, so that is not
 *     modeled.
 *   - The trace build's naming (`lotus_lc_run_canceled`) is a counter.
 *
 * BOUNDED CONFIGURATION (exhaustive within it, nothing beyond):
 *   Phase 1, construction-time delivery: one owner, two children, one
 *     queued run, one pool worker; two threads (the instantiating
 *     thread, which opens the owner and settles it, then joins the
 *     pool and tears the owner's fields down; the pool worker). Child
 *     A is cooperative: its run() fails inline during the owner's
 *     params loop, on the opener's thread, and its spine asks to be
 *     reclaimed (deferred behind the handler). Child B is placed on the
 *     pool: its run() is posted (one ticket, one ring cell) and fails
 *     on the worker, held or delivered in place depending on where the
 *     owner's settle falls, then awaits the decision and reclaims
 *     itself, inside its own run: that reclaim's cancel finds the run's
 *     own hold and does not wait for it. The handler restarts nothing
 *     (no resume path).
 *   Phase 2, a reclaim no join orders, against the run's admission and
 *     its start: one child, one run, one pool worker; two threads. The
 *     instantiating thread posts the child's run, then begins its
 *     Reclaim (the latch, the cancel and its wait) and releases the
 *     arena as soon as lotus_run_cancel_queued returns, with no join
 *     before it, the order of a placed field reassigned on one thread
 *     while its run is queued or running on another pool's worker. The
 *     worker meanwhile dequeues the cell, passes replay's gate look,
 *     admits it, runs it and releases its hold. Every point at which the
 *     reclaim can fall is explored: before the post is admitted (the
 *     cancel wins, the cell is dropped), and after (the reclaim waits
 *     for the run's return). This is the order -DMODEL_RECLAIM_UNJOINED
 *     selected when the started run was this model's open boundary;
 *     with the hold it is the checked configuration, part of the gate,
 *     and the macro is gone. A teardown spine that joins before it
 *     reclaims is a sub-case of these interleavings.
 *
 * SAFETY ASSERTIONS GenMC checks across every interleaving (plus its
 * automatic data-race, use-after-free and double-free detection):
 *   (1) EXACTLY-ONCE TEARDOWN: every child's arena is released exactly
 *       once, whichever of the deferred reclaim at settle, the child's
 *       own spine and the owner's cascade reaches it first (the
 *       `__arena` latch); the ticket is freed exactly once.
 *   (2) RETENTION UNTIL THE HANDLER COMPLETES: the handler finds the
 *       child's arena live and the violation intact, on whichever
 *       thread it runs, so the child and the payload outlive it (the
 *       deferred reclaim, the cross-thread await, the copy freed only
 *       after the handler).
 *   (3) A RUN FINDS ITS CHILD WHOLE OR ITS TICKET CANCELED: each run
 *       ends exactly once, started or canceled, never both; a started
 *       run finds the arena live; a canceled one is dropped without
 *       touching the child, whose arena may already be gone.
 *   (4) NO DELIVERY BEFORE THE OWNER IS ACTIVE: no handler runs before
 *       the owner's params are all stored and settled (the owner's
 *       delivery machinery is up at settle). The handler reads the last
 *       param plainly, so an early or unordered delivery is an assertion
 *       failure or a reported data race. Each failure is delivered once.
 *   (5) A STARTED RUN HOLDS ITS CHILD UNTIL IT RETURNS: a run that was
 *       not canceled has returned before its child's arena is released,
 *       and finds the arena whole as it returns, whatever the reclaim
 *       did meanwhile; a run's own reclaim of its child (phase 1's B) is
 *       not held up by its own hold. Every ticket is unlinked by the
 *       end of each phase.
 *
 * WHAT IT DOES NOT ESTABLISH:
 *   - LIVENESS. Condition-variable liveness is excluded: GenMC has no
 *     condvars, and a spin that never ends is a blocked execution, not
 *     an error. Missed-wakeup and join-progress claims (no waiter
 *     sleeps through the broadcast; a parent waiting for a child never
 *     deadlocks with a child waiting for its parent; the pool join
 *     returns) rest on the deadline oracle and the lifecycle matrix
 *     (crates/hale-codegen/tests/lifecycle_matrix.rs, the l01_* /
 *     l19_* fixtures), not on this model. So does the reclaim's wait
 *     ending: that a started run returns (a run that never returns keeps
 *     its child, and its reclaim waits, by the retention's design), and
 *     that a run publishing back to the reclaiming thread is answered
 *     while that thread waits, which rests on the queue servicing and
 *     the l19_started_run_publishes_back fixtures.
 *   - The resume path (await returning 2, the restart a handler asks
 *     for), more than one held failure per child, nested opens, a
 *     handler that opens and settles a locus of its own, the async
 *     pool's coroutine drain, its abandonment (R20a) and the hold
 *     released there, the wait's timer park on a coroutine and its two
 *     aborting guards, the pools' teardown freeing undequeued cells
 *     (R21), a post refused at shutdown, the self-publish overflow list,
 *     and replay's hold buffer (thread-local, no cross-thread surface).
 *   - What the handler does to the owner beyond reading its params:
 *     a handler running on the child's thread after settle, beside the
 *     owner's birth(), is the execution-domain question (decision line
 *     1, Pending), not this protocol's.
 *   - Correspondence with the C. The model is a transcription; if the
 *     functions above change their locks, orders or steps, this model
 *     must change with them (verification/README.md, "Coverage gaps and
 *     drift").
 *
 * NEGATIVE CONTROLS (each removes one step the protocol needs; expected
 * to fail the named assertion):
 *   -DMODEL_BUG_NO_HOLD             the hold never holds: (4), A's
 *                                   handler runs before the last param
 *   -DMODEL_BUG_RECLAIM_NOW         defer_reclaim never defers: (2), A's
 *                                   arena is released before its handler
 *   -DMODEL_BUG_DELIVERED_BEFORE_HANDLER  the settle retires the node
 *                                   (DELIVERED, unlinked, broadcast)
 *                                   before its handler runs, not after:
 *                                   (2), B's await returns and B reclaims
 *                                   itself under its running handler
 *   -DMODEL_BUG_ADMIT_IGNORES_CANCEL the worker starts a canceled run:
 *                                   (3), started and canceled both
 *   -DMODEL_BUG_RECLAIM_SKIPS_WAIT  the cancel does not wait for the
 *                                   started runs' holds (admission frees
 *                                   nothing, but nothing waits): (5),
 *                                   phase 2 releases the arena before the
 *                                   admitted run returns (GenMC may name
 *                                   it first as the race or the
 *                                   use-after-free on the child's arena)
 *
 * Memory model: GenMC's default (release-acquire), the faithful one for
 * the runtime's orders. No GENMC-FLAGS pin.
 *
 * Run:  genmc -- verification/cascade_model.c   (or run_genmc.sh)
 *       genmc -- -DMODEL_BUG_NO_HOLD verification/cascade_model.c
 */

#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>
#include <assert.h>

/* --- the loci, reduced ----------------------------------------------- */

typedef struct { int alive; } arena_t;

typedef struct owner {
    int param;                    /* the owner's last param */
    int born;                     /* birth() ran */
} owner_t;

typedef struct child {
    arena_t      *arena;          /* the `__arena` slot: NULL once reclaimed */
    owner_t      *owner;
    int           id;
} child_t;

/* The ClosureViolation, reduced. */
typedef struct { int code; int detail; } violation_t;

#define PARAM_SET  42
#define FAIL_CODE  7
#define NCHILD     3

enum { TID_INSTANTIATING = 0, TID_WORKER = 1 };

static int g_teardowns[NCHILD];   /* arena releases per child          (1) */
static int g_delivered[NCHILD];   /* handler completions per child     (4) */
static int g_run_started[NCHILD]; /* runs admitted and started         (3) */
static int g_run_canceled[NCHILD];/* runs named NotStarted(Acknowledged) */
static int g_run_dropped;         /* canceled cells the worker dropped   */
static _Atomic int g_run_returned;/* phase 2's started run has returned (5) */

static child_t *child_create(owner_t *owner, int id) {
    child_t *c = malloc(sizeof *c);
    arena_t *a = malloc(sizeof *a);
    a->alive = 1;
    c->arena = a;
    c->owner = owner;
    c->id = id;
    return c;
}

/* ==================================================================== *
 * The run tickets (lotus_run_ticket_*; decision line 19)
 * ==================================================================== */

typedef struct lotus_run_ticket {
    void                    *child;
    int                      canceled;   /* written and read under the lock */
    int                      held;       /* admitted: the run's hold; under the lock */
    struct lotus_run_ticket *prev;
    struct lotus_run_ticket *next;
} lotus_run_ticket_t;

static lotus_run_ticket_t *g_run_tickets;          /* one bucket */
static _Atomic size_t      g_run_tickets_live;
static pthread_mutex_t     g_run_tickets_lock;
/* pthread_cond_t g_run_holds_cv, reduced as g_held_delivered is. */
static _Atomic int         g_run_holds_epoch;
/* t_run_running: the hold of the run executing on each model thread. */
static lotus_run_ticket_t *g_run_running[2];

/* Under the lock. */
static void lotus_run_ticket_unlink(lotus_run_ticket_t *t) {
    if (t->prev) t->prev->next = t->next;
    else g_run_tickets = t->next;
    if (t->next) t->next->prev = t->prev;
    t->prev = t->next = NULL;
    atomic_fetch_sub_explicit(&g_run_tickets_live, 1, memory_order_release);
}

static lotus_run_ticket_t *lotus_run_ticket_take(void *child) {
    lotus_run_ticket_t *t = malloc(sizeof *t);
    t->child    = child;
    t->canceled = 0;
    t->held     = 0;
    t->prev     = NULL;
    pthread_mutex_lock(&g_run_tickets_lock);
    t->next = g_run_tickets;
    if (t->next) t->next->prev = t;
    g_run_tickets = t;
    atomic_fetch_add_explicit(&g_run_tickets_live, 1, memory_order_release);
    pthread_mutex_unlock(&g_run_tickets_lock);
    return t;
}

/* The worker is about to start a dequeued run cell: 1 = admitted (the
 * child live, the ticket now the run's hold, still linked), 0 = its
 * reclaim canceled it first, and the ticket is freed. */
static int lotus_run_admit(lotus_run_ticket_t *t) {
    pthread_mutex_lock(&g_run_tickets_lock);
    int canceled = t->canceled;
    if (!canceled) t->held = 1;
    pthread_mutex_unlock(&g_run_tickets_lock);
#ifdef MODEL_BUG_ADMIT_IGNORES_CANCEL
    return 1;     /* started anyway; the cancel already unlinked it */
#endif
    if (canceled) free(t);
    return !canceled;
}

/* The run returned: its hold ends, and a waiting reclaim proceeds. */
static void lotus_run_hold_release(lotus_run_ticket_t *t) {
    if (!t) return;
    pthread_mutex_lock(&g_run_tickets_lock);
    lotus_run_ticket_unlink(t);
    atomic_fetch_add_explicit(&g_run_holds_epoch, 1, memory_order_relaxed);
    pthread_mutex_unlock(&g_run_tickets_lock);
    free(t);
}

/* The child's holds other than `own`. Under the lock. */
static int lotus_run_holds_outstanding(void *child, lotus_run_ticket_t *own) {
    int held = 0;
    for (lotus_run_ticket_t *t = g_run_tickets; t; t = t->next)
        if (t->child == child && t->held && t != own) held++;
    return held;
}

/* Wait until every started run of `child` but the caller's own has
 * returned. The condvar is reduced as the await's is: read the epoch
 * under the lock, unlock, spin until a later release, relock, re-check.
 * The production wait services its thread's queue between its timed
 * waits; that adds no shared state here (no cell travels back). */
static void lotus_run_hold_wait(void *child, lotus_run_ticket_t *own) {
#ifdef MODEL_BUG_RECLAIM_SKIPS_WAIT
    (void)child; (void)own;
    return;
#endif
    pthread_mutex_lock(&g_run_tickets_lock);
    while (lotus_run_holds_outstanding(child, own)) {
        int e = atomic_load_explicit(&g_run_holds_epoch, memory_order_relaxed);
        pthread_mutex_unlock(&g_run_tickets_lock);
        while (atomic_load_explicit(&g_run_holds_epoch,
                                    memory_order_relaxed) == e) { /* asleep */ }
        pthread_mutex_lock(&g_run_tickets_lock);
    }
    pthread_mutex_unlock(&g_run_tickets_lock);
}

/* The first step of the Reclaim bracket: cancel the queued runs, then
 * wait for the started ones. Returns the number it canceled (production
 * returns void and names each in the trace build). `self_tid` stands in
 * for the thread-local lookup of the caller's own run. */
static int lotus_run_cancel_queued(void *child, int self_tid) {
    if (!child) return 0;
    if (atomic_load_explicit(&g_run_tickets_live, memory_order_acquire) == 0)
        return 0;
    lotus_run_ticket_t *own = g_run_running[self_tid];
    int canceled = 0;
    int held = 0;
    pthread_mutex_lock(&g_run_tickets_lock);
    lotus_run_ticket_t *t = g_run_tickets;
    while (t) {
        lotus_run_ticket_t *next = t->next;
        if (t->child == child) {
            if (t->held) {
                if (t != own) held++;
            } else {
                t->canceled = 1;
                lotus_run_ticket_unlink(t);
                canceled++;
            }
        }
        t = next;
    }
    pthread_mutex_unlock(&g_run_tickets_lock);
    /* lotus_lc_run_canceled, once per canceled run */
    g_run_canceled[((child_t *)child)->id] += canceled;
    if (held) lotus_run_hold_wait(child, own);
    return canceled;
}

/* ==================================================================== *
 * The cell's transport: the Vyukov bounded MPSC ring
 * (lotus_mpsc_ring_*, as coop_pool_model.c transcribes it)
 * ==================================================================== */

typedef void (*run_fn)(child_t *child);

typedef struct {
    run_fn  handler;
    void   *self_ptr;
    void   *run_ticket;
} run_cell_t;

#define CAP 2               /* power of two, > number of posts */
#define MASK (CAP - 1)

typedef struct {
    _Atomic uint64_t seq;
    run_cell_t       cell;
} slot_t;

typedef struct {
    slot_t           slots[CAP];
    _Atomic uint64_t enqueue_pos;
    _Atomic uint64_t dequeue_pos;
} ring_t;

static void ring_init(ring_t *r) {
    for (uint64_t i = 0; i < CAP; i++)
        atomic_store_explicit(&r->slots[i].seq, i, memory_order_relaxed);
    atomic_store_explicit(&r->enqueue_pos, 0, memory_order_relaxed);
    atomic_store_explicit(&r->dequeue_pos, 0, memory_order_relaxed);
}

static int ring_try_enqueue(ring_t *r, run_fn h, void *self, void *ticket) {
    slot_t *slot;
    uint64_t pos = atomic_load_explicit(&r->enqueue_pos, memory_order_relaxed);
    for (;;) {
        slot = &r->slots[pos & MASK];
        uint64_t seq = atomic_load_explicit(&slot->seq, memory_order_acquire);
        int64_t dif = (int64_t)(seq - pos);
        if (dif == 0) {
            if (atomic_compare_exchange_weak_explicit(
                    &r->enqueue_pos, &pos, pos + 1,
                    memory_order_relaxed, memory_order_relaxed))
                break;
        } else if (dif < 0) {
            return 0;                       /* full */
        } else {
            pos = atomic_load_explicit(&r->enqueue_pos, memory_order_relaxed);
        }
    }
    slot->cell.handler    = h;              /* plain stores — published below */
    slot->cell.self_ptr   = self;
    slot->cell.run_ticket = ticket;
    atomic_store_explicit(&slot->seq, pos + 1, memory_order_release);
    return 1;
}

static int ring_try_dequeue(ring_t *r, run_cell_t *out) {
    uint64_t pos = atomic_load_explicit(&r->dequeue_pos, memory_order_relaxed);
    slot_t *slot = &r->slots[pos & MASK];
    uint64_t seq = atomic_load_explicit(&slot->seq, memory_order_acquire);
    int64_t dif = (int64_t)(seq - (pos + 1));
    if (dif == 0) {
        out->handler    = slot->cell.handler;   /* plain loads — pair w/ release */
        out->self_ptr   = slot->cell.self_ptr;
        out->run_ticket = slot->cell.run_ticket;
        atomic_store_explicit(&slot->seq, pos + MASK + 1, memory_order_release);
        atomic_store_explicit(&r->dequeue_pos, pos + 1, memory_order_relaxed);
        return 1;
    }
    return 0;                               /* empty */
}

static ring_t R;

/* lotus_coop_pool_post_cell with `run` = 1: the ticket is taken before
 * the cell is enqueued. The bound keeps the ring from filling. */
static void lotus_coop_pool_post_run(run_fn h, child_t *child) {
    lotus_run_ticket_t *ticket = lotus_run_ticket_take(child);
    int ok = ring_try_enqueue(&R, h, child, ticket);
    assert(ok);
}

/* Replay's gate looks at a run cell before the drain starts it: a
 * canceled one is ended here, ticket freed, never compared. */
static int lotus_run_cell_drop_canceled(run_cell_t *cell) {
    lotus_run_ticket_t *t = cell->run_ticket;
    if (!t) return 0;
    pthread_mutex_lock(&g_run_tickets_lock);
    int canceled = t->canceled;
    pthread_mutex_unlock(&g_run_tickets_lock);
#ifdef MODEL_BUG_ADMIT_IGNORES_CANCEL
    return 0;
#endif
    if (!canceled) return 0;
    free(t);
    cell->run_ticket = NULL;
    return 1;
}

/* The worker: dequeue one run cell, the replay gate's look, then
 * lotus_coop_pool_dispatch_cell's admission, the run with its hold
 * marked the worker's own, and the hold's release once it returns. */
static void *pool_worker(void *_) {
    (void)_;
    run_cell_t cell;
    while (!ring_try_dequeue(&R, &cell)) { /* spin until the cell arrives */ }
    /* A dropped cell never touches its child. */
    if (lotus_run_cell_drop_canceled(&cell)) {
        g_run_dropped++;
        return NULL;
    }
    lotus_run_ticket_t *hold = cell.run_ticket;
    if (hold && !lotus_run_admit(hold)) {
        g_run_dropped++;
        return NULL;
    }
    g_run_running[TID_WORKER] = hold;
    cell.handler((child_t *)cell.self_ptr);
    g_run_running[TID_WORKER] = NULL;
    lotus_run_hold_release(hold);
    return NULL;
}

/* ==================================================================== *
 * The failure hold (lotus_params_open / _settle, lotus_failure_*)
 * ==================================================================== */

typedef void (*lotus_failure_fn)(void *parent, void *child, void *err);
typedef void (*lotus_reclaim_fn)(void *child);

enum { LOTUS_HELD = 0, LOTUS_DELIVERING = 1, LOTUS_DELIVERED = 2 };

typedef struct lotus_held_failure {
    struct lotus_held_failure *next;
    void *parent;
    int opener;                  /* pthread_t in production */
    lotus_failure_fn fn;
    void *child;
    void *err;
    lotus_reclaim_fn reclaim;
    int state;
    int waiters;
} lotus_held_failure_t;

typedef struct { void *parent; int opener; } lotus_params_open_t;

#define OPEN_CAP 2

static pthread_mutex_t     g_params_open_lock;
static _Atomic int64_t     g_params_open_count;
static lotus_params_open_t g_params_open[OPEN_CAP];
static size_t              g_params_open_len;
static lotus_held_failure_t *g_held_head, *g_held_tail;
static _Atomic int64_t     lotus_held_failure_count;

/* pthread_cond_t g_held_delivered, reduced (see the header). */
static _Atomic int g_held_delivered_epoch;

static void held_delivered_broadcast(void) {        /* lock held */
    atomic_fetch_add_explicit(&g_held_delivered_epoch, 1, memory_order_relaxed);
}

static void held_delivered_wait(void) {             /* lock held */
    int e = atomic_load_explicit(&g_held_delivered_epoch, memory_order_relaxed);
    pthread_mutex_unlock(&g_params_open_lock);
    while (atomic_load_explicit(&g_held_delivered_epoch,
                                memory_order_relaxed) == e) { /* asleep */ }
    pthread_mutex_lock(&g_params_open_lock);
}

static void lotus_params_open(void *parent, int self_tid) {
    pthread_mutex_lock(&g_params_open_lock);
    assert(g_params_open_len < OPEN_CAP);
    g_params_open[g_params_open_len].parent = parent;
    g_params_open[g_params_open_len].opener = self_tid;
    g_params_open_len++;
    atomic_fetch_add_explicit(&g_params_open_count, 1, memory_order_release);
    pthread_mutex_unlock(&g_params_open_lock);
}

/* 1 = held (the caller skips the handler call), 0 = deliver now. */
static int64_t lotus_failure_hold(void *parent, lotus_failure_fn fn,
                                  void *child, const violation_t *err) {
    if (atomic_load_explicit(&g_params_open_count, memory_order_acquire) == 0)
        return 0;
#ifdef MODEL_BUG_NO_HOLD
    return 0;     /* the trace build's ConstructionDelivery skip */
#endif
    pthread_mutex_lock(&g_params_open_lock);
    lotus_params_open_t *open = NULL;
    for (size_t i = g_params_open_len; i-- > 0;) {
        if (g_params_open[i].parent == parent) {
            open = &g_params_open[i];
            break;
        }
    }
    if (!open) {
        pthread_mutex_unlock(&g_params_open_lock);
        return 0;
    }
    lotus_held_failure_t *node = malloc(sizeof *node);
    violation_t *copy = malloc(sizeof *copy);
    copy->code = err->code;
    copy->detail = err->detail;
    node->next = NULL;
    node->parent = parent;
    node->opener = open->opener;
    node->fn = fn;
    node->child = child;
    node->err = copy;
    node->reclaim = NULL;
    node->state = LOTUS_HELD;
    node->waiters = 0;
    if (g_held_tail) g_held_tail->next = node; else g_held_head = node;
    g_held_tail = node;
    atomic_fetch_add_explicit(&lotus_held_failure_count, 1, memory_order_release);
    pthread_mutex_unlock(&g_params_open_lock);
    return 1;
}

/* The latest outstanding failure of `child`, under the lock. */
static lotus_held_failure_t *lotus_held_latest_for(void *child) {
    lotus_held_failure_t *found = NULL;
    for (lotus_held_failure_t *n = g_held_head; n; n = n->next)
        if (n->child == child && n->state != LOTUS_DELIVERED) found = n;
    return found;
}

static void lotus_held_unlink(lotus_held_failure_t *node) {
    lotus_held_failure_t **pp = &g_held_head, *prev = NULL;
    while (*pp && *pp != node) { prev = *pp; pp = &(*pp)->next; }
    if (!*pp) return;
    *pp = node->next;
    if (g_held_tail == node) g_held_tail = prev;
    node->next = NULL;
}

/* 1 = a failure of this child is outstanding and the reclaim now runs
 * right after its handler; 0 = reclaim now. */
static int64_t lotus_failure_defer_reclaim(void *child, lotus_reclaim_fn reclaim) {
#ifdef MODEL_BUG_RECLAIM_NOW
    (void)child; (void)reclaim;
    return 0;
#endif
    pthread_mutex_lock(&g_params_open_lock);
    lotus_held_failure_t *node = lotus_held_latest_for(child);
    if (node) node->reclaim = reclaim;
    pthread_mutex_unlock(&g_params_open_lock);
    return node ? 1 : 0;
}

/* 0 = nothing outstanding; 1 = waited for the handler on another
 * thread's parent; 2 = the resume runs at settle (not in this bound:
 * every caller passes no resume). */
static int64_t lotus_failure_await(void *child, int has_resume, int self_tid) {
    if (atomic_load_explicit(&lotus_held_failure_count, memory_order_acquire) == 0)
        return 0;
    pthread_mutex_lock(&g_params_open_lock);
    lotus_held_failure_t *node = lotus_held_latest_for(child);
    if (!node) {
        pthread_mutex_unlock(&g_params_open_lock);
        return 0;
    }
    if (node->opener == self_tid) {
        int64_t r = 0;
        if (node->state == LOTUS_HELD && has_resume) r = 2;
        pthread_mutex_unlock(&g_params_open_lock);
        return r;
    }
    node->waiters++;
    while (node->state != LOTUS_DELIVERED)
        held_delivered_wait();
    if (--node->waiters == 0) free(node);
    pthread_mutex_unlock(&g_params_open_lock);
    return 1;
}

static void lotus_params_settle(void *parent) {
    pthread_mutex_lock(&g_params_open_lock);
    for (size_t i = g_params_open_len; i-- > 0;) {
        if (g_params_open[i].parent == parent) {
            --g_params_open_len;             /* field by field: no memcpy */
            g_params_open[i].parent = g_params_open[g_params_open_len].parent;
            g_params_open[i].opener = g_params_open[g_params_open_len].opener;
            atomic_fetch_sub_explicit(&g_params_open_count, 1, memory_order_release);
            break;
        }
    }
    pthread_mutex_unlock(&g_params_open_lock);
    for (;;) {
        pthread_mutex_lock(&g_params_open_lock);
        lotus_held_failure_t *node = g_held_head;
        while (node && !(node->parent == parent && node->state == LOTUS_HELD))
            node = node->next;
        if (!node) {
            pthread_mutex_unlock(&g_params_open_lock);
            break;
        }
        node->state = LOTUS_DELIVERING;
#ifdef MODEL_BUG_DELIVERED_BEFORE_HANDLER
        {
            /* The node is retired before its handler runs: a waiter
             * wakes, and a reclaim no longer finds it to defer behind. */
            lotus_failure_fn fn = node->fn;
            void *n_parent = node->parent, *n_child = node->child;
            void *n_err = node->err;
            lotus_reclaim_fn n_reclaim = node->reclaim;
            node->state = LOTUS_DELIVERED;
            lotus_held_unlink(node);
            atomic_fetch_sub_explicit(&lotus_held_failure_count, 1,
                                      memory_order_release);
            if (node->waiters > 0) held_delivered_broadcast();
            else free(node);
            pthread_mutex_unlock(&g_params_open_lock);
            fn(n_parent, n_child, n_err);
            free(n_err);
            if (n_reclaim) n_reclaim(n_child);
            continue;
        }
#endif
        pthread_mutex_unlock(&g_params_open_lock);

        node->fn(node->parent, node->child, node->err);

        pthread_mutex_lock(&g_params_open_lock);
        void *child = node->child;
        void *err = node->err;
        lotus_reclaim_fn reclaim = node->reclaim;
        node->state = LOTUS_DELIVERED;
        lotus_held_unlink(node);
        atomic_fetch_sub_explicit(&lotus_held_failure_count, 1, memory_order_release);
        int waited = node->waiters > 0;
        if (waited)
            held_delivered_broadcast();
        else
            free(node);
        pthread_mutex_unlock(&g_params_open_lock);
        free(err);
        if (reclaim)                         /* no resume in this bound */
            reclaim(child);
    }
}

/* ==================================================================== *
 * The compiled side: the reclaim bracket and the reclaim spine
 * ==================================================================== */

/* emit_locus_arena_destroy: the `__arena` latch, the cancel (and the
 * wait for started runs) first, then the arena released. */
static void child_teardown(child_t *c, int self_tid) {
    arena_t *a = c->arena;
    if (!a) return;                          /* already reclaimed */
    lotus_run_cancel_queued(c, self_tid);
    a->alive = 0;
    free(a);
    c->arena = NULL;                         /* the latch */
    g_teardowns[c->id]++;
}

/* A deferred reclaim, run by the settle on the instantiating thread. */
static void child_teardown_at_settle(void *cv) {
    child_teardown(cv, TID_INSTANTIATING);
}

/* `__reclaim_<L>`: one monotonic load, then ask whether a held handler
 * still needs the child. */
static void child_reclaim_spine(child_t *c, int self_tid) {
    if (atomic_load_explicit(&lotus_held_failure_count, memory_order_relaxed) != 0) {
        if (lotus_failure_defer_reclaim(c, child_teardown_at_settle)) return;
    }
    child_teardown(c, self_tid);
}

/* The owner's on_failure. */
static void owner_on_failure(void *parent, void *child, void *err) {
    owner_t *o = parent;
    child_t *c = child;
    violation_t *v = err;
    /* (4) never before the owner is active: its last param is stored */
    assert(o->param == PARAM_SET);
    /* (2) the child and the violation outlive the handler */
    arena_t *a = c->arena;
    assert(a != NULL);
    assert(a->alive == 1);
    assert(v->code == FAIL_CODE && v->detail == c->id);
    g_delivered[c->id]++;
}

/* A child's run() that fails: raise to the owner (held or in place),
 * learn the decision, then the run end's reclaim. */
static void child_run_fails(child_t *c, int self_tid) {
    assert(c->arena != NULL && c->arena->alive == 1);
    violation_t err;
    err.code = FAIL_CODE;
    err.detail = c->id;
    if (!lotus_failure_hold(c->owner, owner_on_failure, c, &err))
        owner_on_failure(c->owner, c, &err);
    lotus_failure_await(c, 0, self_tid);
    child_reclaim_spine(c, self_tid);
}

/* Its run end reclaims its own child on the worker, inside the run: the
 * hold it waits past is its own (`g_run_running`), never waited for. */
static void child_b_run(child_t *c) {        /* runs on the pool worker */
    g_run_started[c->id]++;
    child_run_fails(c, TID_WORKER);
}

static void child_c_run(child_t *c) {        /* a run that starts and returns */
    arena_t *a = c->arena;
    assert(a != NULL);                       /* (3) whole, never released */
    assert(a->alive == 1);
    g_run_started[c->id]++;
    /* (5) still whole as it returns, whatever the reclaim did meanwhile */
    assert(c->arena == a && a->alive == 1);
    atomic_store_explicit(&g_run_returned, 1, memory_order_relaxed);
}

static void reset(void) {
    atomic_store_explicit(&g_params_open_count, 0, memory_order_relaxed);
    atomic_store_explicit(&lotus_held_failure_count, 0, memory_order_relaxed);
    atomic_store_explicit(&g_held_delivered_epoch, 0, memory_order_relaxed);
    atomic_store_explicit(&g_run_tickets_live, 0, memory_order_relaxed);
    atomic_store_explicit(&g_run_holds_epoch, 0, memory_order_relaxed);
    atomic_store_explicit(&g_run_returned, 0, memory_order_relaxed);
    g_params_open_len = 0;
    g_held_head = g_held_tail = NULL;
    g_run_tickets = NULL;
    g_run_running[TID_INSTANTIATING] = g_run_running[TID_WORKER] = NULL;
    for (int i = 0; i < NCHILD; i++) {
        g_teardowns[i] = g_delivered[i] = 0;
        g_run_started[i] = g_run_canceled[i] = 0;
    }
    g_run_dropped = 0;
    ring_init(&R);
}

/* ==================================================================== *
 * Phase 1 — construction-time delivery: one owner, child A inline on
 * the opener's thread, child B's run posted to the pool.
 * ==================================================================== */
static void phase1_construction_delivery(void) {
    reset();
    pthread_t w;
    pthread_create(&w, NULL, pool_worker, NULL);

    owner_t *owner = malloc(sizeof *owner);
    owner->param = 0;
    owner->born = 0;
    lotus_params_open(owner, TID_INSTANTIATING);

    /* param 1: A, cooperative; its run() fails inline, on this thread,
     * while the params are open, then its spine reclaims it. */
    child_t *a = child_create(owner, 0);
    child_run_fails(a, TID_INSTANTIATING);

    /* param 2: B, placed on the pool; its run() is posted. */
    child_t *b = child_create(owner, 1);
    lotus_coop_pool_post_run(child_b_run, b);

    /* param 3, the last, then the settle and birth(). */
    owner->param = PARAM_SET;
    lotus_params_settle(owner);
    owner->born = 1;

    /* Teardown: the pool join, then the owner's cascade over its
     * fields, each through the latch. */
    pthread_join(w, NULL);
    child_teardown(a, TID_INSTANTIATING);
    child_teardown(b, TID_INSTANTIATING);

    assert(g_teardowns[0] == 1 && g_teardowns[1] == 1);   /* (1) */
    assert(g_delivered[0] == 1 && g_delivered[1] == 1);   /* (4) once each */
    assert(g_run_started[1] == 1 && g_run_canceled[1] == 0);
    assert(atomic_load_explicit(&lotus_held_failure_count,
                                memory_order_relaxed) == 0);
    assert(g_held_head == NULL);
    assert(atomic_load_explicit(&g_run_tickets_live, memory_order_relaxed) == 0);
    assert(g_run_tickets == NULL);
    free(a);
    free(b);
    free(owner);
}

/* ==================================================================== *
 * Phase 2 — a reclaim no join orders, against the run's admission and
 * its start: a placed field reassigned while its run is queued, being
 * admitted, or already running on the worker.
 * ==================================================================== */
static void phase2_unjoined_reclaim(void) {
    reset();
    pthread_t w;
    pthread_create(&w, NULL, pool_worker, NULL);

    owner_t *owner = malloc(sizeof *owner);
    owner->param = PARAM_SET;
    owner->born = 1;
    child_t *c = child_create(owner, 2);
    lotus_coop_pool_post_run(child_c_run, c);

    /* The Reclaim, with no join before it: the latch, the cancel (which
     * waits for a started run's hold), then the arena released at once,
     * with the cell possibly still in the ring. */
    arena_t *arena = c->arena;
    int canceled = lotus_run_cancel_queued(c, TID_INSTANTIATING);
    /* (5) a run that was not canceled has returned before the release */
    assert(canceled ||
           atomic_load_explicit(&g_run_returned, memory_order_relaxed) == 1);
    arena->alive = 0;
    free(arena);
    c->arena = NULL;
    g_teardowns[2]++;
    pthread_join(w, NULL);

    /* (3) exactly one terminal: started, or canceled and dropped. */
    assert(g_run_started[2] + g_run_canceled[2] == 1);
    assert(g_run_canceled[2] == g_run_dropped);
    assert(canceled == g_run_canceled[2]);
    assert(g_teardowns[2] == 1);                          /* (1) */
    assert(atomic_load_explicit(&g_run_tickets_live, memory_order_relaxed) == 0);
    assert(g_run_tickets == NULL);
    free(c);
    free(owner);
}

int main(void) {
    pthread_mutex_init(&g_params_open_lock, NULL);
    pthread_mutex_init(&g_run_tickets_lock, NULL);
    phase1_construction_delivery();
    phase2_unjoined_reclaim();
    return 0;
}
