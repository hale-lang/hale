/*
 * Hale native observation emission — iris handoff P4 (2026-07-27).
 *
 * Implements the iris observation protocol (PROTOCOL.md v0.2 on
 * the iris-observer `observer` branch) as a runtime capability:
 * with `LOTUS_OBS=1` in the environment, the process publishes a
 * `/hale-obs-<pid>` shm segment + registration file, and the
 * runtime's own choke points emit protocol records — NET_SEND /
 * NET_DELIVER at the transport layer (with real per-binding
 * seqs, the records iris seq-matches into cross-process edges),
 * BUS_PUBLISH / BUS_DELIVER at dispatch (locus-attributed),
 * LOCUS_BIRTH / DISSOLVE / RESTART from the lifecycle paths —
 * lighting up every binary of a stack with zero app changes.
 *
 * Segment machinery adapted from the protocol's reference
 * implementation (iris-observer/observe/glue.c + emitter/
 * protocol.h — layouts pinned there by static_asserts; where
 * this file and PROTOCOL.md disagree, that is a bug).
 *
 * Own TU (the lotus_tls.c rationale): helper binaries including
 * lotus_arena.c directly don't pick up the probe surface; the
 * arena TU calls into these hooks through weak-ref-shaped
 * always-defined externs instead.
 *
 * Cost contract: LOTUS_OBS unset → `g_obs_state` stays 0 and
 * every probe is one predictable branch. Enabled but no observer
 * attached (`observer_count == 0`) → counters only, no ring
 * writes (§5 dormant rule). Ring emission is SPSC per the #247
 * primitive: each emitting THREAD owns one ring (TLS
 * assignment, first-come; threads beyond ring_count count into
 * ring_drops_total rather than corrupting a ring).
 *
 * v0 scope notes (each is a PROTOCOL §13 conversation):
 *   - BUS_DELIVER is emitted at fanout/enqueue time per target
 *     (the subject + target locus are in scope there); consumer
 *     -side dispatch latency is therefore not in the deliver
 *     record. Counter `delivered` matches enqueue too.
 *   - Manifest ids are registration-order (the protocol's
 *     "library" rule); `.hale.topo` native ordering lands with
 *     that metadata section.
 *   - On the observer_count 0→1 rising edge the live locus
 *     table is replayed as LOCUS_BIRTH records (late-attach
 *     tree reconstruction, §13 candidate) from whichever
 *     probe thread notices the edge.
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <signal.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define OBS_MAGIC 0x4F42534948414C45ULL
#define OBS_PAGE 4096
#define OBS_ENTRY_CAP 256
#define OBS_INSTANCE_CAP 4096
#define OBS_TS_DELTA_MAX 0x7FFFFFFFULL

/* ekinds (PROTOCOL §8) */
#define EK_EPOCH 0
#define EK_BUS_PUBLISH 1
#define EK_BUS_DELIVER 2
#define EK_NET_SEND 3
#define EK_NET_DELIVER 4
#define EK_LOCUS_BIRTH 5
#define EK_LOCUS_DISSOLVE 6
#define EK_RESTART 7
/* GH #296 Phase 1 (recording mode only — never emitted under plain
 * LOTUS_OBS=1, so a pre-addendum consumer attached to an observed
 * process never sees it): a dequeue-driven handler invoke on the
 * consuming thread. BUS_DELIVER is enqueue-time by design (§5); this
 * record is the CONSUMPTION event replay reconstructs order from.
 * Pairing rule: per queue, the k-th BUS_CONSUME corresponds to the
 * k-th queued delivery (FIFO per single consumer thread). Synchronous
 * direct-dispatch flavors have no consume record — their handler runs
 * at the BUS_DELIVER position on the same ring, which IS the
 * consumption point. w1 packs locus:20 (subscriber) | pub_id:44
 * (the delivery's deterministic identity — see the v0.2 note);
 * id = 0. */
#define EK_DROP_MARK 14

/* GH #296 recorder events live in a PRIVATE event namespace on
 * PRIVATE per-thread rings that are never part of the public
 * observation segment — iris protocol ekinds 8/9/11/12 are
 * SUPERV_TRANS/PLACEMENT/BINDING_UP/BINDING_DOWN, and an observer
 * attaching to a recording process must never decode recorder
 * bookkeeping as lifecycle transitions (review of PR #463,
 * finding 1). In the recording file, tag-0 entries from private
 * rings carry ring | OBS_REC_PRIV_RING; the two namespaces can
 * never be confused because they never share a ring.
 *
 *   REC_EV_CONSUMER 1  — private ring's first record; w1 = the
 *                        claiming thread's stable consumer id
 *   REC_EV_CONSUME  2  — dequeue-driven handler invoke;
 *                        w1 = locus:20 | pub_id:44
 *   REC_EV_ENQ      3  — queued target at enqueue; same w1 shape
 *   REC_EV_JOURNAL  4  — journaled read marker;
 *                        w1 = jkind:8 << 56 | per-thread seq
 */
#define REC_EV_CONSUMER 1
#define REC_EV_CONSUME 2
#define REC_EV_ENQ 3
#define REC_EV_JOURNAL 4
#define OBS_REC_PRIV_RING 0x80000000u

/* manifest kinds */
#define MK_TOPIC 0
#define MK_LOCUS_TYPE 1
#define MK_BINDING 2

typedef struct {
  uint64_t magic;
  uint16_t proto_major, proto_minor;
  uint32_t header_len;
  uint64_t total_len;
  uint32_t pid, ring_count, ring_slots, ts_shift;
  uint64_t started_mono_ns, started_wall_ns;
  uint64_t control_off, manifest_off, manifest_len, modemask_off,
           counters_off, counters_len, rings_off;
  _Atomic uint64_t flags;
  _Atomic uint64_t manifest_gen;
  /* iris handoff-12 P26 (proto 0.2): the topology model's
   * shape_hash, stamped at segment creation from the value codegen
   * embedded at build time. 0 = unstamped (harness builds).
   * Complementary to the per-topic payload shape_hash rows: model
   * identity deliberately excludes payload field shape. */
  uint64_t model_hash;
} obs_hdr_t;

typedef struct { _Atomic uint32_t observer_count, sample_n; } obs_ctrl_t;
typedef struct { _Atomic uint32_t entry_count; uint32_t entry_cap, pool_off;
                 _Atomic uint32_t pool_used; } obs_mh_t;
typedef struct { uint64_t shape_hash, aux_b; uint32_t id, name_off;
                 uint16_t name_len, aux_a; uint8_t kind, flags;
                 uint16_t _pad; } obs_me_t;
typedef struct { _Atomic uint64_t c[8]; } obs_cline_t;
/* MUST match lotus_spsc_desc_t (lotus_arena.c) and PROTOCOL §9. */
typedef struct { uint64_t data_off; _Atomic uint64_t head, dropped;
                 uint32_t tag_a; _Atomic uint32_t tag_b;
                 uint8_t reserved[32]; } obs_rdesc_t;

/* #247 primitives (lotus_arena.c). */
void lotus_spsc_emit(void *seg_base, void *desc, int64_t ring_slots,
                     int64_t w0, int64_t w1);
void lotus_spsc_note_drop(void *desc);

/* 0 = unchecked, 1 = disabled, 2 = enabled. */
static _Atomic int g_obs_state = 0;

/* P26: build-time model identity, set by the codegen prelude before
 * any probe can lazily create the segment. */
static uint64_t g_obs_model_hash = 0;
void lotus_obs_model_hash_set(uint64_t h) { g_obs_model_hash = h; }
/* GH #296 review finding 2: shape_hash is structural compatibility,
 * not executable identity. The CLI stamps a digest of the compiler
 * version + every source byte; exact replay admits on THIS. */
static uint64_t g_obs_exec_digest[4] = {0, 0, 0, 0};
void lotus_obs_exec_digest_set(uint64_t part, uint64_t v) {
  if (part < 4) g_obs_exec_digest[part] = v;
}

/* iris handoff-2 P10: publisher attribution. Codegen notes the
 * publishing locus's self in this TLS right before lowering a
 * `<-` dispatch from inside a locus body (NULL from free-fn
 * contexts), and bus_publish falls back to it when its explicit
 * arg is NULL. */
static __thread void *g_obs_tls_publisher = NULL;
void lotus_obs_note_publisher(void *self) {
  g_obs_tls_publisher = self;
}

/* iris handoff-4 P15: negative-marking of inbound wire re-dispatch.
 * The reader thread re-dispatches every received wire message through
 * the SAME lotus_bus_local_dispatch a genuine publish uses; those are
 * deliveries (already covered by NET_DELIVER + BUS_DELIVER), NOT
 * publishes, and must not inflate the published counter.
 *
 * handoff-3 P13 excluded them by POSITIVE marking — count a publish
 * only when the publisher TLS is set. That was fragile: a cross-pool
 * wire dispatch runs bus_publish on a worker thread that never saw
 * the publishing thread's TLS, and a free-fn publish sets it NULL, so
 * genuine publishes were silently dropped from the counter (the
 * fleet's pub=0). The counter is the dormant-mode contract and must
 * not depend on attribution at all.
 *
 * So invert it: the reader marks its re-dispatch window and
 * bus_publish consumes the mark. Genuine publishes are the unmarked
 * default and always count; attribution stays best-effort. The mark
 * is consume-once so a nested publish from a subscriber handler
 * running in the same fanout is still counted. */
static __thread int g_obs_tls_redispatch = 0;
void lotus_obs_begin_redispatch(void) { g_obs_tls_redispatch = 1; }
void lotus_obs_end_redispatch(void) { g_obs_tls_redispatch = 0; }

/* iris handoff-2 P6: canonical topic shapes, registered by
 * codegen at program start (before the segment exists — obs
 * init is lazy). shape_hash = fnv(subject, canonical payload
 * structure), NEVER the declaring type's name, so two binaries
 * declaring the same subject under different local topic names
 * fuse into one row (the PROTOCOL §13 canonicalization rule
 * proposed by the field report). */
typedef struct { const char *subject; const char *shape; } obs_shape_t;
static obs_shape_t g_shapes[OBS_ENTRY_CAP];
static _Atomic int g_shape_count = 0;

void lotus_obs_topic_shape(const char *subject, const char *shape) {
  if (!subject || !shape) return;
  int n = atomic_load(&g_shape_count);
  if (n >= OBS_ENTRY_CAP) return;
  for (int i = 0; i < n; i++) {
    if (strcmp(g_shapes[i].subject, subject) == 0) return;
  }
  g_shapes[n].subject = strdup(subject);
  g_shapes[n].shape = strdup(shape);
  atomic_store_explicit(&g_shape_count, n + 1, memory_order_release);
}

static const char *obs_shape_for(const char *subject) {
  int n = atomic_load_explicit(&g_shape_count, memory_order_acquire);
  for (int i = 0; i < n; i++) {
    if (strcmp(g_shapes[i].subject, subject) == 0) {
      return g_shapes[i].shape;
    }
  }
  return "";
}

static void *g_seg; static size_t g_seg_len;
/* iris handoff-3 P11/P12: a stable nonzero 16-bit per-process
 * identity. NET_SEND stamps it; the wire carries it; NET_DELIVER
 * echoes the WIRE origin (not the reader's) — so iris matches a
 * send and its delivers on (origin, seq), which is unique across
 * the fleet even with multiple senders multicasting one subject
 * (the receiver-local delivery count summed across senders was
 * the zero-edges cause). Folded from the full pid; never 0 (0 is
 * the unattributed/legacy sentinel). */
static int obs_on(void);
static uint16_t g_obs_origin = 0;
uint64_t lotus_obs_origin(void) { return g_obs_origin; }
int lotus_obs_active(void) { return obs_on(); }
static obs_hdr_t *H; static obs_ctrl_t *C; static obs_mh_t *MH;
static obs_me_t *ME; static char *POOL; static uint8_t *MODE;
static obs_cline_t *CNT; static obs_rdesc_t *RD;
static char g_shm_name[64], g_reg_path[280];
static int g_cnt_line_for[4][OBS_ENTRY_CAP];
static pthread_mutex_t g_obs_lock = PTHREAD_MUTEX_INITIALIZER;

/* subject → topic id cache (linear; manifest is <= 256). */
typedef struct { const char *subject; int64_t id;
                 _Atomic uint64_t seq; } obs_topic_slot_t;
static obs_topic_slot_t g_topics[OBS_ENTRY_CAP];
static _Atomic int g_topic_count = 0;

/* locus instance table: self_ptr → u20 instance id + liveness
 * (for the 0→1 birth replay). */
typedef struct { void *self; uint32_t id, type_id, parent; int live; }
    obs_inst_t;
static obs_inst_t g_inst[OBS_INSTANCE_CAP];
static _Atomic int g_inst_count = 0;
static _Atomic uint32_t g_next_inst_id = 1;

/* per-thread ring assignment (SPSC: one producer per ring). */
static _Atomic int g_ring_next = 0;
static __thread int t_ring = -1;          /* -1 unassigned, -2 none free */
static __thread uint64_t t_epoch_ns = 0;  /* ring epoch anchor */
/* iris handoff-2 P7: records emitted since the last EPOCH. A
 * high-rate ring wraps and OVERWRITES its EPOCH record; a
 * consumer that attaches (or falls behind) then reconstructs
 * from base 0 — observed as ~2^64 ns timestamps on the fleet's
 * hottest segment. Re-emit EPOCH every OBS_EPOCH_EVERY records
 * so every ring window contains at least one anchor. */
#define OBS_EPOCH_EVERY 1024
static __thread uint32_t t_since_epoch = 0;

/* ---- lossless recording mode (GH #296 Phase 1) ------------------
 *
 * LOTUS_OBS_RECORD=<path> opts a run into recording: every ring
 * record is drained to <path> by an in-process thread, and the
 * emission disposition changes from overwrite-oldest to
 * BLOCK-THE-PRODUCER — a live observer prefers losing records to
 * stalling the observed program; a recording that drops a record is
 * a replay that diverges silently, so it must never drop (RFC #296
 * delta 1). Concretely:
 *
 *   - a producer whose ring is full against the drain cursor waits
 *     for the drain instead of overwriting;
 *   - a thread that cannot get a ring at all FAILS THE RUN loudly
 *     (recording defaults to the 64-ring maximum, so this means
 *     >64 emitting threads);
 *   - a drain-side write error fails the run, never truncates
 *     silently.
 *
 * Cost contract: `g_obs_recording` is process-constant from the
 * same constructor that resolves `lotus_obs_live`, so the unset
 * default is one predictable branch on the paths that already pay
 * one for observation — the LOTUS_OBS-unset lowering stays
 * instruction-identical. Recording-on hot cost is one relaxed
 * cursor load + compare per record while the ring has room.
 *
 * File format v0.1 (PRE-STABLE — the recording artifact proper is
 * RFC #296 Phase 2): 64-byte header, then 24-byte entries
 * (u32 ring, u32 reserved, u64 w0, u64 w1) in drain order (per-ring
 * order preserved; cross-ring interleaving meaningless), then a
 * 16-byte trailer whose presence marks a clean finalize. */
