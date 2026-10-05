/* GenMC model of the lotus FAILURE CASCADE: construction-time delivery
 * (the hold, the settle, the deferred reclaim, the cross-thread await),
 * a run's retention against its child's reclaim (a queued run's cancel
 * against the pool worker's admission, and a started run's hold, which
 * the reclaim waits for), and the delivery's domain: a failure raised
 * off its owner's domain is posted there and awaited, and the owner's
 * reclaim waits for it, or, where only the reclaiming thread could run
 * it (a handler replacing a failing sibling), is deferred behind it.
 * F.40 phase 3, L5 (spec/runtime.md § Failure handling, decision L0-1;
 * § Lifecycle obligations, decision lines 1 and 19, join progress;
 * notes/f40-lifecycle-inventory.md rows R19, R19a, C36).
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
 *     thread's params-open to the worker;
 *   - decision L0-1's posted delivery: the owner's domain recorded at
 *     lotus_params_open (lotus_failure_owner_note_locked);
 *     lotus_failure_post (the domain lookup, the in-place answer on the
 *     owner's domain, the cell's hold taken under the lock, the node
 *     linked HELD with `posted`, the domain's pending and refs, the
 *     broadcast, then the poster's wait and its release of the node);
 *     lotus_failure_claim_locked, lotus_failure_deliver_posted,
 *     lotus_failure_service_here, lotus_failure_await_service_locked,
 *     lotus_failure_wait_locked (servicing this thread's own posts, and
 *     running a delivery whose domain has ended in place),
 *     lotus_failure_reclaim_wait_locked (deferring behind a delivery
 *     only the reclaiming thread could run: its own handler's, or one
 *     still held for its domain while it is inside another handler) and
 *     defer_reclaim's posted branch; the settle skipping posted nodes; lotus_failure_hold_cell
 *     (the delivery's hold on the run-ticket table) and the dispatch's
 *     release of whichever hold its cell ended with; lotus_run_hold_wait
 *     servicing posted deliveries between its waits; the joins' wait
 *     for the joined domain's end (lotus_domain_join_wait, which
 *     lotus_pinned_join and the pool join share) and a worker's domain
 *     ending as it leaves its loop.
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
 *     reclaim does not wait for) is an array indexed by that id. A
 *     handler receives the id of the thread running it, which is what
 *     assertion (6) reads. A domain is a model thread id
 *     (`lotus_domain_t` in production, one per thread), and
 *     `t_failure_servicing` an array indexed by it.
 *   - The posted delivery's wake (a wake cell on a parked worker's ring
 *     or a pinned mailbox) is not modeled: every wait here is the epoch
 *     spin below, which the post's broadcast ends. A yield on the
 *     owner's thread is an explicit lotus_failure_service_here call.
 *     The pool-start branch of lotus_failure_wait_locked (C50), the
 *     async pool's coroutine park, the owner record's removal at an
 *     owner's reclaim (lotus_failure_owner_forget) and the pinned
 *     join's freeing of its domain are not modeled. The run-hold wait's
 *     reduced sleep also ends when any delivery is posted, where
 *     production's millisecond slices service the thread's queue.
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
 *     async pool's coroutine, a timer park instead of the condvar); of
 *     that servicing the model keeps the deliveries posted to the
 *     reclaiming thread (phase 3), and no other cell travels back to it.
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
 *     on the worker, held while the owner's params are open, or, once
 *     they have settled, posted to the owner's domain (the instantiating
 *     thread), which runs it inside the pool join (join progress); B
 *     then awaits the decision and reclaims itself, inside its own run:
 *     that reclaim's cancel finds the run's own hold and does not wait
 *     for it. The handler restarts nothing (no resume path).
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
 *   Phase 3, the delivery's domain, run twice: one owner on the
 *     instantiating thread (its domain, params settled) and one child
 *     whose handler cell (not a run: it carries no hold of its own)
 *     fails on a pool worker; two threads. The worker posts the
 *     delivery to the owner's domain and waits for the decision, then
 *     reads the child (what follows the decision in the cell), and its
 *     dispatch releases the hold the post took. The owner observes the
 *     delivery in flight (a probe the post sets, as the
 *     fd_reclaim_under_delivery.hl fixture's gate file does), reads its
 *     own state twice with no yield between (the window), then reclaims
 *     the child: in the first run the reclaim's own wait runs the
 *     delivery; in the second a yield runs it first, and the reclaim
 *     then waits for the cell's hold. The owner then joins the worker.
 *   Phase 4, a sibling replaced from a handler: one owner on the
 *     instantiating thread and two children whose handler cells fail on
 *     two pool workers; three threads. A posts its failure; a yield on
 *     the owner's thread runs A's handler, which releases B's cell (the
 *     message B fails on), waits for B's post (the probe, as the
 *     fd_sibling_replace.hl fixture's gate file), and reclaims B from
 *     inside the handler (`self.b = Kid { }`), B's delivery still held
 *     for the owner's thread. The reclaim is deferred behind it; the
 *     same service runs B's handler once A's has returned, then B's
 *     reclaim once B's poster has resumed, which waits for B's cell's
 *     hold. The owner then joins both workers and tears A down.
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
 *   (6) THE HANDLER RUNS ON ITS OWNER'S DOMAIN: every handler, held,
 *       posted or in place, runs on the thread the owner's params were
 *       opened on, and nothing the handler writes to the owner changes
 *       inside the owner's own window (a plain read pair, so a handler
 *       beside it is an assertion failure or a reported data race).
 *   (7) A POSTED DELIVERY'S CHILD OUTLIVES ITS CELL: the child's arena
 *       is live in the handler and in what the failing cell does after
 *       the decision, whatever the owner's reclaim does meanwhile; the
 *       reclaim releases it once, after the cell's hold ends. Each
 *       posted delivery runs once, its node is freed once, and no
 *       delivery is left posted at the end of the phase.
 *   (8) A RECLAIM NEVER WAITS FOR A DELIVERY ONLY ITS OWN THREAD CAN
 *       RUN, AND HANDLERS DO NOT NEST: no reclaim enters the wait for a
 *       posted delivery while its thread is inside a handler and the
 *       delivery is still held for that thread's domain (the wait could
 *       never end, which GenMC would report only as a blocked
 *       execution, so the model asserts at its entry); no handler starts
 *       while another runs on the same thread. The replaced child stays
 *       whole until its own handler has run, that handler runs after the
 *       replacing one returns, and neither failure is dropped.
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
 *   - The delivery's other domains and orders: a delivery to a pinned
 *     thread's or a pool worker's domain (the owner here is always the
 *     instantiating thread), more than two posted deliveries at once or
 *     a sibling replaced before its failure is posted, a handler that
 *     reclaims its own child (the reclaim deferred behind it,
 *     transcribed, not reached), a decision a handler makes about a
 *     child already replaced (the reclaim wins: the deferral marks the
 *     child's reclaim claim owed, and the restart decision, compiled
 *     code outside this model, reads it; the fd_restart*_replaced
 *     fixtures carry it), a delivery posted to a domain that
 *     has ended (transcribed, not reached: the owner's domain outlives
 *     every post here), and a reclaim that no observation orders after
 *     the post (a handler cell has no hold before it fails; that the
 *     owner does not reclaim a child under its running cell before then
 *     is not this protocol's claim).
 *   - That the joins and the waits END: join progress (the owner, in a
 *     join, runs what is posted to it until the joined domain ends) is
 *     transcribed, and an execution where it would not end is a blocked
 *     one, not an error. The lifecycle matrix's jp_late_failure_* and
 *     C36 cells and the failure_delivery_domain fixtures carry it.
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
 *   -DMODEL_BUG_DELIVER_IN_PLACE    lotus_failure_post never posts, so a
 *                                   failure off the owner's domain calls
 *                                   the handler in place, the code before
 *                                   L5's fourth part: (6), phase 1's B or
 *                                   phase 3's child runs the handler on
 *                                   the worker. A single native run fails
 *                                   it too.
 *   -DMODEL_BUG_NO_CELL_HOLD        the post takes no hold on its child:
 *                                   (7), phase 3's reclaim releases the
 *                                   arena once the poster has left the
 *                                   post, while its cell still reads the
 *                                   child (a race or use-after-free on
 *                                   the child's arena)
 *   -DMODEL_BUG_SIBLING_RECLAIM_WAITS  the reclaim defers only behind
 *                                   its own handler's delivery, the code
 *                                   before PR #1348's review: (8), phase
 *                                   4's reclaim of B, inside A's handler,
 *                                   waits for B's delivery held for its
 *                                   own thread. A single native run
 *                                   fails it too.
 *
 * Memory model: GenMC's default (release-acquire), the faithful one for
 * the runtime's orders. No GENMC-FLAGS pin.
 *
 * Run:  genmc -- verification/cascade_model.c   (or run_genmc.sh)
 *       genmc -- -DMODEL_BUG_NO_HOLD verification/cascade_model.c
 *
 * `lifecycle_flow cascade_model` compiles it and each control with
 * `clang -std=c11 -Wall -Wextra -Werror -pthread -fsyntax-only`, and
 * runs it natively once (one interleaving, not a proof): the model
 * passes, and -DMODEL_BUG_DELIVER_IN_PLACE and
 * -DMODEL_BUG_SIBLING_RECLAIM_WAITS abort.
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
    int domain;                   /* the thread its params opened on (6) */
    int heard;                    /* what its handler writes (6) */
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
#define NCHILD     6

