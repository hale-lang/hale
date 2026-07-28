/*
 * Hale native observation emission — iris handoff P4 (2026-07-27).
 *
 * Implements the iris observation protocol (PROTOCOL.md v0.1 on
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
#define EK_DROP_MARK 14

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
  atomic_store(&H->flags, 0);
  unlink(g_reg_path);
  munmap(g_seg, g_seg_len);
  shm_unlink(g_shm_name);
  g_seg = NULL;
  atomic_store(&g_obs_state, 1);
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

  *H = (obs_hdr_t){ .magic = OBS_MAGIC, .proto_major = 0, .proto_minor = 1,
    .header_len = sizeof(obs_hdr_t), .total_len = g_seg_len,
    .pid = (uint32_t)getpid(), .ring_count = (uint32_t)rings,
    .ring_slots = (uint32_t)slots, .ts_shift = 4,
    .started_mono_ns = obs_mono_ns(), .started_wall_ns = obs_wall_ns(),
    .control_off = control_off, .manifest_off = manifest_off,
    .manifest_len = manifest_len, .modemask_off = modemask_off,
    .counters_off = counters_off, .counters_len = counters_len,
    .rings_off = rings_off };
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
int lotus_obs_note_publisher_wanted = 0;

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
    if (e && e[0] == '1') {
      const char *rs = getenv("LOTUS_OBS_RINGS");
      const char *ss = getenv("LOTUS_OBS_SLOTS");
      int64_t rings = rs ? atoll(rs) : 8;
      int64_t slots = ss ? atoll(ss) : 4096;
      if (rings < 1 || rings > 64) rings = 8;
      if (slots < 64 || (slots & (slots - 1))) slots = 4096;
      st = obs_create(rings, slots) ? 2 : 1;
    } else {
      st = 1;
    }
    if (st == 2) lotus_obs_note_publisher_wanted = 1;
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

/* ---- record emission (per-thread ring, EPOCH-anchored) -------- */

static void obs_emit_raw(uint64_t w0, uint64_t w1) {
  if (t_ring == -2) return;
  if (t_ring == -1) {
    int r = atomic_fetch_add(&g_ring_next, 1);
    if ((uint32_t)r >= H->ring_count) {
      t_ring = -2;
      atomic_fetch_add_explicit(&CNT[0].c[0], 1, memory_order_relaxed);
      return;
    }
    t_ring = r;
    RD[r].tag_b = (uint32_t)(uintptr_t)pthread_self();
  }
  lotus_spsc_emit(g_seg, &RD[t_ring], H->ring_slots, (int64_t)w0,
                  (int64_t)w1);
  atomic_fetch_add_explicit(&CNT[0].c[2], 1, memory_order_relaxed);
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

void lotus_obs_bus_publish(const char *subject, void *publisher_self,
                           uint64_t payload_bytes) {
  if (!obs_on() || !subject) return;
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
  if (redispatch) return; /* inbound re-dispatch — not a publish */
  obs_topic_slot_t *t = obs_topic_slot(subject, 0);
  if (!t) return;
  uint64_t seq =
      atomic_fetch_add_explicit(&t->seq, 1, memory_order_relaxed);
  /* Counters are the dormant-mode contract (P4: enabled-but-unobserved
   * = counters only) — count before the observer gate and independent
   * of attribution. */
  obs_count(MK_TOPIC, t->id, 0, 1);              /* published */
  obs_count(MK_TOPIC, t->id, 2, payload_bytes);  /* bytes */
  if (!obs_gate()) return;
  uint8_t mode = MODE[t->id & (OBS_ENTRY_CAP - 1)];
  if (mode < 2) return; /* OFF / COUNTERS */
  uint32_t locus = pub ? obs_inst_id_of(pub) : 0; /* best-effort */
  obs_emit(EK_BUS_PUBLISH, (uint32_t)t->id,
           obs_size_class(payload_bytes),
           ((uint64_t)locus & 0xFFFFFu)
               | ((seq & 0xFFFFFFFFFFFULL) << 20));
}

void lotus_obs_bus_deliver(const char *subject, void *subscriber_self,
                           uint64_t payload_bytes) {
  if (!obs_on() || !subject) return;
  obs_topic_slot_t *t = obs_topic_slot(subject, 0);
  if (!t) return;
  uint64_t seq =
      atomic_load_explicit(&t->seq, memory_order_relaxed);
  obs_count(MK_TOPIC, t->id, 1, 1); /* delivered */
  if (!obs_gate()) return;
  uint8_t mode = MODE[t->id & (OBS_ENTRY_CAP - 1)];
  if (mode < 2) return;
  uint32_t locus = subscriber_self ? obs_inst_id_of(subscriber_self) : 0;
  obs_emit(EK_BUS_DELIVER, (uint32_t)t->id,
           obs_size_class(payload_bytes),
           ((uint64_t)locus & 0xFFFFFu)
               | (((seq ? seq - 1 : 0) & 0xFFFFFFFFFFFULL) << 20));
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