int lotus_obs_recording = 0; /* read (weak) from lotus_arena.c */
/* Review round 2, finding 8: env VALUES are redacted from the
 * journal by default — names, existence, and lengths are recorded;
 * the value itself is withheld unless LOTUS_OBS_RECORD_ENV=full.
 * A withheld read replays as a NAMED divergence (live fallback),
 * never as a silently substituted value, and the artifact header
 * says which policy produced it. */
static int g_rec_env_full = 0;
/* Defined (with its story) next to obs_gate; obs_create pre-arms
 * it under recording. Tentative declaration so it's visible here. */
static _Atomic uint32_t g_last_observed;
static char g_rec_path[512];
static FILE *g_rec_file = NULL;
static pthread_t g_rec_thread;
static int g_rec_thread_started = 0;
static _Atomic int g_rec_stop = 0;
static _Atomic uint64_t g_rec_cursor[64]; /* per-ring drained head */
static uint64_t g_rec_written = 0;        /* drain-thread-owned */
static int g_rec_durable = 0; /* LOTUS_OBS_RECORD_DURABLE=1 */
static __thread uint64_t t_consume_seq = 0;

#define OBS_REC_MAGIC 0x30434552454C4148ULL /* "HALEREC0" */
#define OBS_REC_END 0x30444E45454C4148ULL   /* "HALEEND0" */

/* ---- format v0.2: tagged entry stream (GH #296 Phase 2/3) ------
 *
 * The v0.1 stream was homogeneous 24-byte ring records. v0.2 tags
 * every entry so payload blobs and journal values ride the same
 * file (still PRE-STABLE — the self-describing artifact is a later
 * phase):
 *
 *   tag 0 (ring):    u32 tag, u32 ring, u64 w0, u64 w1        (24B)
 *   tag 1 (payload): u32 tag, u32 topic_id, u64 pub_id,
 *                    u64 flags (bit 0 = external ingress),
 *                    u64 size, size bytes padded to 8
 *   tag 2 (journal): u32 tag, u32 jkind, u64 consumer_id,
 *                    u64 seq, u64 size, size bytes padded to 8
 *
 * Identity. msg_id = consumer_id:16 << 48 | per-thread publish
 * seq:48 — deterministic across runs (a re-executed publisher
 * thread re-derives the same ids in the same order), which is what
 * lets a replay match a re-executed delivery to its recorded
 * consume without any global coordination. 48-bit seqs do not
 * wrap in any realistic recording (~8,900 years at 1M msg/s), and
 * range guards below FAIL LOUDLY rather than letting components
 * overlap. 0 = unidentified (structural cells: run starts,
 * cross-pool creates — matched by per-queue count, not id).
 *
 * Consumer identity. Stable across runs, unlike pthread ids:
 * main = 1, cooperative pool workers = 16 + registration index,
 * pinned locus threads = 64 + obs instance id (guarded: an id
 * reaching the anonymous floor fails the recording), anonymous
 * (ingress) threads = 60000 + claim order — a range DISJOINT from
 * every stable class by construction. The drain and heartbeat
 * never claim one. Carried in each private ring's first record
 * and in journal entries. */
#define OBS_REC_ANON_BASE 60000u
#define OBS_REC_CONSUMER_MAX 0xFFFFu
#define OBS_REC_TAG_RING 0u
#define OBS_REC_TAG_PAYLOAD 1u
#define OBS_REC_TAG_JOURNAL 2u
/* v0.3: run-stable identity maps for the comparator — a = subtype
 * (1 = topic id→subject name, 2 = public ring→consumer id),
 * b/cfield = ids, bytes = name (subtype 1). Manifest topic ids and
 * public ring indices are per-run registration/claim order, so the
 * file must carry its own maps for cross-run comparison. */
#define OBS_REC_TAG_META 3u
#define OBS_REC_META_TOPIC 1u
#define OBS_REC_META_PUBRING 2u
/* GH #296 phase 5b: subject-hash → name map. PAYLOAD records carry
 * the stable FNV of the subject (registration-order manifest ids
 * race across runs); this subtype lets a reader NAME a payload's
 * subject without a live registry — which is what makes feed-mode
 * "recorded topic X has no subscriber here" reports speakable. */
#define OBS_REC_META_SUBJHASH 3u

/* journal kinds — one per interposed runtime read */
#define JK_TIME_NOW 1u
#define JK_TIME_MONO_NS 2u
#define JK_RAND_NEXT_INT 3u
#define JK_OS_GETRANDOM 4u
#define JK_ENV_VAR 5u
#define JK_ENV_VAR_EXISTS 6u
#define JK_ENV_ARG 7u
#define JK_ENV_ARGS_COUNT 8u

static __thread uint64_t t_consumer_id = 0; /* 0 = unassigned */
static __thread uint64_t t_pub_seq = 0;
static __thread uint64_t t_journal_seq = 0;
/* GH #296 phase 5b: a pub_id decided BEFORE the dispatch that would
 * otherwise derive one. Set by the ingress wire capture (the reader
 * records the verbatim wire bytes and the dispatch must reuse that
 * record's identity, not mint a second) and by the replay injector
 * (an injected delivery must carry its RECORDED identity so the
 * per-consumer order enforcement can match it). Consumed once. */
static __thread uint64_t t_rec_forced_pub = 0;

/* The stable subject hash payload records carry (see the v0.3 note
 * in lotus_obs_record_publish_payload). */
static uint32_t obs_subject_hash(const char *subject) {
  uint32_t h = 2166136261u;
  for (const char *q = subject; *q; q++) {
    h ^= (uint8_t)*q;
    h *= 16777619u;
  }
  return h;
}

/* Capture side-channel: payload + journal blobs travel from the
 * emitting thread to the drain on an MPSC push list (atomic
 * exchange head), bounded by a byte budget — a producer over
 * budget BLOCKS (the recording disposition), never drops. */
typedef struct obs_rec_blob {
  struct obs_rec_blob *next;
  uint32_t tag;      /* OBS_REC_TAG_PAYLOAD / _JOURNAL */
  uint32_t a;        /* topic_id / jkind */
  uint64_t b;        /* pub_id / consumer_id */
  uint64_t c;        /* flags / seq */
  uint64_t size;
  /* bytes follow */
} obs_rec_blob_t;
/* Private recorder rings: one per emitting thread, same 16-byte
 * slot + release-store discipline as the public SPSC, but in
 * process-private memory (see the REC_EV note above). The drain
 * sweeps them with the same blocking-cursor contract. */
typedef struct {
  uint64_t *slots;           /* H->ring_slots * 2 u64s */
  _Atomic uint64_t head;
  uint64_t consumer;
} obs_rec_ring_t;
static obs_rec_ring_t g_rec_rings[64];
static _Atomic int g_rec_ring_next = 0;
static __thread int t_rec_ring = -2; /* -2 = unassigned */
static _Atomic uint64_t g_rec_priv_cursor[64];

static void obs_rec_claim(void);

static obs_rec_blob_t *_Atomic g_rec_blob_head = NULL;
static _Atomic uint64_t g_rec_blob_bytes = 0;
#define OBS_REC_BLOB_BUDGET (64ull << 20)

static void obs_rec_blob_push(uint32_t tag, uint32_t a, uint64_t b,
                              uint64_t c, const void *bytes,
                              uint64_t size) {
  /* Reserve BEFORE allocating (concurrent producers cannot
   * overshoot the budget), back off while over. */
  uint64_t need = sizeof(obs_rec_blob_t) + size;
  if (size > OBS_REC_BLOB_BUDGET / 2 || need < size) {
    fprintf(stderr,
            "hale: LOTUS_OBS_RECORD capture of %llu bytes exceeds "
            "the recorder's budget — failing the run rather than "
            "hanging on an unsatisfiable reservation\n",
            (unsigned long long)size);
    fflush(NULL);
    _exit(74);
  }
  for (;;) {
    uint64_t prev = atomic_fetch_add_explicit(
        &g_rec_blob_bytes, need, memory_order_relaxed);
    if (prev + need <= OBS_REC_BLOB_BUDGET) break;
    atomic_fetch_sub_explicit(&g_rec_blob_bytes, need,
                              memory_order_relaxed);
    if (atomic_load_explicit(&g_rec_stop, memory_order_relaxed))
      return;
    struct timespec ts = {0, 50 * 1000};
    nanosleep(&ts, NULL);
  }
  obs_rec_blob_t *n = malloc(need);
  if (!n) {
    /* Never a silent hole: a recording that cannot capture is a
     * failed recording (review finding 4). */
    fprintf(stderr,
            "hale: LOTUS_OBS_RECORD capture allocation failed — "
            "failing the run rather than recording a gap\n");
    fflush(NULL);
    _exit(74);
  }
  n->tag = tag; n->a = a; n->b = b; n->c = c; n->size = size;
  if (size && bytes) memcpy(n + 1, bytes, size);
  obs_rec_blob_t *old = atomic_load_explicit(&g_rec_blob_head,
                                             memory_order_relaxed);
  do {
    n->next = old;
  } while (!atomic_compare_exchange_weak_explicit(
      &g_rec_blob_head, &old, n, memory_order_release,
      memory_order_relaxed));
}

typedef struct {
  uint64_t magic;
  uint16_t maj, min;
  uint32_t header_len; /* 96 */
  uint32_t pid, ring_count, ring_slots, ts_shift;
  uint64_t started_mono_ns, started_wall_ns;
  uint64_t model_hash;
  /* Framed SHA-256 build-manifest digest (full 32 bytes — review
   * round 2, finding 2): toolchain source hash + compiler version
   * + build options + framed source paths/lengths/contents. All
   * zero = unstamped. Env-value redaction state rides flags. */
  uint64_t exec_digest[4];
  uint64_t flags; /* bit 0 = env values redacted */
} obs_rec_hdr_t;
_Static_assert(sizeof(obs_rec_hdr_t) == 96, "recording header is 96B");

typedef struct {
  uint32_t tag; /* OBS_REC_TAG_RING */
  uint32_t ring;
  uint64_t w0, w1;
} obs_rec_entry_t;
_Static_assert(sizeof(obs_rec_entry_t) == 24, "recording entry is 24B");

/* Stable per-thread consumer identity (see the v0.2 note above).
 * The main thread is stamped in the same pre-main constructor
 * that resolves the flags; workers and pinned threads are stamped
 * by the arena at thread start. */
void lotus_obs_note_consumer(uint64_t id) { t_consumer_id = id; }
uint64_t lotus_obs_consumer_id(void) { return t_consumer_id; }

/* ---- replay (GH #296, LOTUS_REPLAY=<path>) ----------------------
 *
 * Loads a v0.2 recording and serves it back: journal values
 * (time/entropy/env) FIFO per (consumer, jkind), the per-consumer
 * consume order for Phase-4 enforcement, payload blobs for
 * ingress/diff. Serving degrades, never refuses (RFC #296 Q6): a
 * miss falls back to the live read and counts as a divergence,
 * summarized on stderr at teardown.
 *
 * LOTUS_REPLAY implies observation: consumer identity, instance
 * ids, and the pub_id assignment all ride the obs machinery, so a
 * replayed run creates the segment exactly as a recorded one does.
 * The file mapping is kept for process life — served strings are
 * pointers into it. */
int lotus_replay_active = 0;
/* GH #296 phase 5b: feed mode (LOTUS_REPLAY_FEED=<path>) — the
 * backtesting contract. The recording is consumed as an INPUT TAPE
 * only: recorded ingress is injected, transport bindings are
 * suppressed (the wire belongs to the tape), and nothing else of
 * replay applies — no journal serving, no order enforcement, no
 * model admission. Same inputs, changed code, live everything else. */
int lotus_replay_feed = 0;
static char g_replay_path[512];
static const uint8_t *g_rp_base; /* mmap'd recording */
static int g_rp_truncated = 0; /* accepted without a finalize trailer */
static uint64_t g_rp_dropped_tail = 0;
static size_t g_rp_len;
static uint64_t g_replay_at = 0; /* LOTUS_REPLAY_AT: stop at Nth consume */
static uint64_t g_replay_at_consumer = 0; /* 0 = process-wide count */

#define RP_MAX_CONSUMERS 96
#define RP_MAX_JK 9

typedef struct {
  const uint8_t *args;   /* framed invocation arguments */
  uint64_t args_len;
  const uint8_t *p;      /* result bytes */
  uint64_t size;
  uint32_t jkind;
  uint32_t withheld;     /* value redacted at record time */
} rp_blob_ref_t;
typedef struct { uint32_t locus; uint64_t msg; } rp_consume_t;
static uint32_t obs_inst_id_of(void *self);
typedef struct {
  rp_blob_ref_t *items;
  uint32_t len, cap, cursor;
} rp_queue_t;
typedef struct {
  uint64_t consumer;
  /* ONE unified input queue per consumer (review finding 7): the
   * next read must match in kind AND argument identity, so a
   * cross-kind reordering or a changed argument is the FIRST
   * named divergence instead of a plausible wrong value. */
  rp_queue_t journal;
  rp_consume_t *consume;           /* recorded delivery order */
  uint32_t consume_len, consume_cap, consume_cursor;
} rp_consumer_t;
static rp_consumer_t g_rp_consumers[RP_MAX_CONSUMERS];
static int g_rp_consumer_count = 0;
static _Atomic uint64_t g_rp_divergences[RP_MAX_JK];
static _Atomic uint64_t g_rp_consume_count = 0;
/* Round 3 follow-up: deliveries past the recorded stream's end (or
 * on a consumer the recording never saw) are counted, not silently
 * unconstrained — plain `hale replay` reports them even without
 * --diff. Structural cells (pub_id 0) stay uncounted. */
static _Atomic uint64_t g_rp_unexpected = 0;
void lotus_replay_note_unexpected(void) {
  atomic_fetch_add_explicit(&g_rp_unexpected, 1, memory_order_relaxed);
}
typedef struct { uint64_t pub_id; uint32_t topic; uint32_t flags;
                 rp_blob_ref_t bytes; } rp_payload_t;
static rp_payload_t *g_rp_payloads = NULL;
static uint32_t g_rp_payload_len = 0, g_rp_payload_cap = 0;
/* phase 5b: subject-hash → name (META_SUBJHASH), and the ingress
 * tape — indices into g_rp_payloads, artifact order — that the
 * injector walks. */
typedef struct { uint32_t hash; const char *name; } rp_subj_t;
static rp_subj_t *g_rp_subjects = NULL;
static uint32_t g_rp_subject_len = 0, g_rp_subject_cap = 0;
static uint32_t *g_rp_ingress = NULL;
static uint32_t g_rp_ingress_len = 0, g_rp_ingress_cap = 0;
static _Atomic uint64_t g_rp_injected = 0;
static _Atomic uint64_t g_rp_inject_dropped = 0;
static _Atomic uint64_t g_rp_bindings_suppressed = 0;

/* Creation happens ONLY during rp_load (single-threaded, pre-main);
 * at runtime every caller is lookup-only — the table is immutable
 * and per-consumer cursors are touched only by their own thread,
 * so no lock is needed anywhere. */