enum { TID_INSTANTIATING = 0, TID_WORKER = 1, TID_WORKER2 = 2, NTID = 3 };

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
static lotus_run_ticket_t *g_run_running[NTID];

/* Defined with the posted delivery below. */
static void lotus_failure_service_here(int self_tid);
static _Atomic int64_t g_failure_posted_count;

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
 * waits; of that, the deliveries posted to this thread are kept (a hold
 * may be a posted failure's, whose cell waits for this thread to run
 * it), and the reduced sleep also ends when one is posted. */
static void lotus_run_hold_wait(void *child, lotus_run_ticket_t *own,
                                int self_tid) {
#ifdef MODEL_BUG_RECLAIM_SKIPS_WAIT
    (void)child; (void)own; (void)self_tid;
    return;
#endif
    pthread_mutex_lock(&g_run_tickets_lock);
    while (lotus_run_holds_outstanding(child, own)) {
        int e = atomic_load_explicit(&g_run_holds_epoch, memory_order_relaxed);
        pthread_mutex_unlock(&g_run_tickets_lock);
        lotus_failure_service_here(self_tid);
        while (atomic_load_explicit(&g_run_holds_epoch,
                                    memory_order_relaxed) == e &&
               atomic_load_explicit(&g_failure_posted_count,
                                    memory_order_acquire) == 0) { /* asleep */ }
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
    if (held) lotus_run_hold_wait(child, own, self_tid);
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

/* Defined with the posted delivery below. */
static void lotus_domain_end(int tid);

/* The worker: dequeue one run cell, the replay gate's look, then
 * lotus_coop_pool_dispatch_cell's admission, the run with its hold
 * marked the worker's own, and the release of whichever hold the cell
 * ended with (the run's, or one a posted failure took). */
static void pool_worker_cell(void) {
    run_cell_t cell;
    while (!ring_try_dequeue(&R, &cell)) { /* spin until the cell arrives */ }
    /* A dropped cell never touches its child. */
    if (lotus_run_cell_drop_canceled(&cell)) {
        g_run_dropped++;
        return;
    }
    lotus_run_ticket_t *hold = cell.run_ticket;
    if (hold && !lotus_run_admit(hold)) {
        g_run_dropped++;
        return;
    }
    g_run_running[TID_WORKER] = hold;
    cell.handler((child_t *)cell.self_ptr);
    hold = g_run_running[TID_WORKER];
    g_run_running[TID_WORKER] = NULL;
    lotus_run_hold_release(hold);
}

/* lotus_coop_pool_worker: its cells, then, leaving its loop, what was
 * posted to it, and its domain ends. */
static void *pool_worker(void *_) {
    (void)_;
    pool_worker_cell();
    lotus_failure_service_here(TID_WORKER);
    lotus_domain_end(TID_WORKER);
    return NULL;
}

/* ==================================================================== *
 * The failure hold (lotus_params_open / _settle, lotus_failure_*)
 * ==================================================================== */

/* `self_tid` stands in for the handler's own thread (assertion (6)). */
typedef void (*lotus_failure_fn)(void *parent, void *child, void *err,
                                 int self_tid);
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
    int posted;                  /* L0-1: the domain it waits for, -1 none */
    int deliverer;               /* the thread running its handler */
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

/* Decision L0-1's domains, under g_params_open_lock but where noted. */
static int g_domain_alive[NTID];      /* the thread still consumes */
static int g_domain_pending[NTID];    /* posted deliveries not yet claimed */
static int g_domain_refs[NTID];       /* posters still naming it */
static int g_servicing[NTID];         /* t_failure_servicing: its own thread's */
static _Atomic int g_posted_probe;    /* phases 3, 4: posts so far, observed */
static int g_handler_depth[NTID];     /* handlers running on the thread (8) */

#define OWNERS_CAP 2
static struct { void *owner; int tid; } g_owner_domains[OWNERS_CAP];
static size_t          g_owner_domains_len;
static _Atomic int64_t g_owner_domain_count;

/* lotus_failure_owner_note_locked: the thread opening the owner's
 * params is its domain. */
static void lotus_failure_owner_note_locked(void *owner, int self_tid) {
    for (size_t i = 0; i < g_owner_domains_len; i++) {
        if (g_owner_domains[i].owner == owner) {
            g_owner_domains[i].tid = self_tid;
            return;
        }
    }
    assert(g_owner_domains_len < OWNERS_CAP);
    g_owner_domains[g_owner_domains_len].owner = owner;
    g_owner_domains[g_owner_domains_len].tid = self_tid;
    g_owner_domains_len++;
    atomic_fetch_add_explicit(&g_owner_domain_count, 1, memory_order_release);
}

static void lotus_params_open(void *parent, int self_tid) {
    pthread_mutex_lock(&g_params_open_lock);
    assert(g_params_open_len < OPEN_CAP);
    g_params_open[g_params_open_len].parent = parent;
    g_params_open[g_params_open_len].opener = self_tid;
    g_params_open_len++;
    atomic_fetch_add_explicit(&g_params_open_count, 1, memory_order_release);
    lotus_failure_owner_note_locked(parent, self_tid);
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
    node->posted = -1;
    node->deliverer = -1;
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

/* Defined with the posted delivery below. */
static int lotus_failure_reclaim_wait_locked(lotus_held_failure_t *n,
                                             int self_tid);

/* 1 = a failure of this child is outstanding and the reclaim now runs
 * right after its handler; 0 = reclaim now. A posted delivery is waited
 * for instead (run here if it was posted to this thread), unless this
 * is its own handler. */
static int64_t lotus_failure_defer_reclaim(void *child, lotus_reclaim_fn reclaim,
                                           int self_tid) {
#ifdef MODEL_BUG_RECLAIM_NOW
    (void)child; (void)reclaim; (void)self_tid;
    return 0;
#endif
    pthread_mutex_lock(&g_params_open_lock);
    lotus_held_failure_t *node = lotus_held_latest_for(child);
    int deferred = node != NULL;
    if (node && node->posted >= 0)
        deferred = lotus_failure_reclaim_wait_locked(node, self_tid);
    if (deferred) node->reclaim = reclaim;
    pthread_mutex_unlock(&g_params_open_lock);
    return deferred;
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

static void lotus_params_settle(void *parent, int self_tid) {
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
        while (node && !(node->parent == parent && node->state == LOTUS_HELD &&
                         node->posted < 0))
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
            fn(n_parent, n_child, n_err, self_tid);
            free(n_err);
            if (n_reclaim) n_reclaim(n_child);
            continue;
        }
#endif
        pthread_mutex_unlock(&g_params_open_lock);

        /* Under the posted delivery's guard: handlers do not nest. */
        int was_servicing = g_servicing[self_tid];
        g_servicing[self_tid] = 1;
        node->fn(node->parent, node->child, node->err, self_tid);
        g_servicing[self_tid] = was_servicing;

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
 * Decision L0-1: the posted delivery (lotus_failure_post and its waits)
 * ==================================================================== */

/* The delivery's hold on its child, the run hold's twin: a failure
 * posted from a cell that carries no hold of its own holds the child
 * until the cell returns. Linked and held under one lock. Called with
 * g_params_open_lock held (that lock, then the tickets'). */
static void lotus_failure_hold_cell(void *child, int self_tid) {
#ifdef MODEL_BUG_NO_CELL_HOLD
    (void)child; (void)self_tid;
    return;
#endif
    if (g_run_running[self_tid]) return;
    lotus_run_ticket_t *t = malloc(sizeof *t);
    t->child    = child;
    t->canceled = 0;
    t->held     = 1;
    t->prev     = NULL;
    pthread_mutex_lock(&g_run_tickets_lock);
    t->next = g_run_tickets;
    if (t->next) t->next->prev = t;
    g_run_tickets = t;
    atomic_fetch_add_explicit(&g_run_tickets_live, 1, memory_order_release);
    pthread_mutex_unlock(&g_run_tickets_lock);
    g_run_running[self_tid] = t;
}

/* Lock held. */
static void lotus_failure_claim_locked(lotus_held_failure_t *n, int self_tid) {
    n->state = LOTUS_DELIVERING;
    n->deliverer = self_tid;
    g_domain_pending[n->posted]--;
    atomic_fetch_sub_explicit(&g_failure_posted_count, 1, memory_order_release);
}

/* Run a claimed delivery's handler here, unlocked; then it is
 * delivered and its poster resumes. A reclaim its handler asked for
 * runs after every other waiter has resumed. */
static void lotus_failure_deliver_posted(lotus_held_failure_t *n,
                                         int self_waiting, int self_tid) {
    n->fn(n->parent, n->child, n->err, self_tid);
    pthread_mutex_lock(&g_params_open_lock);
    void *err = n->err;
    void *child = n->child;
    lotus_reclaim_fn reclaim = n->reclaim;
    n->err = NULL;
    n->state = LOTUS_DELIVERED;
    lotus_held_unlink(n);
    atomic_fetch_sub_explicit(&lotus_held_failure_count, 1, memory_order_release);
    held_delivered_broadcast();
    if (reclaim) {
        if (!self_waiting) n->waiters++;
        while (n->waiters > 1) held_delivered_wait();
        if (!self_waiting) free(n);
    }
    pthread_mutex_unlock(&g_params_open_lock);
    free(err);
    if (reclaim) reclaim(child);
}

/* What is posted to this thread's domain, run here; never inside a
 * handler this thread is already running for one. */
static void lotus_failure_service_here(int self_tid) {
    if (atomic_load_explicit(&g_failure_posted_count, memory_order_acquire) == 0)
        return;
    if (g_servicing[self_tid]) return;
    g_servicing[self_tid] = 1;
    for (;;) {
        pthread_mutex_lock(&g_params_open_lock);
        lotus_held_failure_t *n = g_held_head;
        while (n && !(n->posted == self_tid && n->state == LOTUS_HELD)) n = n->next;
        if (!n) {
            pthread_mutex_unlock(&g_params_open_lock);
            break;
        }
        lotus_failure_claim_locked(n, self_tid);
        pthread_mutex_unlock(&g_params_open_lock);
        lotus_failure_deliver_posted(n, 0, self_tid);
    }
    g_servicing[self_tid] = 0;
}

/* Lock held. */
static void lotus_failure_await_service_locked(int self_tid) {
    if (g_domain_pending[self_tid] == 0 || g_servicing[self_tid]) return;
    pthread_mutex_unlock(&g_params_open_lock);
    lotus_failure_service_here(self_tid);
    pthread_mutex_lock(&g_params_open_lock);
}

/* Wait, the lock held, until `n` is delivered, servicing this thread's
 * own posts; a delivery whose domain has ended runs here. */
static void lotus_failure_wait_locked(lotus_held_failure_t *n, int self_tid) {
    while (n->state != LOTUS_DELIVERED) {
        if (n->state == LOTUS_HELD && !g_domain_alive[n->posted]) {
            lotus_failure_claim_locked(n, self_tid);
            pthread_mutex_unlock(&g_params_open_lock);
            lotus_failure_deliver_posted(n, 1, self_tid);
            pthread_mutex_lock(&g_params_open_lock);
            continue;
        }
        if (g_domain_pending[self_tid] > 0 && !g_servicing[self_tid]) {
            lotus_failure_await_service_locked(self_tid);
            continue;
        }
        held_delivered_wait();
    }
}

/* defer_reclaim's wait, the lock held: 0 once the posted delivery is
 * delivered and its poster has resumed; 1 when the reclaim is deferred
 * behind it instead, since only this thread could run it: its own
 * handler, or one still held for this thread while it is inside another
 * handler (handlers do not nest). */
static int lotus_failure_reclaim_wait_locked(lotus_held_failure_t *n,
                                             int self_tid) {
    if (n->state == LOTUS_DELIVERING && n->deliverer == self_tid) return 1;
#ifndef MODEL_BUG_SIBLING_RECLAIM_WAITS
    if (g_servicing[self_tid] && n->state == LOTUS_HELD && n->posted == self_tid)
        return 1;
#endif
    /* (8) never a wait for a delivery only this thread could run */
    assert(!(g_servicing[self_tid] && n->state == LOTUS_HELD &&
             n->posted == self_tid));
    n->waiters++;
    lotus_failure_wait_locked(n, self_tid);
    while (n->waiters > 1) held_delivered_wait();
    if (--n->waiters == 0) free(n);
    return 0;
}

/* 0 = call the handler in place (this thread is the owner's domain, or
 * the owner has none); 1 = posted to the owner's domain, and its
 * handler has returned. */
static int64_t lotus_failure_post(void *parent, lotus_failure_fn fn, void *child,
                                  const violation_t *err, int self_tid) {
#ifdef MODEL_BUG_DELIVER_IN_PLACE
    (void)parent; (void)fn; (void)child; (void)err; (void)self_tid;
    return 0;
#endif
    if (!parent ||
        atomic_load_explicit(&g_owner_domain_count, memory_order_acquire) == 0)
        return 0;
    pthread_mutex_lock(&g_params_open_lock);
    int d = -1;
    for (size_t i = 0; i < g_owner_domains_len; i++)
        if (g_owner_domains[i].owner == parent) d = g_owner_domains[i].tid;
    if (d < 0 || !g_domain_alive[d] || d == self_tid) {
        pthread_mutex_unlock(&g_params_open_lock);
        return 0;
    }
    lotus_failure_hold_cell(child, self_tid);
    lotus_held_failure_t *n = malloc(sizeof *n);
    violation_t *copy = malloc(sizeof *copy);
    copy->code = err->code;
    copy->detail = err->detail;
    n->next = NULL;
    n->parent = parent;
    n->opener = d;
    n->fn = fn;
    n->child = child;
    n->err = copy;
    n->reclaim = NULL;
    n->state = LOTUS_HELD;
    n->waiters = 1;                     /* this thread, until it resumes */
    n->posted = d;
    n->deliverer = -1;
    if (g_held_tail) g_held_tail->next = n; else g_held_head = n;
    g_held_tail = n;
    atomic_fetch_add_explicit(&lotus_held_failure_count, 1, memory_order_release);
    g_domain_pending[d]++;
    g_domain_refs[d]++;
    atomic_fetch_add_explicit(&g_failure_posted_count, 1, memory_order_release);
    held_delivered_broadcast();
    pthread_mutex_unlock(&g_params_open_lock);
    /* (the wake cell; then phase 3's probe: the delivery is in flight) */
    atomic_fetch_add_explicit(&g_posted_probe, 1, memory_order_release);
    pthread_mutex_lock(&g_params_open_lock);
    lotus_failure_wait_locked(n, self_tid);
    g_domain_refs[d]--;
    held_delivered_broadcast();
    if (--n->waiters == 0) free(n);
    pthread_mutex_unlock(&g_params_open_lock);
    return 1;
}

/* A thread's domain ends (the key's destructor; a worker leaving its
 * loop). */
static void lotus_domain_end(int tid) {
    pthread_mutex_lock(&g_params_open_lock);
    g_domain_alive[tid] = 0;
    held_delivered_broadcast();
    pthread_mutex_unlock(&g_params_open_lock);
}

/* A join's wait (join progress): the deliveries posted to this thread
 * run, nothing else, until the joined domain has ended and no poster
 * names it; then the pthread_join. */
static void lotus_domain_join_wait(int joined, int self_tid) {
    pthread_mutex_lock(&g_params_open_lock);
    while (g_domain_alive[joined] || g_domain_refs[joined] > 0) {
        if (g_domain_pending[self_tid] > 0 && !g_servicing[self_tid]) {
            lotus_failure_await_service_locked(self_tid);
            continue;
        }
        held_delivered_wait();
    }
    pthread_mutex_unlock(&g_params_open_lock);
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
        if (lotus_failure_defer_reclaim(c, child_teardown_at_settle, self_tid)) return;
    }
    child_teardown(c, self_tid);
}

/* What a handler does besides hearing the failure: phase 4's replaces a
 * sibling. NULL elsewhere. */
static void (*g_handler_body)(owner_t *o, child_t *c, int self_tid);

/* The owner's on_failure. */
static void owner_on_failure(void *parent, void *child, void *err, int self_tid) {
    owner_t *o = parent;
    child_t *c = child;
    violation_t *v = err;
    /* (6) on the owner's domain, never on the failing child's thread */
    assert(self_tid == o->domain);
    /* (8) never started inside another handler on the same thread */
    assert(g_handler_depth[self_tid] == 0);
    g_handler_depth[self_tid]++;
    /* (4) never before the owner is active: its last param is stored */
    assert(o->param == PARAM_SET);
    /* (2), (7) the child and the violation outlive the handler */
    arena_t *a = c->arena;
    assert(a != NULL);
    assert(a->alive == 1);
    assert(v->code == FAIL_CODE && v->detail == c->id);
    if (g_handler_body) g_handler_body(o, c, self_tid);
    o->heard++;
    g_delivered[c->id]++;
    g_handler_depth[self_tid]--;
}

/* The compiled delivery (emit_on_failure_call): held while the owner's
 * params are open, else posted to the owner's domain, else in place. */
static void child_raise(child_t *c, int self_tid) {
    violation_t err;
    err.code = FAIL_CODE;
    err.detail = c->id;
    if (!lotus_failure_hold(c->owner, owner_on_failure, c, &err) &&
        !lotus_failure_post(c->owner, owner_on_failure, c, &err, self_tid))
        owner_on_failure(c->owner, c, &err, self_tid);
}

/* A child's run() that fails: raise to the owner, learn the decision,
 * then the run end's reclaim. */
static void child_run_fails(child_t *c, int self_tid) {
    assert(c->arena != NULL && c->arena->alive == 1);
    child_raise(c, self_tid);
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
    atomic_store_explicit(&g_failure_posted_count, 0, memory_order_relaxed);
    atomic_store_explicit(&g_owner_domain_count, 0, memory_order_relaxed);
    atomic_store_explicit(&g_posted_probe, 0, memory_order_relaxed);
    g_params_open_len = 0;
    g_owner_domains_len = 0;
    g_held_head = g_held_tail = NULL;
    g_run_tickets = NULL;
    for (int t = 0; t < NTID; t++) {
        g_run_running[t] = NULL;
        g_domain_alive[t] = 1;
        g_domain_pending[t] = g_domain_refs[t] = 0;
        g_servicing[t] = 0;
        g_handler_depth[t] = 0;
    }
    g_handler_body = NULL;
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
    owner->domain = TID_INSTANTIATING;
    owner->heard = 0;
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
    lotus_params_settle(owner, TID_INSTANTIATING);
    owner->born = 1;

    /* Teardown: the pool join, which runs B's delivery if B failed after
     * the settle (join progress), then the owner's cascade over its
     * fields, each through the latch. */
    lotus_domain_join_wait(TID_WORKER, TID_INSTANTIATING);
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
    assert(atomic_load_explicit(&g_failure_posted_count,
                                memory_order_relaxed) == 0);     /* (7) */
    assert(owner->heard == 2);
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
    owner->domain = TID_INSTANTIATING;
    owner->heard = 0;
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
    lotus_domain_join_wait(TID_WORKER, TID_INSTANTIATING);
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

/* ==================================================================== *
 * Phase 3 — the delivery's domain: a handler cell fails on the pool
 * worker, its owner is on the instantiating thread, and the owner
 * reclaims the child while the delivery is in flight.
 * ==================================================================== */

/* A handler cell of the child's on the worker (it carries no run
 * ticket): it fails, learns the decision, then reads the child as what
 * follows the decision does; its dispatch releases the hold. */
static void child_d_cell(child_t *c, int self_tid) {
    assert(c->arena != NULL && c->arena->alive == 1);
    child_raise(c, self_tid);
    lotus_failure_await(c, 0, self_tid);
    /* (7) still whole after the decision, until the cell returns */
    arena_t *a = c->arena;
    assert(a != NULL && a->alive == 1);
}

/* A worker running one failing handler cell, its dispatch releasing the
 * hold the post took, then leaving its loop. */
static void child_cell_worker(child_t *c, int self_tid) {
    g_run_running[self_tid] = NULL;            /* a bus cell: no hold */
    child_d_cell(c, self_tid);
    lotus_run_ticket_t *took = g_run_running[self_tid];
    g_run_running[self_tid] = NULL;
    lotus_run_hold_release(took);
    lotus_failure_service_here(self_tid);
    lotus_domain_end(self_tid);
}

static child_t *g_phase3_child;

static void *phase3_worker(void *_) {
    (void)_;
    child_cell_worker(g_phase3_child, TID_WORKER);
    return NULL;
}

static void phase3_delivery_domain(int yield_first) {
    reset();
    owner_t *owner = malloc(sizeof *owner);
    owner->param = PARAM_SET;
    owner->born = 1;
    owner->domain = TID_INSTANTIATING;
    owner->heard = 0;
    /* Its params opened (and settled) on this thread: its domain. */
    pthread_mutex_lock(&g_params_open_lock);
    lotus_failure_owner_note_locked(owner, TID_INSTANTIATING);
    pthread_mutex_unlock(&g_params_open_lock);
    child_t *c = child_create(owner, 3);
    g_phase3_child = c;
    pthread_t w;
    pthread_create(&w, NULL, phase3_worker, NULL);

    /* The owner observes the delivery in flight (the fixture's gate),
     * then its own window, with no yield: nothing changes in it (6). */
    while (!atomic_load_explicit(&g_posted_probe, memory_order_acquire)) { }
    int h1 = owner->heard;
    int h2 = owner->heard;
    assert(h1 == h2 && h1 == 0);
    if (yield_first) {
        /* A yield: the delivery runs here. */
        lotus_failure_service_here(TID_INSTANTIATING);
        assert(owner->heard == 1);
    }
    /* `self.k = Kid { }`: the reclaim waits for the posted delivery
     * (running it, if no yield did) and for the cell's hold (7). */
    child_reclaim_spine(c, TID_INSTANTIATING);
    assert(owner->heard == 1 && g_teardowns[3] == 1);

    lotus_domain_join_wait(TID_WORKER, TID_INSTANTIATING);
    pthread_join(w, NULL);

    assert(g_delivered[3] == 1 && g_teardowns[3] == 1);  /* (7) once each */
    assert(g_held_head == NULL);
    assert(atomic_load_explicit(&lotus_held_failure_count,
                                memory_order_relaxed) == 0);
    assert(atomic_load_explicit(&g_failure_posted_count,
                                memory_order_relaxed) == 0);
    assert(atomic_load_explicit(&g_run_tickets_live, memory_order_relaxed) == 0);
    assert(g_run_tickets == NULL);
    free(c);
    free(owner);
}

/* ==================================================================== *
 * Phase 4 — a sibling replaced from a handler: the owner's handler for
 * A replaces B while B's failure is posted to the owner, undelivered.
 * ==================================================================== */

static child_t *g_phase4_a, *g_phase4_b;
static _Atomic int g_phase4_go;            /* the message B fails on */

/* A's handler: publish what B fails on, wait for B's post (the
 * fixture's gate file), then `self.b = Kid { }`, the old B's reclaim,
 * from inside this handler. */
static void phase4_replace_sibling(owner_t *o, child_t *c, int self_tid) {
    (void)o;
    if (c != g_phase4_a) return;
    atomic_store_explicit(&g_phase4_go, 1, memory_order_release);
    while (atomic_load_explicit(&g_posted_probe, memory_order_acquire) < 2) { }
    child_t *b = g_phase4_b;
    child_reclaim_spine(b, self_tid);
    /* (8) deferred behind B's delivery: the old B is still whole and
     * not yet heard */
    assert(b->arena != NULL && b->arena->alive == 1);
    assert(g_teardowns[b->id] == 0 && g_delivered[b->id] == 0);
}

static void *phase4_worker_a(void *_) {
    (void)_;
    child_cell_worker(g_phase4_a, TID_WORKER);
    return NULL;
}

static void *phase4_worker_b(void *_) {
    (void)_;
    while (!atomic_load_explicit(&g_phase4_go, memory_order_acquire)) { }
    child_cell_worker(g_phase4_b, TID_WORKER2);
    return NULL;
}

static void phase4_sibling_replaced(void) {
    reset();
    atomic_store_explicit(&g_phase4_go, 0, memory_order_relaxed);
    owner_t *owner = malloc(sizeof *owner);
    owner->param = PARAM_SET;
    owner->born = 1;
    owner->domain = TID_INSTANTIATING;
    owner->heard = 0;
    pthread_mutex_lock(&g_params_open_lock);
    lotus_failure_owner_note_locked(owner, TID_INSTANTIATING);
    pthread_mutex_unlock(&g_params_open_lock);
    child_t *a = child_create(owner, 4);
    child_t *b = child_create(owner, 5);
    g_phase4_a = a;
    g_phase4_b = b;
    g_handler_body = phase4_replace_sibling;
    pthread_t wa, wb;
    pthread_create(&wa, NULL, phase4_worker_a, NULL);
    pthread_create(&wb, NULL, phase4_worker_b, NULL);

    /* A's failure is posted; a yield runs it, and B's after it returns,
     * then the old B's deferred reclaim, once B's poster has resumed. */
    while (!atomic_load_explicit(&g_posted_probe, memory_order_acquire)) { }
    lotus_failure_service_here(TID_INSTANTIATING);
    assert(g_delivered[4] == 1 && g_delivered[5] == 1);  /* (8) none dropped */
    assert(g_teardowns[5] == 1 && g_teardowns[4] == 0);
    assert(owner->heard == 2);

    lotus_domain_join_wait(TID_WORKER, TID_INSTANTIATING);
    pthread_join(wa, NULL);
    lotus_domain_join_wait(TID_WORKER2, TID_INSTANTIATING);
    pthread_join(wb, NULL);
    child_teardown(a, TID_INSTANTIATING);               /* the cascade */

    assert(g_teardowns[4] == 1 && g_teardowns[5] == 1);  /* (1) */
    assert(g_held_head == NULL);
    assert(atomic_load_explicit(&lotus_held_failure_count,
                                memory_order_relaxed) == 0);
    assert(atomic_load_explicit(&g_failure_posted_count,
                                memory_order_relaxed) == 0);
    assert(atomic_load_explicit(&g_run_tickets_live, memory_order_relaxed) == 0);
    assert(g_run_tickets == NULL);
    free(a);
    free(b);
    free(owner);
}

int main(void) {
    pthread_mutex_init(&g_params_open_lock, NULL);
    pthread_mutex_init(&g_run_tickets_lock, NULL);
    phase1_construction_delivery();
    phase2_unjoined_reclaim();
    phase3_delivery_domain(0);
    phase3_delivery_domain(1);
    phase4_sibling_replaced();
    return 0;
}