static rp_consumer_t *rp_consumer_create(uint64_t cid) {
  for (int i = 0; i < g_rp_consumer_count; i++) {
    if (g_rp_consumers[i].consumer == cid) return &g_rp_consumers[i];
  }
  if (g_rp_consumer_count >= RP_MAX_CONSUMERS) return NULL;
  rp_consumer_t *c = &g_rp_consumers[g_rp_consumer_count++];
  memset(c, 0, sizeof *c);
  c->consumer = cid;
  return c;
}
static rp_consumer_t *rp_consumer(uint64_t cid) {
  for (int i = 0; i < g_rp_consumer_count; i++) {
    if (g_rp_consumers[i].consumer == cid) return &g_rp_consumers[i];
  }
  return NULL;
}

static void rp_queue_push(rp_queue_t *q, rp_blob_ref_t e) {
  if (q->len == q->cap) {
    q->cap = q->cap ? q->cap * 2 : 16;
    rp_blob_ref_t *g = realloc(q->items,
                               q->cap * sizeof(rp_blob_ref_t));
    if (!g) {
      fprintf(stderr, "hale: LOTUS_REPLAY out of memory\n");
      _exit(66);
    }
    q->items = g;
  }
  q->items[q->len++] = e;
}

static uint64_t rp_ring_consumer[64]; /* ring idx → consumer id */

static void rp_fail(const char *why) {
  fprintf(stderr, "hale: LOTUS_REPLAY `%s`: %s\n", g_replay_path, why);
  fflush(NULL);
  _exit(66);
}

static void rp_load(void) {
  /* Round 3: the validated artifact must be an IMMUTABLE snapshot.
   * MAP_PRIVATE does not provide that (whether later file writes
   * appear through a private mapping is unspecified), so the bytes
   * are READ once into anonymous memory and every served pointer
   * aliases that copy — post-validation file mutation is
   * structurally irrelevant. The fd itself comes from the CLI when
   * available (LOTUS_REPLAY_FD, inherited): the object the CLI
   * validated and admitted is the object this process reads, with
   * no path re-resolution window. Direct LOTUS_REPLAY invocation
   * (no fd) opens once with O_NOFOLLOW and revalidates fully. */
  int fd = -1;
  const char *fdenv = getenv("LOTUS_REPLAY_FD");
  if (fdenv && fdenv[0]) fd = atoi(fdenv);
  if (fd < 0) fd = open(g_replay_path, O_RDONLY | O_NOFOLLOW);
  if (fd < 0) rp_fail(strerror(errno));
  struct stat st;
  if (fstat(fd, &st) != 0 || !S_ISREG(st.st_mode) || st.st_size < 112) {
    close(fd);
    rp_fail("not a recording (too small or not a regular file)");
  }
  g_rp_len = (size_t)st.st_size;
  uint8_t *snap = malloc(g_rp_len);
  if (!snap) {
    close(fd);
    rp_fail("out of memory loading the recording");
  }
  size_t got = 0;
  if (lseek(fd, 0, SEEK_SET) != 0) {
    close(fd);
    rp_fail("recording fd is not seekable");
  }
  while (got < g_rp_len) {
    ssize_t r = read(fd, snap + got, g_rp_len - got);
    if (r < 0 && errno == EINTR) continue;
    if (r <= 0) {
      close(fd);
      rp_fail("short read loading the recording");
    }
    got += (size_t)r;
  }
  close(fd);
  g_rp_base = snap;
  const obs_rec_hdr_t *h = (const obs_rec_hdr_t *)g_rp_base;
  if (h->magic != OBS_REC_MAGIC || h->maj != 0 || h->min < 3) {
    rp_fail("not a v0.3+ recording (magic/version mismatch)");
  }
  if (h->header_len != sizeof(obs_rec_hdr_t)) {
    rp_fail("unexpected header length");
  }
  size_t end = g_rp_len;
  uint64_t trailer_count = 0;
  if (end >= sizeof(obs_rec_hdr_t) + 16 &&
      *(const uint64_t *)(g_rp_base + end - 16) == OBS_REC_END) {
    trailer_count = *(const uint64_t *)(g_rp_base + end - 8);
    end -= 16;
  } else {
    /* GH #296 phase 5 (WAL durability): the drain appends whole
     * frames in stream order, so a crash-truncated file is EXACT
     * up to one torn frame at the tail — a usable prefix. Partial
     * history stays opt-in (the refusal was a deliberate
     * review-round posture): the flag is the operator saying "I
     * know the tape ends early." */
    const char *tol = getenv("LOTUS_REPLAY_ALLOW_TRUNCATED");
    if (!(tol && tol[0] == '1')) {
      rp_fail("no clean-finalize trailer — the recording is "
              "truncated (crashed writer?); pass --allow-truncated "
              "(LOTUS_REPLAY_ALLOW_TRUNCATED=1) to replay the "
              "recorded prefix");
    }
    g_rp_truncated = 1;
  }
  uint64_t parsed = 0;
  memset(rp_ring_consumer, 0, sizeof rp_ring_consumer);
  size_t off = h->header_len;
  while (off + 8 <= end) {
    uint32_t tag = *(const uint32_t *)(g_rp_base + off);
    uint32_t a = *(const uint32_t *)(g_rp_base + off + 4);
    parsed++;
    if (tag == OBS_REC_TAG_RING) {
      if (end - off < 24) {
        if (g_rp_truncated) { parsed--; break; }
        rp_fail("truncated ring record");
      }
      uint64_t w0 = *(const uint64_t *)(g_rp_base + off + 8);
      uint64_t w1 = *(const uint64_t *)(g_rp_base + off + 16);
      uint32_t ekind = (uint32_t)((w0 >> 20) & 0x1F);
      if (a & OBS_REC_PRIV_RING) {
        uint32_t pr = a & ~OBS_REC_PRIV_RING;
        if (pr >= 64) rp_fail("private ring index out of range");
        if (ekind == REC_EV_CONSUMER) {
          rp_ring_consumer[pr] = w1;
        } else if (ekind == REC_EV_CONSUME) {
          rp_consumer_t *c =
              rp_consumer_create(rp_ring_consumer[pr]);
          if (c) {
            if (c->consume_len == c->consume_cap) {
              c->consume_cap =
                  c->consume_cap ? c->consume_cap * 2 : 64;
              rp_consume_t *g = realloc(
                  c->consume,
                  c->consume_cap * sizeof(rp_consume_t));
              if (!g) rp_fail("out of memory");
              c->consume = g;
            }
            c->consume[c->consume_len].locus =
                (uint32_t)(w0 & 0xFFFFFu);
            c->consume[c->consume_len].msg = w1;
            c->consume_len++;
          }
        }
      } /* public-ring records: standard iris protocol, no
         * recorder meaning — nothing to index for replay */
      off += 24;
    } else if (tag == OBS_REC_TAG_PAYLOAD || tag == OBS_REC_TAG_JOURNAL
               || tag == OBS_REC_TAG_META) {
      if (end - off < 32) {
        if (g_rp_truncated) { parsed--; break; }
        rp_fail("truncated blob header");
      }
      uint64_t b = *(const uint64_t *)(g_rp_base + off + 8);
      uint64_t cfield = *(const uint64_t *)(g_rp_base + off + 16);
      uint64_t size = *(const uint64_t *)(g_rp_base + off + 24);
      const uint8_t *bytes = g_rp_base + off + 32;
      if (size > end - off - 32) {
        if (g_rp_truncated) { parsed--; break; }
        rp_fail("blob length out of range");
      }
      uint64_t padded = (size + 7) & ~7ull;
      if (padded < size || padded > end - off - 32) {
        if (g_rp_truncated) { parsed--; break; }
        rp_fail("blob padding out of range");
      }
      if (tag == OBS_REC_TAG_JOURNAL) {
        /* a = jkind; b = consumer; cfield bit 63 = value withheld
         * (redaction); bytes = [u32 args_len][args][result]. */
        if (a >= RP_MAX_JK) rp_fail("journal kind out of range");
        if (size < 4) rp_fail("journal entry too small");
        uint32_t args_len;
        memcpy(&args_len, bytes, 4);
        if ((uint64_t)args_len + 4 > size) {
          rp_fail("journal argument frame out of range");
        }
        rp_blob_ref_t e;
        e.args = bytes + 4;
        e.args_len = args_len;
        e.p = bytes + 4 + args_len;
        e.size = size - 4 - args_len;
        e.jkind = a;
        e.withheld = (cfield >> 63) & 1;
        /* per-kind result shape (review round 2, finding 4) */
        if (!e.withheld) {
          switch (a) {
            case JK_TIME_NOW: case JK_TIME_MONO_NS:
            case JK_RAND_NEXT_INT: case JK_ENV_VAR_EXISTS:
            case JK_ENV_ARGS_COUNT:
              if (e.size != 8) rp_fail("journal i64 wrong size");
              break;
            case JK_ENV_VAR: case JK_ENV_ARG:
              if (e.size == 0 || e.p[e.size - 1] != 0) {
                rp_fail("journal string not NUL-terminated");
              }
              break;
            default: break;
          }
        }
        rp_consumer_t *c = rp_consumer_create(b);
        if (c) rp_queue_push(&c->journal, e);
      } else if (tag == OBS_REC_TAG_PAYLOAD) {
        if (g_rp_payload_len == g_rp_payload_cap) {
          g_rp_payload_cap =
              g_rp_payload_cap ? g_rp_payload_cap * 2 : 64;
          rp_payload_t *g = realloc(
              g_rp_payloads,
              g_rp_payload_cap * sizeof(rp_payload_t));
          if (!g) rp_fail("out of memory");
          g_rp_payloads = g;
        }
        rp_payload_t *pl = &g_rp_payloads[g_rp_payload_len++];
        pl->pub_id = b;
        pl->topic = a;
        pl->flags = (uint32_t)cfield;
        pl->bytes.p = bytes;
        pl->bytes.size = size;
        pl->bytes.args = NULL;
        pl->bytes.args_len = 0;
        pl->bytes.jkind = 0;
        pl->bytes.withheld = 0;
        /* phase 5b: ingress payloads with verbatim wire bytes (bit
         * 0 set, bit 1 clear) are the injectable tape. */
        if ((cfield & 1) && !(cfield & 2)) {
          if (g_rp_ingress_len == g_rp_ingress_cap) {
            g_rp_ingress_cap = g_rp_ingress_cap ? g_rp_ingress_cap * 2 : 64;
            uint32_t *g = realloc(g_rp_ingress,
                                  g_rp_ingress_cap * sizeof(uint32_t));
            if (!g) rp_fail("out of memory");
            g_rp_ingress = g;
          }
          g_rp_ingress[g_rp_ingress_len++] = g_rp_payload_len - 1;
        }
      } else {
        /* META: subject-hash map feeds injection + reports; other
         * subtypes carry no replay state — validated + skipped. */
        if (b == OBS_REC_META_SUBJHASH && size > 0 &&
            bytes[size - 1] == 0) {
          if (g_rp_subject_len == g_rp_subject_cap) {
            g_rp_subject_cap =
                g_rp_subject_cap ? g_rp_subject_cap * 2 : 32;
            rp_subj_t *g = realloc(
                g_rp_subjects, g_rp_subject_cap * sizeof(rp_subj_t));
            if (!g) rp_fail("out of memory");
            g_rp_subjects = g;
          }
          g_rp_subjects[g_rp_subject_len].hash = a;
          g_rp_subjects[g_rp_subject_len].name =
              (const char *)bytes;
          g_rp_subject_len++;
        }
      }
      off += 32 + padded;
    } else {
      rp_fail("unknown entry tag — recording from a newer hale?");
    }
  }
  if (!g_rp_truncated) {
    if (off != end) rp_fail("entries do not end at the trailer");
    if (parsed != trailer_count) {
      rp_fail("trailer count disagrees with parsed entries");
    }
  } else {
    g_rp_dropped_tail = end - off;
    fprintf(stderr,
            "hale: LOTUS_REPLAY: no clean finalize — replaying the "
            "recorded prefix (%llu entries; %llu torn tail byte(s) "
            "dropped)\n",
            (unsigned long long)parsed,
            (unsigned long long)g_rp_dropped_tail);
  }
}

/* ---- phase 5b: the ingress tape (injection + feed) -------------
 *
 * The reader threads' wire captures (verbatim bytes, ingress flag)
 * become the injectable tape. The arena-side injector walks it in
 * artifact order through these accessors; identity rides
 * t_rec_forced_pub so an injected delivery matches its recorded
 * consume events without minting a new pub_id. */
uint64_t lotus_replay_ingress_count(void) {
  return (lotus_replay_active || lotus_replay_feed)
             ? g_rp_ingress_len
             : 0;
}
int lotus_replay_ingress_get(uint64_t i, uint32_t *topic,
                             uint64_t *pub_id, const void **bytes,
                             uint64_t *size) {
  if (i >= g_rp_ingress_len) return 0;
  const rp_payload_t *pl = &g_rp_payloads[g_rp_ingress[i]];
  *topic = pl->topic;
  *pub_id = pl->pub_id;
  *bytes = pl->bytes.p;
  *size = pl->bytes.size;
  return 1;
}
const char *lotus_replay_topic_name(uint32_t topic) {
  for (uint32_t i = 0; i < g_rp_subject_len; i++) {
    if (g_rp_subjects[i].hash == topic) return g_rp_subjects[i].name;
  }
  return NULL;
}
uint32_t lotus_replay_subject_hash(const char *subject) {
  return obs_subject_hash(subject);
}
/* Called by the injector immediately before dispatching one tape
 * payload. Strict replay: pin the RECORDED identity (and, when a
 * verify recording is running under --diff, record the injected
 * payload so the comparator can pair it). Feed: identity is the
 * new run's own — nothing to pin. */
uint64_t lotus_replay_inject_begin(const char *subject,
                                   const void *bytes, uint64_t size,
                                   uint64_t pub_id) {
  if (!lotus_replay_active) return 0;
  if (lotus_obs_recording && subject) {
    obs_rec_blob_push(OBS_REC_TAG_PAYLOAD, obs_subject_hash(subject),
                      pub_id, 1u, bytes, size);
  }
  t_rec_forced_pub = pub_id;
  return pub_id;
}
void lotus_replay_note_injected(void) {
  atomic_fetch_add_explicit(&g_rp_injected, 1, memory_order_relaxed);
}
void lotus_replay_note_inject_dropped(uint32_t topic) {
  (void)topic;
  atomic_fetch_add_explicit(&g_rp_inject_dropped, 1,
                            memory_order_relaxed);
}
void lotus_replay_note_binding_suppressed(void) {
  atomic_fetch_add_explicit(&g_rp_bindings_suppressed, 1,
                            memory_order_relaxed);
}

/* Live-side twin of inject_begin, called by the reader threads
 * before they deserialize received wire bytes: capture the VERBATIM
 * wire form with the ingress flag (the injectable shape — the
 * struct-bytes record local_dispatch would otherwise push is
 * metadata-only and cannot be re-fed), derive the pub_id here, and
 * pin it so the dispatch a few lines later reuses this identity. */
uint64_t lotus_obs_record_ingress_wire(const char *subject,
                                       const void *bytes,
                                       uint64_t size) {
  if ((!lotus_obs_recording && !lotus_replay_active) || !subject)
    return 0;
  if (!obs_on()) return 0;
  if (t_consumer_id == 0) obs_rec_claim();
  uint64_t cid = t_consumer_id;
  if (cid > OBS_REC_CONSUMER_MAX || t_pub_seq >= 0xFFFFFFFFFFFFULL) {
    fprintf(stderr,
            "hale: recording identity out of range (consumer %llu, "
            "seq %llu) — refusing to record ambiguously\n",
            (unsigned long long)cid,
            (unsigned long long)t_pub_seq);
    fflush(NULL);
    _exit(70);
  }
  uint64_t pub_id = (cid << 48) | (++t_pub_seq & 0xFFFFFFFFFFFFULL);
  if (lotus_obs_recording) {
    obs_rec_blob_push(OBS_REC_TAG_PAYLOAD, obs_subject_hash(subject),
                      pub_id, 1u, bytes, size);
  }
  t_rec_forced_pub = pub_id;
  return pub_id;
}

/* Feed-mode exit report — dropped tape entries are the headline
 * fact: silence would read as "everything was fed." */
static void rp_feed_report(void) {
  uint64_t inj = atomic_load(&g_rp_injected);
  uint64_t drop = atomic_load(&g_rp_inject_dropped);
  fprintf(stderr,
          "hale feed: %llu of %u recorded ingress payload(s) "
          "injected%s\n",
          (unsigned long long)inj, g_rp_ingress_len,
          drop ? "" : "; all matched");
  if (drop) {
    fprintf(stderr,
            "hale feed: %llu dropped — no matching subscribed "
            "subject in this program\n",
            (unsigned long long)drop);
  }
}

/* The next recorded input for this consumer, iff it matches the
 * caller's kind AND its EXACT encoded arguments (memcmp — hashes
 * were collision-prone: value|1 folded adjacent integers; review
 * round 2, finding 3) and, when nonzero, the result size. Any
 * mismatch — including a withheld (redacted) value — is a named
 * divergence, and a mismatched entry is NOT consumed. */
static const rp_blob_ref_t *rp_serve(uint32_t jkind, const void *args,
                                     uint64_t args_len,
                                     uint64_t want_size) {
  if (!lotus_replay_active || jkind >= RP_MAX_JK) return NULL;
  rp_consumer_t *c = rp_consumer(t_consumer_id);
  if (!c) {
    atomic_fetch_add_explicit(&g_rp_divergences[jkind], 1,
                              memory_order_relaxed);
    return NULL;
  }
  rp_queue_t *q = &c->journal;
  if (q->cursor >= q->len) {
    atomic_fetch_add_explicit(&g_rp_divergences[jkind], 1,
                              memory_order_relaxed);
    return NULL;
  }
  const rp_blob_ref_t *e = &q->items[q->cursor];
  if (e->jkind != jkind || e->args_len != args_len ||
      (args_len && memcmp(e->args, args, args_len) != 0) ||
      e->withheld || (want_size && e->size != want_size)) {
    atomic_fetch_add_explicit(&g_rp_divergences[jkind], 1,
                              memory_order_relaxed);
    if (!e->withheld) return NULL;
    /* A withheld value still ADVANCES the stream (its identity
     * matched the recording's shape; only the value is absent) so
     * subsequent reads stay aligned. */
    if (e->jkind == jkind && e->args_len == args_len &&
        (!args_len || memcmp(e->args, args, args_len) == 0)) {
      q->cursor++;
    }
    return NULL;
  }
  q->cursor++;
  return e;
}

int lotus_replay_serve_i64(uint32_t jkind, const void *args,
                           uint64_t args_len, int64_t *out) {
  const rp_blob_ref_t *e = rp_serve(jkind, args, args_len, 8);
  if (!e) return 0;
  memcpy(out, e->p, 8);
  return 1;
}

const void *lotus_replay_serve_blob(uint32_t jkind, const void *args,
                                    uint64_t args_len,
                                    uint64_t want_size,
                                    uint64_t *out_n) {
  const rp_blob_ref_t *e = rp_serve(jkind, args, args_len, want_size);
  if (!e) return NULL;
  *out_n = e->size;
  return e->p;
}

/* Phase 4: the calling thread's next expected delivery identity.
 * Returns 1 with *out = pub_id (0 = a structural cell) when the
 * recorded stream has one left; 0 = stream exhausted (no
 * constraint — consume freely). */
int lotus_replay_expected_consume(uint64_t *out_msg,
                                  uint32_t *out_locus) {
  if (!lotus_replay_active) return 0;
  rp_consumer_t *c = rp_consumer(t_consumer_id);
  if (!c || c->consume_cursor >= c->consume_len) return 0;
  *out_msg = c->consume[c->consume_cursor].msg;
  *out_locus = c->consume[c->consume_cursor].locus;
  return 1;
}

/* Subscriber instance id for the enforcement gate (delivery
 * identity = target locus + message id). */
uint64_t lotus_obs_pub_inst_id(void *self) {
  if (!self || !obs_on()) return 0;
  return obs_inst_id_of(self);
}

/* Called at every dequeue-driven handler invoke under replay:
 * advances this consumer's recorded stream and drives
 * LOTUS_REPLAY_AT (stop at the Nth consume process-wide, SIGSTOP
 * so a debugger can attach). */
void lotus_replay_note_consume(void) {
  if (!lotus_replay_active) return;
  rp_consumer_t *c = rp_consumer(t_consumer_id);
  if (c && c->consume_cursor < c->consume_len) c->consume_cursor++;
  uint64_t n = atomic_fetch_add_explicit(&g_rp_consume_count, 1,
                                         memory_order_relaxed) + 1;
  /* consumer:N form — stable across multi-consumer runs, unlike
   * the process-wide ordinal (review, --at stability note). */
  if (g_replay_at_consumer && g_replay_at && c &&
      t_consumer_id == g_replay_at_consumer &&
      (uint64_t)c->consume_cursor == g_replay_at) {
    fprintf(stderr,
            "hale replay: stopped at consumer %llu consume #%llu "
            "(pid %d) — attach a debugger, then SIGCONT\n",
            (unsigned long long)t_consumer_id,
            (unsigned long long)g_replay_at, (int)getpid());
    fflush(NULL);
    raise(SIGSTOP);
    return;
  }
  if (!g_replay_at_consumer && g_replay_at && n == g_replay_at) {
    fprintf(stderr,
            "hale replay: stopped at consume #%llu (pid %d) — "
            "attach a debugger, then SIGCONT\n",
            (unsigned long long)n, (int)getpid());
    fflush(NULL);
    raise(SIGSTOP);
  }
}

/* Teardown summary: replay honesty is the divergence report. */
uint64_t lotus_replay_order_divergences(void) __attribute__((weak));

static void rp_report(void) {
  if (!lotus_replay_active) return;
  uint64_t unconsumed_journal = 0, unconsumed_deliveries = 0;
  for (int i = 0; i < g_rp_consumer_count; i++) {
    rp_consumer_t *c = &g_rp_consumers[i];
    unconsumed_journal += c->journal.len - c->journal.cursor;
    unconsumed_deliveries += c->consume_len - c->consume_cursor;
  }
  uint64_t order_div = lotus_replay_order_divergences
      ? lotus_replay_order_divergences()
      : 0;
  uint64_t unexpected =
      atomic_load_explicit(&g_rp_unexpected, memory_order_relaxed);
  const char *status_path = getenv("LOTUS_REPLAY_STATUS");
  if (status_path && status_path[0]) {
    FILE *sf = fopen(status_path, "w");
    if (sf) {
      uint64_t jd = 0;
      for (int i = 0; i < RP_MAX_JK; i++)
        jd += atomic_load(&g_rp_divergences[i]);
      fprintf(sf,
              "journal_divergences=%llu\norder_divergences=%llu\n"
              "unconsumed_journal=%llu\nunconsumed_deliveries=%llu\n"
              "unexpected_deliveries=%llu\nconsumes=%llu\n",
              (unsigned long long)jd, (unsigned long long)order_div,
              (unsigned long long)unconsumed_journal,
              (unsigned long long)unconsumed_deliveries,
              (unsigned long long)unexpected,
              (unsigned long long)atomic_load(&g_rp_consume_count));
      fclose(sf);
    }
  }
  static const char *jk_names[RP_MAX_JK] = {
    "?", "time.now", "time.monotonic", "rand.next_int",
    "os.getrandom", "env.var", "env.var_exists", "env.arg",
    "env.args_count" };
  uint64_t total = order_div + unconsumed_journal
      + unconsumed_deliveries + unexpected;
  for (int i = 0; i < RP_MAX_JK; i++)
    total += atomic_load(&g_rp_divergences[i]);
  if (g_rp_truncated) {
    fprintf(stderr,
            "hale replay: note — truncated recording; coverage ends "
            "at the torn tail\n");
  }
  uint64_t sup = atomic_load(&g_rp_bindings_suppressed);
  if (sup) {
    fprintf(stderr,
            "hale replay: hermetic wire — %llu binding(s) "
            "suppressed; %llu of %u recorded ingress payload(s) "
            "injected (%llu dropped)\n",
            (unsigned long long)sup,
            (unsigned long long)atomic_load(&g_rp_injected),
            g_rp_ingress_len,
            (unsigned long long)atomic_load(&g_rp_inject_dropped));
  }
  if (total == 0) {
    fprintf(stderr,
            "hale replay: journal served fully — 0 divergences, "
            "%llu consumes\n",
            (unsigned long long)atomic_load(&g_rp_consume_count));
    return;
  }
  fprintf(stderr,
          "hale replay: %llu divergences from the recorded "
          "history:\n",
          (unsigned long long)total);
  for (int i = 0; i < RP_MAX_JK; i++) {
    uint64_t d = atomic_load(&g_rp_divergences[i]);
    if (d) fprintf(stderr, "  %-22s %llu\n", jk_names[i],
                   (unsigned long long)d);
  }
  if (order_div)
    fprintf(stderr, "  %-22s %llu\n", "delivery-order",
            (unsigned long long)order_div);
  if (unconsumed_journal)
    fprintf(stderr, "  %-22s %llu\n", "unconsumed-journal",
            (unsigned long long)unconsumed_journal);
  if (unconsumed_deliveries)
    fprintf(stderr, "  %-22s %llu\n", "unconsumed-deliveries",
            (unsigned long long)unconsumed_deliveries);
  if (unexpected)
    fprintf(stderr, "  %-22s %llu\n", "unexpected-deliveries",
            (unsigned long long)unexpected);
}

/* Pinned-locus threads: identity = 64 + the locus's obs instance
 * id (registered by the SPAWNING thread before the pinned thread
 * runs — handoff-2 ordering), deterministic when the spawn order
 * is. Called unconditionally from the synthesized __pinned_main
 * prologue; thread start is a cold path and this no-ops when
 * observation is off. */
void lotus_obs_note_consumer_locus(void *self);

static uint64_t obs_mono_ns(void) {
  struct timespec ts; clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}
static uint64_t obs_wall_ns(void) {
  struct timespec ts; clock_gettime(CLOCK_REALTIME, &ts);
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}
static size_t obs_page_up(size_t n) {
  return (n + OBS_PAGE - 1) & ~((size_t)OBS_PAGE - 1);
}

void lotus_obs_teardown(void) {
  if (!g_seg) return;
  /* GH #296: finalize the recording BEFORE the lock and the unmap —
   * the drain reads the segment, and a producer blocked on a full
   * ring is waiting for the drain's cursor. Setting g_rec_stop
   * releases blocked producers (their wait loop checks it); the
   * drain does one final full sweep after observing the flag, so
   * every record published before this point reaches the file. The
   * trailer is the clean-finalize marker: a reader that doesn't
   * find it holds a truncated recording and must say so. */
  if (g_rec_thread_started) {
    atomic_store(&g_rec_stop, 1);
    pthread_join(g_rec_thread, NULL);
    g_rec_thread_started = 0;
    if (g_rec_file) {
      /* Re-stamp the identity fields at finalize: the header is
       * first written at segment creation, which the FIRST PROBE
       * triggers — and a prelude registration (a topic binding)
       * can probe before the model/exec stamps run, snapshotting
       * zeros. Finalize-time values are the authoritative ones. */
      uint64_t ident[6] = { g_obs_model_hash,
                            g_obs_exec_digest[0], g_obs_exec_digest[1],
                            g_obs_exec_digest[2], g_obs_exec_digest[3],
                            g_rec_env_full ? 0u : 1u };
      if (fseek(g_rec_file, 48, SEEK_SET) != 0 ||
          fwrite(ident, sizeof ident, 1, g_rec_file) != 1 ||
          fseek(g_rec_file, 0, SEEK_END) != 0) {
        g_rec_file = NULL;
        fprintf(stderr,
                "hale: LOTUS_OBS_RECORD identity re-stamp failed\n");
        fflush(NULL);
        _exit(74);
      }
      uint64_t trailer[2] = { OBS_REC_END, g_rec_written };
      if (fwrite(trailer, sizeof trailer, 1, g_rec_file) != 1 ||
          fflush(g_rec_file) != 0 || fclose(g_rec_file) != 0) {
        g_rec_file = NULL;
        fprintf(stderr,
                "hale: LOTUS_OBS_RECORD finalize failed: %s — "
                "failing the run (an unfinalized recording must "
                "not pass silently)\n",
                strerror(errno));
        fflush(NULL);
        _exit(74);
      }
      g_rec_file = NULL;
    }
    for (int r = 0; r < 64; r++) {
      free(g_rec_rings[r].slots);
      g_rec_rings[r].slots = NULL;
    }
  }
  /* Serialize against the P18 heartbeat: it probes the segment
   * under g_obs_lock, so taking the lock here means the unmap
   * never races a heartbeat mid-deref. */
  pthread_mutex_lock(&g_obs_lock);
  if (!g_seg) {
    pthread_mutex_unlock(&g_obs_lock);
    return;
  }
  atomic_store(&H->flags, 0);
  unlink(g_reg_path);
  munmap(g_seg, g_seg_len);
  shm_unlink(g_shm_name);
  g_seg = NULL;
  atomic_store(&g_obs_state, 1);
  pthread_mutex_unlock(&g_obs_lock);
}

/* iris handoff-5 P18: the observer-attach birth replay was driven
 * lazily from INSIDE probes (obs_gate's 0→1 edge detection) — so a
 * process whose steady state emits no probes (a quiet main parked
 * in a read loop with pinned raw-fd readers; or hot paths on the
 * fully-devirtualized direct dispatch) never noticed the observer
 * and never replayed its live loci: segment registered, zero
 * records, "silent" to the consumer. This thread exists ONLY when
 * LOTUS_OBS=1 and does one obs_gate() probe per 250ms under
 * g_obs_lock — the replay latency after attach is ≤250ms even if
 * no probe ever fires again. Detached; exits with the process.
 * It claims one SPSC ring slot for its replay emissions (TLS ring
 * assignment), which the default of 8 rings accommodates. */
static int obs_gate(void);
static void *obs_record_drain_main(void *arg);
static void *obs_heartbeat_main(void *arg) {
  (void)arg;
  for (;;) {
    struct timespec ts = {0, 250 * 1000 * 1000};
    nanosleep(&ts, NULL);
    pthread_mutex_lock(&g_obs_lock);
    if (g_seg && atomic_load(&g_obs_state) == 2) {
      obs_gate();
    }
    pthread_mutex_unlock(&g_obs_lock);
  }
  return NULL;
}


/* ---- stale-segment sweep ---------------------------------------
 *
 * Clean exit unlinks the segment and its registration via atexit.
 * A SIGKILLed process cannot: by definition it runs no handler. The
 * segments are ~570 KB each, so a fleet stopped with `docker stop`
 * (which never reaches dissolve) accumulates them on the host —
 * A downstream fleet measured 442 stale segments, 245 MB of tmpfs,
 * from one run.
 *
 * PROTOCOL §1 already says consumers may GC a stale REGISTRATION by
 * pid liveness; that covers the small JSON file, not the segment.
 * A dead emitter can't clean up after itself, so someone else must,
 * and the cheapest reliable "someone else" is the next process to
 * start: it is already paying init cost, it has the same uid, and a
 * restarting fleet sweeps itself.
 *
 * Conservative on purpose. A pid that is ALIVE is skipped even
 * though it might be a recycled pid whose segment is stale —
 * unlinking a live process's segment would blind its observer,
 * which is far worse than leaving one file behind. Failures are
 * ignored throughout: another process sweeping concurrently, or a
 * segment owned by a different uid, are both fine.
 */
static void obs_sweep_stale(void) {
  DIR *d = opendir("/dev/shm");
  if (d) {
    struct dirent *e;
    int self = (int)getpid();
    while ((e = readdir(d)) != NULL) {
      int pid = 0;
      if (sscanf(e->d_name, "hale-obs-%d", &pid) != 1) continue;
      if (pid <= 0 || pid == self) continue;
      /* ESRCH => no such process. EPERM => alive, other uid: skip. */
      if (kill((pid_t)pid, 0) == 0 || errno != ESRCH) continue;
      char nm[64];
      snprintf(nm, sizeof nm, "/hale-obs-%d", pid);
      shm_unlink(nm);
    }
    closedir(d);
  }
  /* The registration files alongside them. */
  const char *xdg = getenv("XDG_RUNTIME_DIR");
  char dir[192];
  if (xdg) snprintf(dir, sizeof dir, "%s/hale", xdg);
  else snprintf(dir, sizeof dir, "/tmp/hale-obs");
  DIR *rd = opendir(dir);
  if (!rd) return;
  struct dirent *e;
  int self = (int)getpid();
  while ((e = readdir(rd)) != NULL) {
    int pid = 0;
    if (sscanf(e->d_name, "%d.json", &pid) != 1) continue;
    if (pid <= 0 || pid == self) continue;
    if (kill((pid_t)pid, 0) == 0 || errno != ESRCH) continue;
    char path[280];
    snprintf(path, sizeof path, "%s/%d.json", dir, pid);
    unlink(path);
  }
  closedir(rd);
}


static int obs_create(int64_t rings, int64_t slots) {
  size_t manifest_len =
      obs_page_up(sizeof(obs_mh_t) + OBS_ENTRY_CAP * 32 + 8192);
  size_t modemask_len = obs_page_up(OBS_ENTRY_CAP);
  size_t counters_len = obs_page_up((1 + OBS_ENTRY_CAP) * 64);
  size_t rings_hdr = obs_page_up((size_t)rings * sizeof(obs_rdesc_t));
  size_t ring_bytes = (size_t)slots * 16;
  size_t off = 0;
  size_t control_off  = (off += OBS_PAGE);
  size_t manifest_off = (off += OBS_PAGE);
  size_t modemask_off = (off += manifest_len);
  size_t counters_off = (off += modemask_len);
  size_t rings_off    = (off += counters_len);
  g_seg_len = rings_off + rings_hdr + (size_t)rings * ring_bytes;

  obs_sweep_stale();
  snprintf(g_shm_name, sizeof g_shm_name, "/hale-obs-%d", (int)getpid());
  shm_unlink(g_shm_name);
  int fd = shm_open(g_shm_name, O_CREAT | O_EXCL | O_RDWR, 0600);
  if (fd < 0) return 0;
  if (ftruncate(fd, (off_t)g_seg_len) < 0) { close(fd); return 0; }
  g_seg = mmap(NULL, g_seg_len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  close(fd);
  if (g_seg == MAP_FAILED) { g_seg = NULL; return 0; }

  H = (obs_hdr_t *)g_seg;
  C = (obs_ctrl_t *)((char *)g_seg + control_off);
  MH = (obs_mh_t *)((char *)g_seg + manifest_off);
  ME = (obs_me_t *)((char *)MH + sizeof(obs_mh_t));
  MH->entry_cap = OBS_ENTRY_CAP;
  MH->pool_off = (uint32_t)(sizeof(obs_mh_t) + OBS_ENTRY_CAP * 32);
  POOL = (char *)MH + MH->pool_off;
  MODE = (uint8_t *)g_seg + modemask_off;
  CNT = (obs_cline_t *)((char *)g_seg + counters_off);
  RD = (obs_rdesc_t *)((char *)g_seg + rings_off);
  memset(MODE, 2 /* PACKED */, OBS_ENTRY_CAP);
  memset(g_cnt_line_for, -1, sizeof g_cnt_line_for);

  *H = (obs_hdr_t){ .magic = OBS_MAGIC, .proto_major = 0, .proto_minor = 2,
    .header_len = sizeof(obs_hdr_t), .total_len = g_seg_len,
    .pid = (uint32_t)getpid(), .ring_count = (uint32_t)rings,
    .ring_slots = (uint32_t)slots, .ts_shift = 4,
    .started_mono_ns = obs_mono_ns(), .started_wall_ns = obs_wall_ns(),
    .control_off = control_off, .manifest_off = manifest_off,
    .manifest_len = manifest_len, .modemask_off = modemask_off,
    .counters_off = counters_off, .counters_len = counters_len,
    .rings_off = rings_off, .model_hash = g_obs_model_hash };
  for (int64_t i = 0; i < rings; i++)
    RD[i] = (obs_rdesc_t){
        .data_off = rings_off + rings_hdr + (uint64_t)i * ring_bytes,
        .tag_a = (uint32_t)i };
  atomic_store(&H->flags, 1);

  const char *xdg = getenv("XDG_RUNTIME_DIR");
  char dir[192];
  if (xdg) snprintf(dir, sizeof dir, "%s/hale", xdg);
  else snprintf(dir, sizeof dir, "/tmp/hale-obs");
  mkdir(dir, 0700);
  snprintf(g_reg_path, sizeof g_reg_path, "%s/%d.json", dir,
           (int)getpid());
  char tmp[300]; snprintf(tmp, sizeof tmp, "%s.tmp", g_reg_path);
  FILE *f = fopen(tmp, "w");
  if (f) {
    char exe[128] = "hale-app";
    ssize_t n = readlink("/proc/self/exe", exe, sizeof exe - 1);
    if (n > 0) exe[n] = 0;
    fprintf(f,
      "{\n  \"proto\": \"0.1\",\n  \"pid\": %d,\n  \"exe\": \"%s\",\n"
      "  \"shm\": \"%s\",\n  \"started_mono_ns\": %llu,\n"
      "  \"started_wall_ns\": %llu,\n  \"rings\": %d\n}\n",
      (int)getpid(), exe, g_shm_name,
      (unsigned long long)H->started_mono_ns,
      (unsigned long long)H->started_wall_ns, (int)rings);
    fclose(f);
    rename(tmp, g_reg_path);
  }
  {
    uint32_t pid = (uint32_t)getpid();
    uint16_t o = (uint16_t)((pid ^ (pid >> 16)) & 0xFFFFu);
    g_obs_origin = o ? o : (uint16_t)0xFFFFu;
  }
  atexit(lotus_obs_teardown);
  /* GH #296: recording setup, fail-closed at every step. The file
   * opens (and its header lands) before any record can be emitted;
   * the drain thread is joinable so teardown can finalize. The
   * recording counts as an attached observer — ring emission is on
   * from the first probe without anyone mmapping the segment. */
  if (lotus_obs_recording) {
    unlink(g_rec_path);
    int rec_fd = open(g_rec_path,
                      O_CREAT | O_EXCL | O_WRONLY | O_NOFOLLOW, 0600);
    g_rec_file = rec_fd >= 0 ? fdopen(rec_fd, "wb") : NULL;
    if (!g_rec_file) {
      fprintf(stderr,
              "hale: LOTUS_OBS_RECORD could not open `%s`: %s\n",
              g_rec_path, strerror(errno));
      fflush(NULL);
      _exit(74);
    }
    setvbuf(g_rec_file, NULL, _IOFBF, 1 << 20);
    obs_rec_hdr_t rh = { .magic = OBS_REC_MAGIC, .maj = 0, .min = 3,
      .header_len = sizeof(obs_rec_hdr_t), .pid = (uint32_t)getpid(),
      .ring_count = (uint32_t)rings, .ring_slots = (uint32_t)slots,
      .ts_shift = 4, .started_mono_ns = H->started_mono_ns,
      .started_wall_ns = H->started_wall_ns,
      .model_hash = g_obs_model_hash,
      .exec_digest = { g_obs_exec_digest[0], g_obs_exec_digest[1],
                       g_obs_exec_digest[2], g_obs_exec_digest[3] },
      .flags = g_rec_env_full ? 0 : 1 };
    if (fwrite(&rh, sizeof rh, 1, g_rec_file) != 1) {
      fprintf(stderr, "hale: LOTUS_OBS_RECORD header write failed\n");
      fflush(NULL);
      _exit(74);
    }
    atomic_store(&C->observer_count, 1u);
    /* Pre-arm the 0→1 edge detector: the recording observes from
     * record zero, so there are no pre-attach births to replay —
     * and without this, the FIRST birth probe (which registers
     * itself, then gates) would trip the edge and re-emit itself:
     * a duplicate LOCUS_BIRTH as the recording's opening record. */
    atomic_store_explicit(&g_last_observed, 1u, memory_order_relaxed);
    if (pthread_create(&g_rec_thread, NULL, obs_record_drain_main,
                       NULL) != 0) {
      fprintf(stderr,
              "hale: LOTUS_OBS_RECORD drain thread failed to start\n");
      fflush(NULL);
      _exit(74);
    }
    g_rec_thread_started = 1;
  }
  /* P18: spawn the replay heartbeat (detached; see
   * obs_heartbeat_main). Best-effort — a failed spawn degrades to
   * the old probe-driven replay, never blocks obs creation. */
  {
    pthread_t hb;
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED);
    (void)pthread_create(&hb, &attr, obs_heartbeat_main, NULL);
    pthread_attr_destroy(&attr);
  }
  return 1;
}

/* Dormant-cost gate for the codegen-emitted publisher-attribution
 * note (bench regression, v0.11.13→ found 2026-07-28). lower_send
 * used to emit an UNCONDITIONAL `lotus_obs_note_publisher` call
 * before every `<-` — a call + TLS store even with LOTUS_OBS
 * unset, ~1ns on a ~1.7ns devirtualized publish (+55% on the
 * bus_dispatch microbench). Codegen now loads this flag and only
 * calls when observation is live, restoring the "dormant = one
 * predictable branch" cost contract. Written once, under
 * g_obs_lock, when the env gate resolves; plain int is fine (the
 * transition happens before any publish that could care, and a
 * momentarily-stale 0 only skips attribution on the first
 * publishes of a not-yet-probed thread). */
int lotus_obs_live = 0;

/* iris handoff-6 P19: codegen snapshots this flag at FUNCTION ENTRY
 * (the dormant-cost hoist), so it must be final before ANY user code
 * — including `fn main`'s own body, whose entry block runs BEFORE
 * the first locus-birth probe that used to set it. A publish lowered
 * into main's body therefore snapshotted 0 forever. Resolve the env
 * in a constructor instead: the flag is process-constant from before
 * main, which is the exact property the hoist's soundness argument
 * claimed. (Segment creation stays lazy at the first probe; if it
 * later fails, the flag stays 1 and the probes early-out on
 * !obs_on() — a few dead branches in a broken-obs process.) */
__attribute__((constructor)) static void obs_live_ctor(void) {
  const char *e = getenv("LOTUS_OBS");
  if (e && e[0] == '1') lotus_obs_live = 1;
  /* GH #296: LOTUS_OBS_RECORD implies observation — the recording
   * IS an observer. Resolved here for the same P19 reason as
   * lotus_obs_live: both flags must be process-constant before any
   * user code (including fn main's body) can snapshot them. */
  /* The constructor runs on the main thread: stamp its stable
   * consumer identity here so the first ring it claims carries it. */
  t_consumer_id = 1;
  /* GH #296: replay. Implies observation (identity machinery rides
   * it); loads the recording eagerly so the first read is served. */
  /* GH #296 phase 5b: feed mode. Mutually exclusive with strict
   * replay — one recording cannot be both the law and merely the
   * input tape in a single run. */
  const char *feed = getenv("LOTUS_REPLAY_FEED");
  const char *rp = getenv("LOTUS_REPLAY");
  if (feed && feed[0] && rp && rp[0]) {
    fprintf(stderr,
            "hale: LOTUS_REPLAY and LOTUS_REPLAY_FEED are mutually "
            "exclusive\n");
    _exit(64);
  }
  if (feed && feed[0]) {
    size_t n = strlen(feed);
    if (n >= sizeof g_replay_path) {
      fprintf(stderr, "hale: LOTUS_REPLAY_FEED path too long\n");
      _exit(64);
    }
    memcpy(g_replay_path, feed, n + 1);
    rp_load();
    lotus_replay_feed = 1;
    atexit(rp_feed_report);
  }
  if (rp && rp[0]) {
    size_t n = strlen(rp);
    if (n >= sizeof g_replay_path) {
      fprintf(stderr, "hale: LOTUS_REPLAY path too long\n");
      _exit(64);
    }
    memcpy(g_replay_path, rp, n + 1);
    const char *at = getenv("LOTUS_REPLAY_AT");
    if (at && at[0]) g_replay_at = (uint64_t)atoll(at);
    const char *atc = getenv("LOTUS_REPLAY_AT_CONSUMER");
    if (atc && atc[0]) g_replay_at_consumer = (uint64_t)atoll(atc);
    rp_load();
    lotus_replay_active = 1;
    lotus_obs_live = 1;
    atexit(rp_report);
  }
  const char *envmode = getenv("LOTUS_OBS_RECORD_ENV");
  if (envmode && strcmp(envmode, "full") == 0) g_rec_env_full = 1;
  const char *rec = getenv("LOTUS_OBS_RECORD");
  if (rec && rec[0]) {
    size_t n = strlen(rec);
    if (n < sizeof g_rec_path) {
      memcpy(g_rec_path, rec, n + 1);
      lotus_obs_recording = 1;
      lotus_obs_live = 1;
    } else {
      fprintf(stderr,
              "hale: LOTUS_OBS_RECORD path exceeds %zu bytes; "
              "recording refused\n",
              sizeof g_rec_path - 1);
      _exit(64);
    }
  }
  /* GH #296 phase 5: durability grade. Default recording rides the
   * page cache (survives a process crash, not power loss);
   * DURABLE=1 pushes every flushed drain sweep through fdatasync. */
  const char *dur = getenv("LOTUS_OBS_RECORD_DURABLE");
  if (dur && dur[0] == '1') g_rec_durable = 1;
}

/* Round 4/5: recording must initialize EAGERLY (a probe-free
 * program must still produce a recording, replacing anything at
 * the path) — but NOT from the C constructor: segment creation
 * snapshots the model identity into the shared header, and at
 * constructor time the prelude has not stamped it yet, so a
 * ctor-driven init published a live segment claiming to be
 * unstamped for its whole life (round 5 — regressing the
 * obs_model_hash contract iris reads). The generated prelude calls
 * this immediately AFTER the identity setters: still before any
 * user code or probe, so probe-free programs are covered and the
 * segment is born with its immutable header complete. The .halerec
 * artifact additionally re-stamps at finalize. */
void lotus_obs_eager_init(void) {
  if (lotus_obs_recording || lotus_replay_active) {
    (void)obs_on();
  }
}

/* Enable check + lazy init. Fast path after first call: one
 * relaxed load + compare. */
static int obs_on(void) {
  int st = atomic_load_explicit(&g_obs_state, memory_order_relaxed);
  if (st == 2) return 1;
  if (st == 1) return 0;
  pthread_mutex_lock(&g_obs_lock);
  st = atomic_load(&g_obs_state);
  if (st == 0) {
    const char *e = getenv("LOTUS_OBS");
    if ((e && e[0] == '1') || lotus_obs_recording ||
        lotus_replay_active) {
      const char *rs = getenv("LOTUS_OBS_RINGS");
      const char *ss = getenv("LOTUS_OBS_SLOTS");
      /* Recording defaults to the ring maximum: a thread that
       * cannot get a ring fails a recorded run (never drops), so
       * make that as rare as the 64-ring ceiling allows. */
      int64_t rings = rs ? atoll(rs) : (lotus_obs_recording ? 64 : 8);
      int64_t slots = ss ? atoll(ss) : 4096;
      if (rings < 1 || rings > 64) rings = lotus_obs_recording ? 64 : 8;
      if (slots < 64 || (slots & (slots - 1))) slots = 4096;
      st = obs_create(rings, slots) ? 2 : 1;
      if (st != 2 && lotus_obs_recording) {
        fprintf(stderr,
                "hale: LOTUS_OBS_RECORD was set but the observation "
                "segment could not be created; failing the run "
                "rather than running unrecorded\n");
        fflush(NULL);
        _exit(74);
      }
    } else {
      st = 1;
    }
    if (st == 2) lotus_obs_live = 1;
    atomic_store(&g_obs_state, st);
  }
  pthread_mutex_unlock(&g_obs_lock);
  return st == 2;
}

static uint64_t obs_fnv(const char *a, const char *b) {
  uint64_t h = 0xcbf29ce484222325ull;
  for (const char *p = a; *p; p++) { h ^= (uint8_t)*p; h *= 0x100000001b3ull; }
  h ^= ':'; h *= 0x100000001b3ull;
  for (const char *p = b; p && *p; p++) { h ^= (uint8_t)*p; h *= 0x100000001b3ull; }
  return h;
}

/* iris handoff-2: binding ids start at 1 here (reference glue
 * started them at 0, which collided with the arena-side
 * `obs_binding_id == 0` = unregistered sentinel and produced
 * records the consumer showed as `unknown:0`). Consumers read
 * ids from manifest entries, so the numbering start is free. */
static int64_t g_next_id[4] = {1, 1, 1, 0};

/* Under g_obs_lock. */
static int64_t obs_manifest_add(uint8_t kind, uint8_t flg,
                                const char *name, uint16_t aux_a,
                                uint64_t shape, uint64_t aux_b) {
  uint32_t i = atomic_load(&MH->entry_count);
  if (i >= OBS_ENTRY_CAP) {
    atomic_fetch_add(&CNT[0].c[1], 1); /* manifest_overflow */
    return -1;
  }
  int64_t id = g_next_id[kind]++;
  if (id >= OBS_ENTRY_CAP) return -1;
  uint32_t len = (uint32_t)strlen(name);
  uint32_t noff = atomic_fetch_add(&MH->pool_used, len);
  memcpy(POOL + noff, name, len);
  ME[i] = (obs_me_t){ .shape_hash = shape, .aux_b = aux_b,
                      .id = (uint32_t)id, .name_off = noff,
                      .name_len = (uint16_t)len, .aux_a = aux_a,
                      .kind = kind, .flags = flg, ._pad = 0 };
  if (kind == MK_TOPIC || kind == MK_BINDING) {
    int line = 1;
    for (uint32_t j = 0; j < i; j++)
      if (ME[j].kind == MK_TOPIC || ME[j].kind == MK_BINDING) line++;
    g_cnt_line_for[kind][id] = line;
  }
  atomic_store_explicit(&MH->entry_count, i + 1, memory_order_release);
  atomic_fetch_add_explicit(&H->manifest_gen, 1, memory_order_release);
  return id;
}

static void obs_count(int kind, int64_t id, int cell, uint64_t delta) {
  if (id < 0 || id >= OBS_ENTRY_CAP) return;
  int line = g_cnt_line_for[kind][id];
  if (line > 0)
    atomic_fetch_add_explicit(&CNT[line].c[cell], delta,
                              memory_order_relaxed);
}

/* iris handoff-12 P22 — the per-binding backpressure cells
 * (PROTOCOL §6: 3 = queue_depth gauge, 4 = send_block_ns,
 * 5 = retries) were reserved since v0 and written by no path. The
 * arena's transport code measures them (fd occupancy and send
 * timing live there) and writes through these two: counters-tier,
 * relaxed, no observer gate — exactly the enabled-but-unobserved
 * contract cells 0-2 already keep. */
void lotus_obs_binding_cell_add(int64_t binding_id, int64_t cell,
                                uint64_t delta) {
  if (!obs_on() || cell < 0 || cell > 7) return;
  obs_count(MK_BINDING, binding_id, (int)cell, delta);
}

void lotus_obs_binding_cell_gauge(int64_t binding_id, int64_t cell,
                                  uint64_t value) {
  if (!obs_on() || cell < 0 || cell > 7) return;
  if (binding_id < 0 || binding_id >= OBS_ENTRY_CAP) return;
  int line = g_cnt_line_for[MK_BINDING][binding_id];
  if (line > 0)
    atomic_store_explicit(&CNT[line].c[cell], value,
                          memory_order_relaxed);
}

/* ---- record emission (per-thread ring, EPOCH-anchored) -------- */

static void obs_emit_raw(uint64_t w0, uint64_t w1) {
  if (t_ring == -2) return;
  if (t_ring == -1) {
    int r = atomic_fetch_add(&g_ring_next, 1);
    if ((uint32_t)r >= H->ring_count) {
      /* GH #296: under recording this thread's records would
       * silently vanish forever (the -2 disposition), which is
       * exactly what a recording must never do. Block-or-fail:
       * there is no ring left to block on, so fail the run. */
      if (lotus_obs_recording) {
        fprintf(stderr,
                "hale: LOTUS_OBS_RECORD needs one observation ring "
                "per emitting thread and all %u are taken; raise "
                "LOTUS_OBS_RINGS (max 64) or reduce thread count — "
                "a recording that drops records is not a recording\n",
                H->ring_count);
        fflush(NULL);
        _exit(70);
      }
      t_ring = -2;
      atomic_fetch_add_explicit(&CNT[0].c[0], 1, memory_order_relaxed);
      return;
    }
    t_ring = r;
    RD[r].tag_b = (uint32_t)(uintptr_t)pthread_self();
    /* v0.3: file-side map public ring → stable consumer id, so the
     * comparator can align PUBLIC streams across runs (ring claim
     * order races). Never touches the segment. Field contract
     * (matches both Rust readers): a = ring, b = subtype,
     * c = consumer. */
    if (lotus_obs_recording) {
      obs_rec_claim();
      obs_rec_blob_push(OBS_REC_TAG_META, (uint32_t)r,
                        OBS_REC_META_PUBRING, t_consumer_id, NULL, 0);
    }
  }
  /* GH #296 recording disposition: overwrite-oldest becomes
   * block-the-producer. The ring is effectively a bounded SPSC
   * queue against the drain cursor: writing record `head` clobbers
   * slot `head - ring_slots`, so wait until the drain has consumed
   * past it. One relaxed-load compare when the ring has room;
   * 50µs naps when it doesn't (recording is a debugging mode — the
   * stall is the contract, RFC #296 delta 1). g_rec_stop releases
   * waiters at teardown so nobody blocks on a drain that exited. */
  if (lotus_obs_recording) {
    uint64_t head = RD[t_ring].head; /* producer-owned */
    while (head - atomic_load_explicit(&g_rec_cursor[t_ring],
                                       memory_order_acquire)
           >= (uint64_t)H->ring_slots) {
      if (atomic_load_explicit(&g_rec_stop, memory_order_relaxed))
        break;
      struct timespec ts = {0, 50 * 1000};
      nanosleep(&ts, NULL);
    }
  }
  lotus_spsc_emit(g_seg, &RD[t_ring], H->ring_slots, (int64_t)w0,
                  (int64_t)w1);
  atomic_fetch_add_explicit(&CNT[0].c[2], 1, memory_order_relaxed);
}

/* Emit a recorder event onto this thread's PRIVATE ring —
 * lossless (blocks against the drain cursor), never the public
 * segment. Recording mode only. */
static void obs_rec_emit(uint32_t ev, uint64_t w1);
static void obs_rec_emit2(uint32_t ev, uint32_t id20, uint64_t w1);

/* Claim this thread's private ring (assigning a unique anonymous
 * consumer id if the thread has none). Split from the emit so an
 * identity-only claim never writes a spurious marker. */
static void obs_rec_claim(void) {
  if (!lotus_obs_recording || t_rec_ring != -2) return;
  {
    int r = atomic_fetch_add(&g_rec_ring_next, 1);
    if (r >= 64) {
      fprintf(stderr,
              "hale: LOTUS_OBS_RECORD supports at most 64 emitting "
              "threads — a recording that drops records is not a "
              "recording\n");
      fflush(NULL);
      _exit(70);
    }
    uint64_t *slots = calloc((size_t)H->ring_slots, 16);
    if (!slots) {
      fprintf(stderr,
              "hale: LOTUS_OBS_RECORD private ring allocation "
              "failed\n");
      fflush(NULL);
      _exit(74);
    }
    g_rec_rings[r].slots = slots;
    /* A thread with no stable identity (ingress readers) gets a
     * unique anonymous id — unique within the run, NOT stable
     * across runs (their consume streams replay by count only;
     * fleet identity is Phase 5). Never a shared fallback, and the
     * range starts at OBS_REC_ANON_BASE so it cannot overlap the
     * pinned range (guarded at note_consumer_locus). */
    if (t_consumer_id == 0)
      t_consumer_id = OBS_REC_ANON_BASE + (uint64_t)r;
    g_rec_rings[r].consumer = t_consumer_id;
    atomic_store_explicit(&g_rec_rings[r].head, 0,
                          memory_order_relaxed);
    t_rec_ring = r;
    obs_rec_emit(REC_EV_CONSUMER, g_rec_rings[r].consumer);
  }
}

static void obs_rec_emit2(uint32_t ev, uint32_t id20, uint64_t w1) {
  if (!lotus_obs_recording) return;
  obs_rec_claim();
  if (t_rec_ring < 0) return;
  obs_rec_ring_t *rr2 = &g_rec_rings[t_rec_ring];
  uint64_t h2 = atomic_load_explicit(&rr2->head, memory_order_relaxed);
  while (h2 - atomic_load_explicit(
             &g_rec_priv_cursor[t_rec_ring], memory_order_acquire)
         >= (uint64_t)H->ring_slots) {
    if (atomic_load_explicit(&g_rec_stop, memory_order_relaxed))
      return;
    struct timespec ts = {0, 50 * 1000};
    nanosleep(&ts, NULL);
  }
  uint64_t *slot2 =
      rr2->slots + (h2 & ((uint64_t)H->ring_slots - 1)) * 2;
  slot2[0] = ((uint64_t)(id20 & 0xFFFFFu))
      | (((uint64_t)ev & 0x1Fu) << 20);
  slot2[1] = w1;
  atomic_store_explicit(&rr2->head, h2 + 1, memory_order_release);
}

static void obs_rec_emit(uint32_t ev, uint64_t w1) {
  if (!lotus_obs_recording) return;
  obs_rec_claim();
  if (t_rec_ring < 0) return;
  obs_rec_ring_t *rr = &g_rec_rings[t_rec_ring];
  uint64_t head =
      atomic_load_explicit(&rr->head, memory_order_relaxed);
  while (head - atomic_load_explicit(
             &g_rec_priv_cursor[t_rec_ring], memory_order_acquire)
         >= (uint64_t)H->ring_slots) {
    if (atomic_load_explicit(&g_rec_stop, memory_order_relaxed))
      return;
    struct timespec ts = {0, 50 * 1000};
    nanosleep(&ts, NULL);
  }
  uint64_t *slot =
      rr->slots + (head & ((uint64_t)H->ring_slots - 1)) * 2;
  slot[0] = ((uint64_t)ev & 0x1Fu) << 20;
  slot[1] = w1;
  atomic_store_explicit(&rr->head, head + 1, memory_order_release);
}

static void obs_rec_write_failed(void) {
  fprintf(stderr,
          "hale: LOTUS_OBS_RECORD write to `%s` failed: %s — "
          "failing the run rather than truncating silently\n",
          g_rec_path, strerror(errno));
  fflush(NULL);
  _exit(74);
}

/* GH #296: the recording drain. Sweeps every ring oldest-first,
 * appends raw records to the file, publishes per-ring cursors that
 * blocked producers wait on. Reads are safe without the seqlock
 * snapshot: a slot below an acquire-loaded head is fully written
 * (release-ordered by lotus_spsc_emit), and it cannot be
 * overwritten because its producer blocks until the cursor passes
 * the slot being reclaimed. Exit: one final full sweep after
 * g_rec_stop, so everything published before teardown lands. */
static void *obs_record_drain_main(void *arg) {
  (void)arg;
  for (;;) {
    int stop = atomic_load_explicit(&g_rec_stop, memory_order_acquire);
    int wrote = 0;
    for (uint32_t r = 0; r < H->ring_count; r++) {
      uint64_t cur =
          atomic_load_explicit(&g_rec_cursor[r], memory_order_relaxed);
      uint64_t head =
          atomic_load_explicit(&RD[r].head, memory_order_acquire);
      while (cur < head) {
        const uint64_t *slot = (const uint64_t *)((const char *)g_seg
            + RD[r].data_off
            + (cur & ((uint64_t)H->ring_slots - 1)) * 16u);
        obs_rec_entry_t ent = { .tag = OBS_REC_TAG_RING, .ring = r,
                                .w0 = slot[0], .w1 = slot[1] };
        if (fwrite(&ent, sizeof ent, 1, g_rec_file) != 1) {
          obs_rec_write_failed();
        }
        g_rec_written++;
        cur++;
        wrote = 1;
      }
      atomic_store_explicit(&g_rec_cursor[r], cur,
                            memory_order_release);
    }
    /* Private recorder rings (consume/enq/journal markers). Read
     * head FIRST (acquire): a nonzero head guarantees the slots
     * pointer is visible (the claim stores slots before its first
     * release-store of head). */
    for (uint32_t r = 0; r < 64; r++) {
      uint64_t head = atomic_load_explicit(&g_rec_rings[r].head,
                                           memory_order_acquire);
      uint64_t cur = atomic_load_explicit(&g_rec_priv_cursor[r],
                                          memory_order_relaxed);
      if (cur >= head) continue;
      const uint64_t *slots = g_rec_rings[r].slots;
      while (cur < head) {
        const uint64_t *slot =
            slots + (cur & ((uint64_t)H->ring_slots - 1)) * 2;
        obs_rec_entry_t ent = { .tag = OBS_REC_TAG_RING,
                                .ring = OBS_REC_PRIV_RING | r,
                                .w0 = slot[0], .w1 = slot[1] };
        if (fwrite(&ent, sizeof ent, 1, g_rec_file) != 1) {
          obs_rec_write_failed();
        }
        g_rec_written++;
        cur++;
        wrote = 1;
      }
      atomic_store_explicit(&g_rec_priv_cursor[r], cur,
                            memory_order_release);
    }
    /* Blobs (payloads + journal values): pop the MPSC list. The
     * exchange yields newest-first; reverse to restore push order
     * (each pushing thread's own order survives the reversal). */
    obs_rec_blob_t *lifo = atomic_exchange_explicit(
        &g_rec_blob_head, NULL, memory_order_acquire);
    obs_rec_blob_t *fifo = NULL;
    while (lifo) {
      obs_rec_blob_t *nx = lifo->next;
      lifo->next = fifo;
      fifo = lifo;
      lifo = nx;
    }
    while (fifo) {
      obs_rec_blob_t *b = fifo;
      fifo = fifo->next;
      uint32_t hdr32[2] = { b->tag, b->a };
      uint64_t hdr64[3] = { b->b, b->c, b->size };
      uint64_t pad = (8 - (b->size & 7)) & 7;
      static const uint8_t zeros[8] = {0};
      if (fwrite(hdr32, sizeof hdr32, 1, g_rec_file) != 1 ||
          fwrite(hdr64, sizeof hdr64, 1, g_rec_file) != 1 ||
          (b->size &&
           fwrite(b + 1, b->size, 1, g_rec_file) != 1) ||
          (pad && fwrite(zeros, pad, 1, g_rec_file) != 1)) {
        obs_rec_write_failed();
      }
      atomic_fetch_sub_explicit(&g_rec_blob_bytes,
                                sizeof(obs_rec_blob_t) + b->size,
                                memory_order_relaxed);
      free(b);
      g_rec_written++;
      wrote = 1;
    }
    /* GH #296 phase 5 (WAL durability): identity used to be
     * authoritative only at the finalize re-stamp — so a crashed
     * run's artifact carried whatever the first probe snapshotted
     * (possibly zeros) and could never be admitted. Stamp the
     * header the moment the ctor-sequenced setters have run.
     * Drain-thread-owned; pwrite after a flush, so the append
     * position never moves. Finalize still re-stamps — both agree. */
    static uint64_t stamped_key = 0;
    uint64_t ident_key = g_obs_model_hash ^ g_obs_exec_digest[0] ^
                         g_obs_exec_digest[1];
    if (ident_key && ident_key != stamped_key) {
      uint64_t ident[6] = { g_obs_model_hash,
                            g_obs_exec_digest[0], g_obs_exec_digest[1],
                            g_obs_exec_digest[2], g_obs_exec_digest[3],
                            g_rec_env_full ? 0u : 1u };
      if (fflush(g_rec_file) != 0 ||
          pwrite(fileno(g_rec_file), ident, sizeof ident, 48) !=
              (ssize_t)sizeof ident) {
        obs_rec_write_failed();
      }
      stamped_key = ident_key;
      wrote = 1;
    }
    if (wrote && fflush(g_rec_file) != 0) {
      obs_rec_write_failed();
    }
    if (wrote && g_rec_durable &&
        fdatasync(fileno(g_rec_file)) != 0) {
      obs_rec_write_failed();
    }
    if (stop) break;
    if (!wrote) {
      struct timespec ts = {0, 200 * 1000};
      nanosleep(&ts, NULL);
    }
  }
  return NULL;
}

static void obs_emit(uint32_t ekind, uint32_t id, uint32_t size_class,
                     uint64_t w1) {
  uint64_t now = obs_mono_ns();
  uint64_t delta = (now - t_epoch_ns) >> H->ts_shift;
  if (t_epoch_ns == 0 || delta > OBS_TS_DELTA_MAX ||
      t_since_epoch >= OBS_EPOCH_EVERY) {
    t_epoch_ns = now;
    t_since_epoch = 0;
    obs_emit_raw((uint64_t)EK_EPOCH << 20, now);
    delta = 0;
  }
  t_since_epoch++;
  uint64_t w0 = ((uint64_t)(id & 0xFFFFFu))
      | ((uint64_t)(ekind & 0x1Fu) << 20)
      | ((uint64_t)(size_class & 0xFFu) << 25)
      | ((delta & OBS_TS_DELTA_MAX) << 33);
  obs_emit_raw(w0, w1);
}

static uint32_t obs_size_class(uint64_t bytes) {
  if (bytes == 0) return 0;
  uint32_t c = 1;
  while ((1ull << c) < bytes && c < 255) c++;
  return c;
}

/* dormant gate + 0→1 birth replay */
static _Atomic uint32_t g_last_observed = 0;

static void obs_replay_births(void);

static int obs_gate(void) {
  uint32_t oc = atomic_load_explicit(&C->observer_count,
                                     memory_order_relaxed);
  uint32_t last = atomic_load_explicit(&g_last_observed,
                                       memory_order_relaxed);
  if (oc != last) {
    atomic_store_explicit(&g_last_observed, oc, memory_order_relaxed);
    if (last == 0 && oc > 0) obs_replay_births();
  }
  return oc > 0;
}

/* ---- topics ---------------------------------------------------- */

static obs_topic_slot_t *obs_topic_slot(const char *subject,
                                        int networked) {
  int n = atomic_load_explicit(&g_topic_count, memory_order_acquire);
  for (int i = 0; i < n; i++) {
    if (strcmp(g_topics[i].subject, subject) == 0) return &g_topics[i];
  }
  pthread_mutex_lock(&g_obs_lock);
  n = atomic_load(&g_topic_count);
  for (int i = 0; i < n; i++) {
    if (strcmp(g_topics[i].subject, subject) == 0) {
      pthread_mutex_unlock(&g_obs_lock);
      return &g_topics[i];
    }
  }
  obs_topic_slot_t *slot = NULL;
  if (n < OBS_ENTRY_CAP) {
    int64_t id = obs_manifest_add(MK_TOPIC, networked ? 1 : 0, subject,
                                  0,
                                  obs_fnv(subject, obs_shape_for(subject)),
                                  0);
    if (id >= 0) {
      char *copy = strdup(subject);
      if (copy) {
        g_topics[n].subject = copy;
        g_topics[n].id = id;
        atomic_store(&g_topics[n].seq, 0);
        slot = &g_topics[n];
        atomic_store_explicit(&g_topic_count, n + 1,
                              memory_order_release);
        /* v0.3: file-side map manifest topic id → subject name, so
         * the comparator can align PUBLIC bus streams across runs
         * (manifest ids are registration order, which races). */
        if (lotus_obs_recording) {
          /* a = topic id, b = subtype, c = 0 (field contract as at
           * the pubring push). */
          obs_rec_blob_push(OBS_REC_TAG_META, (uint32_t)id,
                            OBS_REC_META_TOPIC, 0,
                            subject, strlen(subject) + 1);
          /* phase 5b: and the stable-hash spelling, which is the id
           * space PAYLOAD records live in. */
          obs_rec_blob_push(OBS_REC_TAG_META,
                            obs_subject_hash(subject),
                            OBS_REC_META_SUBJHASH, 0,
                            subject, strlen(subject) + 1);
        }
      }
    }
  }
  pthread_mutex_unlock(&g_obs_lock);
  return slot;
}

/* ---- instance table -------------------------------------------- */

static uint32_t obs_inst_id_of(void *self) {
  int n = atomic_load_explicit(&g_inst_count, memory_order_acquire);
  for (int i = 0; i < n; i++) {
    if (g_inst[i].self == self && g_inst[i].live) return g_inst[i].id;
  }
  return 0; /* unattributed */
}

/* ---- public probes (called from lotus_arena.c + codegen) ------- */

/* Returns a sequence TOKEN: the assigned per-topic seq + 1, or 0
 * when nothing was recorded (obs off, redispatch, unregistered
 * topic). Fanout passes the token to every lotus_obs_bus_deliver
 * for this publish, so a deliver record names the EXACT publish it
 * belongs to — the old reload-high-water-minus-one attributed both
 * of two racing same-subject deliveries to the later publish
 * (review round 3, finding 3). A local/SSA token also survives a
 * nested same-subject publish inside a handler, which any
 * TLS-based handoff would not. */
uint64_t lotus_obs_bus_publish(const char *subject,
                               void *publisher_self,
                               uint64_t payload_bytes) {
  if (!obs_on() || !subject) return 0;
  /* iris handoff-4 P15: consume the redispatch mark FIRST. An inbound
   * wire message re-dispatched by the reader thread is a delivery, not
   * a publish (it's covered by NET_DELIVER + per-subscriber
   * BUS_DELIVER) — skip it entirely. Consume-once so a nested publish
   * from a subscriber handler in the same fanout still counts. The
   * publisher TLS is now an ATTRIBUTION hint only, never a gate: the
   * published counter must fire for every genuine publish even when
   * the locus can't be attributed (cross-pool worker thread / free-fn
   * publish), which is what handoff-3 P13's positive gate broke. */
  int redispatch = g_obs_tls_redispatch;
  g_obs_tls_redispatch = 0;
  void *pub = publisher_self ? publisher_self : g_obs_tls_publisher;
  g_obs_tls_publisher = NULL;
  if (redispatch) return 0; /* inbound re-dispatch — not a publish */
  obs_topic_slot_t *t = obs_topic_slot(subject, 0);
  if (!t) return 0;
  uint64_t seq =
      atomic_fetch_add_explicit(&t->seq, 1, memory_order_relaxed);
  /* Counters are the dormant-mode contract (P4: enabled-but-unobserved
   * = counters only) — count before the observer gate and independent
   * of attribution. */
  obs_count(MK_TOPIC, t->id, 0, 1);              /* published */
  obs_count(MK_TOPIC, t->id, 2, payload_bytes);  /* bytes */
  if (!obs_gate()) return seq + 1;
  uint8_t mode = MODE[t->id & (OBS_ENTRY_CAP - 1)];
  if (mode < 2) return seq + 1; /* OFF / COUNTERS */
  uint32_t locus = pub ? obs_inst_id_of(pub) : 0; /* best-effort */
  /* iris handoff-7: w1 = locus:20 (HIGH bits 44..63) | seq:44
   * (low) — PROTOCOL §8 / iris emitter/protocol.h `obs_bus_w1`.
   * The original pack transposed the fields (locus low, seq<<20),
   * so every protocol-conformant consumer decoded `w1 >> 44` and
   * read the top of a small seq → 0: attribution was CORRECT all
   * along and unreadable. protocol.h is the executable reference;
   * the contract tests decode with these same shifts. */
  obs_emit(EK_BUS_PUBLISH, (uint32_t)t->id,
           obs_size_class(payload_bytes),
           (((uint64_t)locus & 0xFFFFFu) << 44)
               | (seq & 0xFFFFFFFFFFFULL));
  return seq + 1;
}

void lotus_obs_bus_deliver(const char *subject, void *subscriber_self,
                           uint64_t payload_bytes, uint64_t seq_token) {
  if (!obs_on() || !subject) return;
  obs_topic_slot_t *t = obs_topic_slot(subject, 0);
  if (!t) return;
  /* seq_token = the OWNING publish's seq + 1, from that publish's
   * probe return. 0 (no publish probe fired — dormant publish,
   * inbound redispatch) falls back to the old approximate
   * high-water read, which is only reachable when no exact pairing
   * exists to get wrong. */
  uint64_t seq;
  if (seq_token) {
    seq = seq_token - 1;
  } else {
    uint64_t hw =
        atomic_load_explicit(&t->seq, memory_order_relaxed);
    seq = hw ? hw - 1 : 0;
  }
  obs_count(MK_TOPIC, t->id, 1, 1); /* delivered */
  if (!obs_gate()) return;
  uint8_t mode = MODE[t->id & (OBS_ENTRY_CAP - 1)];
  if (mode < 2) return;
  uint32_t locus = subscriber_self ? obs_inst_id_of(subscriber_self) : 0;
  /* handoff-7: same protocol packing as the publish probe. */
  obs_emit(EK_BUS_DELIVER, (uint32_t)t->id,
           obs_size_class(payload_bytes),
           (((uint64_t)locus & 0xFFFFFu) << 44)
               | (seq & 0xFFFFFFFFFFFULL));
}

/* GH #296 Phase 1: the consumption event. Emitted by the arena's
 * dequeue-driven dispatch paths (main-queue drain, coop-pool drain,
 * mailbox drain, async coro start) right before the handler runs, on
 * the CONSUMING thread — so its ring position is the per-consumer
 * delivery order, which is the thing a replay reconstructs and which
 * enqueue-time BUS_DELIVER records cannot give (they land on the
 * publisher's ring in publisher order). Recording mode only: call
 * sites gate on lotus_obs_recording, and the fn re-checks so a
 * mis-gated caller cannot leak protocol-addendum records into a
 * plain observer's stream. */
void lotus_obs_record_consume(void *subscriber_self, uint64_t pub_id) {
  if (!lotus_obs_recording) return;
  if (!obs_on()) return;
  (void)obs_gate();
  (void)t_consume_seq; /* order = ring position since v0.2 */
  uint32_t locus = subscriber_self ? obs_inst_id_of(subscriber_self) : 0;
  /* Target locus rides w0's 20-bit id field; w1 is the FULL 64-bit
   * message id. (locus, msg_id) is the delivery identity — two
   * subscribers of one publish on one consumer are distinguishable
   * (review round 2, finding 5). */
  obs_rec_emit2(REC_EV_CONSUME, locus, pub_id);
}

void lotus_obs_note_consumer_locus(void *self) {
  if (!lotus_obs_live || !self) return;
  if (!obs_on()) return;
  uint32_t id = obs_inst_id_of(self);
  if (!id) return;
  uint64_t cid = 64 + (uint64_t)id;
  if ((lotus_obs_recording || lotus_replay_active) &&
      cid >= OBS_REC_ANON_BASE) {
    fprintf(stderr,
            "hale: recording/replay supports pinned consumer ids "
            "below %u (got %llu) — this run cannot be identified "
            "faithfully\n",
            OBS_REC_ANON_BASE, (unsigned long long)cid);
    fflush(NULL);
    _exit(70);
  }
  t_consumer_id = cid;
}

/* GH #296 Phase 2: capture a publish's payload once, on the
 * publishing thread, returning its deterministic identity —
 * pub_id = consumer_id:8 << 36 | per-thread seq:36. A re-executed
 * publisher thread re-derives the same ids in the same order,
 * which is what lets a replay match deliveries without global
 * coordination. Must be called BEFORE lotus_obs_bus_publish at
 * the same site: the redispatch mark (external ingress) is peeked
 * here and consumed there. */
uint64_t lotus_obs_record_publish_payload(const char *subject,
                                          const void *payload,
                                          uint64_t size,
                                          int raw_struct) {
  if ((!lotus_obs_recording && !lotus_replay_active) || !subject)
    return 0;
  /* phase 5b: an identity pinned by the ingress wire capture or the
   * replay injector — this dispatch IS that record; do not mint a
   * second id or push a second payload. */
  if (t_rec_forced_pub) {
    uint64_t forced = t_rec_forced_pub;
    t_rec_forced_pub = 0;
    return forced;
  }
  if (!obs_on()) return 0;
  int ingress = g_obs_tls_redispatch; /* peek only */
  if (t_consumer_id == 0) obs_rec_claim(); /* unique anon id */
  uint64_t cid = t_consumer_id;
  /* msg_id = consumer:16 | seq:48, full width — recorder records
   * are a private format, so nothing forces identity into 44 bits
   * (review round 2, finding 7). The guards fail loudly instead of
   * wrapping or overlapping. */
  if (cid > OBS_REC_CONSUMER_MAX || t_pub_seq >= 0xFFFFFFFFFFFFULL) {
    fprintf(stderr,
            "hale: recording identity out of range (consumer %llu, "
            "seq %llu) — refusing to record ambiguously\n",
            (unsigned long long)cid,
            (unsigned long long)t_pub_seq);
    fflush(NULL);
    _exit(70);
  }
  uint64_t pub_id = (cid << 48) | (++t_pub_seq & 0xFFFFFFFFFFFFULL);
  /* Under replay the identity is re-derived (same threads, same
   * per-thread order → same ids) so dequeues can be matched to the
   * recorded consume order; nothing is written anywhere. */
  if (!lotus_obs_recording) return pub_id;
  /* Topic identity in the ARTIFACT must survive a re-run: manifest
   * ids are registration-order, and two racing publishers register
   * their topics in either order — caught by the racing-replay
   * test comparing per-run ids. Use a stable subject hash. */
  uint32_t topic = 2166136261u;
  for (const char *q = subject; *q; q++) {
    topic ^= (uint8_t)*q;
    topic *= 16777619u;
  }
  /* flags: bit 0 = external ingress (wire re-dispatch); bit 1 =
   * raw in-process struct — for which the artifact stores METADATA
   * ONLY (declared size in flags bits 32..63, zero bytes): an ABI
   * snapshot would carry heap pointers, uninitialized padding, and
   * potentially secret-derived bytes while being unusable for
   * comparison anyway (review round 2, finding 8). Canonical
   * per-topic recording codecs are the staged fix. Wire captures
   * are canonical bytes, stored verbatim, unflagged. */
  if (raw_struct) {
    uint64_t flags = 2u | (ingress ? 1u : 0u) | (size << 32);
    obs_rec_blob_push(OBS_REC_TAG_PAYLOAD, topic, pub_id, flags,
                      NULL, 0);
  } else {
    uint64_t flags = ingress ? 1u : 0u;
    obs_rec_blob_push(OBS_REC_TAG_PAYLOAD, topic, pub_id, flags,
                      payload, size);
  }
  return pub_id;
}

/* GH #296 Phase 2: one record per QUEUED target at enqueue, on
 * the publishing thread's ring. pub_id 0 (unidentified structural
 * cell, or not recording) emits nothing. */
void lotus_obs_record_enqueue(uint64_t pub_id, void *subscriber_self) {
  if (!lotus_obs_recording || !pub_id) return;
  if (!obs_on()) return;
  uint32_t locus = subscriber_self ? obs_inst_id_of(subscriber_self) : 0;
  obs_rec_emit2(REC_EV_ENQ, locus, pub_id);
}

/* GH #296 Phase 3: journal a nondeterministic input read. The
 * value blob rides the capture list keyed (jkind, consumer, seq);
 * the ring record is the interleaving marker. */
void lotus_obs_journal_bytes(uint32_t jkind, const void *args,
                             uint64_t args_len, const void *p,
                             uint64_t n) {
  if (!lotus_obs_recording) return;
  if (!obs_on()) return;
  if (t_consumer_id == 0) obs_rec_claim();
  uint64_t cid = t_consumer_id;
  uint64_t seq = ++t_journal_seq;
  /* Env VALUES are withheld under the default policy — the entry
   * keeps the call's exact identity (kind + args) and the value's
   * length, so replay reports a NAMED withheld divergence instead
   * of substituting anything. */
  int withhold = !g_rec_env_full &&
      (jkind == JK_ENV_VAR || jkind == JK_ENV_ARG);
  if (args_len > 0xFFFFFFFFull) return;
  uint32_t al = (uint32_t)args_len;
  /* A withheld value still records its LENGTH (8-byte LE in the
   * result slot) so the artifact can truthfully report coverage —
   * a missing value and a 4KiB withheld credential are different
   * facts (round 3 follow-up). */
  uint64_t body_len = 4ull + al + (withhold ? 8 : n);
  uint8_t small[256];
  uint8_t *buf = body_len <= sizeof small ? small : malloc(body_len);
  if (!buf) {
    fprintf(stderr,
            "hale: LOTUS_OBS_RECORD journal allocation failed\n");
    fflush(NULL);
    _exit(74);
  }
  memcpy(buf, &al, 4);
  if (al) memcpy(buf + 4, args, al);
  if (withhold) {
    uint64_t vlen = n;
    memcpy(buf + 4 + al, &vlen, 8);
  } else if (n) {
    memcpy(buf + 4 + al, p, n);
  }
  uint64_t cfield = (seq & 0x7FFFFFFFFFFFFFFFull)
      | (withhold ? (1ull << 63) : 0);
  obs_rec_blob_push(OBS_REC_TAG_JOURNAL, jkind, cid, cfield, buf,
                    body_len);
  if (buf != small) free(buf);
  obs_rec_emit(REC_EV_JOURNAL,
               ((uint64_t)jkind << 56) | (seq & 0xFFFFFFFFFFFFFFULL));
}

void lotus_obs_journal_i64(uint32_t jkind, const void *args,
                           uint64_t args_len, int64_t v) {
  lotus_obs_journal_bytes(jkind, args, args_len, &v, 8);
}

/* binding_obs_id: cached on the remote entry by the caller (the
 * arena TU stores the id we return on first use; -1 = not yet
 * registered). */
int64_t lotus_obs_binding_register(const char *subject,
                                   int64_t transport_kind) {
  if (!obs_on() || !subject) return -1;
  pthread_mutex_lock(&g_obs_lock);
  int64_t id = obs_manifest_add(MK_BINDING, 1, subject,
                                (uint16_t)transport_kind, 0, 0);
  pthread_mutex_unlock(&g_obs_lock);
  return id;
}

/* iris handoff-4 P14: the record id field IS the topic id for ekinds
 * 3/4 (PROTOCOL §8) — the join key the consumer uses to place the NET
 * event on the fused topic row. It was hardcoded 0, so iris could not
 * associate any NET event with any topic and edges were structurally
 * impossible regardless of (origin, seq). The subject is in hand at
 * every emit site; resolve the topic slot here. binding_id still
 * drives the per-binding counter line only. */
static uint32_t obs_net_topic_id(const char *subject) {
  if (!subject) return 0;
  obs_topic_slot_t *t = obs_topic_slot(subject, 1);
  return t ? (uint32_t)t->id : 0;
}

void lotus_obs_net_send(const char *subject, int64_t binding_id,
                        uint64_t origin, uint64_t seq, uint64_t bytes) {
  if (!obs_on()) return;
  if (binding_id >= 0) {
    obs_count(MK_BINDING, binding_id, 0, 1);
    obs_count(MK_BINDING, binding_id, 2, bytes);
  }
  if (!obs_gate()) return;
  /* w1 = origin:16 | seq:48 (PROTOCOL §8, handoff-3 amendment).
   * origin identifies the SENDER; the receiver echoes both from
   * the wire so (origin, seq) is a fleet-unique message id. */
  obs_emit(EK_NET_SEND, obs_net_topic_id(subject), obs_size_class(bytes),
           ((uint64_t)origin & 0xFFFFu)
               | ((seq & 0xFFFFFFFFFFFFULL) << 16));
}

void lotus_obs_net_deliver(const char *subject, int64_t binding_id,
                           uint64_t origin, uint64_t seq, uint64_t bytes) {
  if (!obs_on()) return;
  if (binding_id >= 0) obs_count(MK_BINDING, binding_id, 1, 1);
  if (!obs_gate()) return;
  obs_emit(EK_NET_DELIVER, obs_net_topic_id(subject),
           obs_size_class(bytes),
           ((uint64_t)origin & 0xFFFFu)
               | ((seq & 0xFFFFFFFFFFFFULL) << 16));
}

/* iris handoff-8 P21: the adapter (Hale-owned-wire) ingest probe.
 * The C reader threads cache their obs binding id on the transport
 * entry struct; adapter inbound has no such struct, so the dedupe
 * cache lives here: subject -> (binding id, local seq). Called once
 * per inbound message from lotus_bus_dispatch_wire_inbound with the
 * wire (origin, seq) when the self-describing obs header was
 * present, or (0, 0) when headerless — the local per-subject
 * counter then supplies a monotonic seq so per-binding counting
 * still works (it just won't cross-process pair, same contract as
 * a non-framed transport). */
typedef struct {
  char *subject;
  int64_t id;
  uint64_t local_seq;
} obs_adapter_binding_t;
static obs_adapter_binding_t g_adapter_bindings[256];
static _Atomic int g_adapter_binding_count = 0;

void lotus_obs_adapter_net_deliver(const char *subject, uint64_t origin,
                                   uint64_t seq, uint64_t bytes) {
  if (!obs_on() || !subject) return;
  int n = atomic_load_explicit(&g_adapter_binding_count,
                               memory_order_acquire);
  obs_adapter_binding_t *row = NULL;
  for (int i = 0; i < n; i++) {
    if (strcmp(g_adapter_bindings[i].subject, subject) == 0) {
      row = &g_adapter_bindings[i];
      break;
    }
  }
  if (!row) {
    pthread_mutex_lock(&g_obs_lock);
    n = atomic_load(&g_adapter_binding_count);
    for (int i = 0; i < n; i++) {
      if (strcmp(g_adapter_bindings[i].subject, subject) == 0) {
        row = &g_adapter_bindings[i];
        break;
      }
    }
    if (!row && n < 256) {
      int64_t id = obs_manifest_add(MK_BINDING, 1, subject,
                                    2 /* adapter */, 0, 0);
      char *copy = strdup(subject);
      if (id >= 0 && copy) {
        g_adapter_bindings[n].subject = copy;
        g_adapter_bindings[n].id = id;
        g_adapter_bindings[n].local_seq = 0;
        row = &g_adapter_bindings[n];
        atomic_store_explicit(&g_adapter_binding_count, n + 1,
                              memory_order_release);
      } else if (copy) {
        free(copy);
      }
    }
    pthread_mutex_unlock(&g_obs_lock);
  }
  if (!row) return;
  uint64_t use_seq = seq ? seq : ++row->local_seq;
  lotus_obs_net_deliver(subject, row->id, origin, use_seq, bytes);
}

void lotus_obs_locus_birth(void *self, const char *type_name,
                           void *parent_self) {
  if (!obs_on() || !self || !type_name) return;
  pthread_mutex_lock(&g_obs_lock);
  /* type id: manifest lookup-or-add by name */
  uint32_t type_id = 0;
  uint32_t mc = atomic_load(&MH->entry_count);
  for (uint32_t i = 0; i < mc; i++) {
    if (ME[i].kind == MK_LOCUS_TYPE &&
        strncmp(POOL + ME[i].name_off, type_name, ME[i].name_len) == 0 &&
        type_name[ME[i].name_len] == 0) {
      type_id = ME[i].id;
      break;
    }
  }
  if (type_id == 0) {
    int64_t id = obs_manifest_add(MK_LOCUS_TYPE, 0, type_name, 0, 0, 0);
    if (id > 0) type_id = (uint32_t)id;
  }
  uint32_t inst_id =
      atomic_fetch_add(&g_next_inst_id, 1) & 0xFFFFFu;
  uint32_t parent = parent_self ? obs_inst_id_of(parent_self) : 0;
  int n = atomic_load(&g_inst_count);
  if (n < OBS_INSTANCE_CAP) {
    g_inst[n] = (obs_inst_t){ .self = self, .id = inst_id,
                              .type_id = type_id, .parent = parent,
                              .live = 1 };
    atomic_store_explicit(&g_inst_count, n + 1, memory_order_release);
  }
  pthread_mutex_unlock(&g_obs_lock);
  if (!obs_gate()) return;
  obs_emit(EK_LOCUS_BIRTH, inst_id, 0,
           ((uint64_t)parent) | ((uint64_t)(type_id & 0xFFFFFu) << 32));
}

void lotus_obs_locus_dissolve(void *self, int64_t reason) {
  if (!obs_on() || !self) return;
  uint32_t inst_id = 0;
  pthread_mutex_lock(&g_obs_lock);
  int n = atomic_load(&g_inst_count);
  for (int i = 0; i < n; i++) {
    if (g_inst[i].self == self && g_inst[i].live) {
      g_inst[i].live = 0;
      inst_id = g_inst[i].id;
      break;
    }
  }
  pthread_mutex_unlock(&g_obs_lock);
  if (inst_id == 0 || !obs_gate()) return;
  obs_emit(EK_LOCUS_DISSOLVE, inst_id, 0, (uint64_t)reason);
}

void lotus_obs_restart(const char *subject) {
  if (!obs_on()) return;
  if (!obs_gate()) return;
  obs_emit(EK_RESTART, 0, 0, 0);
  (void)subject;
}

static void obs_replay_births(void) {
  int n = atomic_load_explicit(&g_inst_count, memory_order_acquire);
  for (int i = 0; i < n; i++) {
    if (!g_inst[i].live) continue;
    obs_emit(EK_LOCUS_BIRTH, g_inst[i].id, 0,
             ((uint64_t)g_inst[i].parent)
                 | ((uint64_t)(g_inst[i].type_id & 0xFFFFFu) << 32));
  }
}
