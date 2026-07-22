/* synth — synthetic iris protocol emitter.
 *
 * Simulates a small order-processing Hale system (main →
 * supervisor → producer/router/workers, one networked binding)
 * and emits protocol-v0 telemetry into a shm segment, so the
 * consumer stack can be built and tested before any real
 * runtime emits.
 *
 * Not a Hale program on purpose: this is the protocol's test
 * fixture, independent of the toolchain.
 *
 *   ./synth [--rate N] [--rings N] [--slots N] [--duration S]
 *           [--loss PPM] [--churn S]
 *
 * SIGUSR1 writes a post-mortem dump; SIGINT/SIGTERM cleans up.
 */
#define _GNU_SOURCE
#include "protocol.h"

#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

/* ---- config ------------------------------------------------ */

static struct {
  uint64_t rate;      /* msgs/sec aggregate */
  uint32_t rings;
  uint32_t slots;     /* power of two */
  uint32_t duration;  /* seconds, 0 = forever */
  uint32_t loss_ppm;  /* NET_DELIVER drop rate */
  uint32_t churn_s;   /* worker restart period */
} cfg = { .rate = 50000, .rings = 4, .slots = 1u << 16,
          .duration = 0, .loss_ppm = 0, .churn_s = 7 };

/* ---- topology ---------------------------------------------- */

enum { T_ORDERS_NEW = 1, T_ORDERS_FILL = 2, T_RISK_CHECK = 3, T_METRICS_TICK = 4 };
enum { LT_MAIN = 1, LT_SUPERVISOR = 2, LT_PRODUCER = 3, LT_ROUTER = 4, LT_WORKER = 5 };
enum { B_ORDERS_UNIX = 0 };
#define N_WORKERS 3

/* locus instance ids (dynamic space, seeded by births) */
enum { LI_MAIN = 1, LI_SUP = 2, LI_PRODUCER = 3, LI_ROUTER = 4, LI_WORKER0 = 5 };
static _Atomic uint32_t next_instance = LI_WORKER0 + N_WORKERS;
static uint32_t workers[N_WORKERS]; /* current instance id per worker slot */

/* ---- segment ----------------------------------------------- */

static void *seg;
static size_t seg_len;
static obs_header *H;
static obs_control *CTRL;
static obs_manifest_hdr *MH;
static obs_manifest_entry *ME;
static char *POOL;
static uint8_t *MODE;
static obs_counter_line *CNT; /* [0]=global, then per topic/binding entry order */
static obs_ring_desc *RD;
static char shm_name[64];
static char reg_path[256];

static volatile sig_atomic_t running = 1;
static volatile sig_atomic_t want_dump = 0;

static uint64_t now_mono_ns(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}
static uint64_t now_wall_ns(void) {
  struct timespec ts;
  clock_gettime(CLOCK_REALTIME, &ts);
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

/* ---- manifest ---------------------------------------------- */

static uint32_t pool_put(const char *s) {
  uint32_t len = (uint32_t)strlen(s);
  uint32_t off = atomic_fetch_add(&MH->pool_used, len);
  memcpy(POOL + off, s, len);
  return off;
}

static void manifest_add(uint32_t id, uint8_t kind, uint8_t flags,
                         const char *name, uint16_t aux_a,
                         uint64_t shape_hash, uint64_t aux_b) {
  uint32_t i = atomic_load(&MH->entry_count);
  if (i >= MH->entry_cap) {
    atomic_fetch_add(&CNT[0].c[OBS_CG_MANIFEST_OVERFLOW], 1);
    return;
  }
  obs_manifest_entry *e = &ME[i];
  e->id = id; e->kind = kind; e->flags = flags; e->_pad = 0;
  e->name_off = pool_put(name);
  e->name_len = (uint16_t)strlen(name);
  e->aux_a = aux_a; e->shape_hash = shape_hash; e->aux_b = aux_b;
  atomic_store_explicit(&MH->entry_count, i + 1, memory_order_release);
  atomic_fetch_add_explicit(&H->manifest_gen, 1, memory_order_release);
}

/* trivial stand-in for the real canonicalized shape hash
 * (PROTOCOL.md freeze open item) — FNV-1a of "name:shape" */
static uint64_t shape_hash(const char *name, const char *shape) {
  uint64_t h = 0xcbf29ce484222325ull;
  for (const char *p = name; *p; p++) { h ^= (uint8_t)*p; h *= 0x100000001b3ull; }
  h ^= ':'; h *= 0x100000001b3ull;
  for (const char *p = shape; *p; p++) { h ^= (uint8_t)*p; h *= 0x100000001b3ull; }
  return h;
}

/* counter line index: [0] global; topics/bindings in manifest
 * entry order counting only those kinds. Precomputed at setup. */
static uint32_t cnt_topic[8];   /* topic_id -> line index */
static uint32_t cnt_binding[8]; /* binding_id -> line index */

/* ---- rings ------------------------------------------------- */

typedef struct {
  obs_ring_desc *d;
  obs_record *slots;
  uint64_t last_epoch_ns; /* producer-side epoch base */
  uint64_t rng;           /* xorshift state */
} ring_ctx;

static ring_ctx *RC;

static void ring_raw_emit(ring_ctx *r, uint64_t w0, uint64_t w1) {
  uint64_t h = atomic_load_explicit(&r->d->head, memory_order_relaxed);
  obs_record *slot = &r->slots[h & (cfg.slots - 1)];
  slot->word1 = w1;
  slot->word0 = w0;
  atomic_store_explicit(&r->d->head, h + 1, memory_order_release);
  atomic_fetch_add_explicit(&CNT[0].c[OBS_CG_RECORDS_TOTAL], 1,
                            memory_order_relaxed);
}

static void ring_epoch(ring_ctx *r, uint64_t mono) {
  r->last_epoch_ns = mono;
  ring_raw_emit(r, obs_word0(0, OBS_EK_EPOCH, 0, 0), mono);
}

static void ring_emit(ring_ctx *r, uint32_t id, uint32_t ekind,
                      uint32_t szclass, uint64_t w1) {
  uint64_t mono = now_mono_ns();
  uint64_t delta = (mono - r->last_epoch_ns) >> H->ts_shift;
  if (r->last_epoch_ns == 0 || delta > OBS_TS_DELTA_MAX ||
      mono - r->last_epoch_ns > 1000000000ull) {
    ring_epoch(r, mono);
    delta = 0;
  }
  ring_raw_emit(r, obs_word0(id, ekind, szclass, delta), w1);
}

static int emitting(void) {
  return atomic_load_explicit(&CTRL->observer_count,
                              memory_order_relaxed) > 0;
}
static uint8_t topic_mode(uint32_t topic_id) {
  return MODE[topic_id];
}

static uint64_t xorshift(uint64_t *s) {
  uint64_t x = *s; x ^= x << 13; x ^= x >> 7; x ^= x << 17;
  return *s = x;
}

/* ---- simulated message flow -------------------------------- */

static _Atomic uint64_t net_seq = 0;

/* One end-to-end order: producer -> (net) -> router -> worker
 * -> fill back to producer. All simulated on one scheduler. */
static void simulate_order(ring_ctx *r) {
  int obs = emitting();
  uint32_t sz = 6 + (uint32_t)(xorshift(&r->rng) % 4); /* 64..512 B class */

  /* producer publishes orders.new */
  atomic_store_explicit(&r->d->current_locus, LI_PRODUCER,
                        memory_order_relaxed);
  if (topic_mode(T_ORDERS_NEW) >= OBS_MODE_COUNTERS) {
    atomic_fetch_add_explicit(&CNT[cnt_topic[T_ORDERS_NEW]].c[OBS_CT_PUBLISHED], 1, memory_order_relaxed);
    atomic_fetch_add_explicit(&CNT[cnt_topic[T_ORDERS_NEW]].c[OBS_CT_BYTES], 1u << sz, memory_order_relaxed);
  }
  if (obs && topic_mode(T_ORDERS_NEW) >= OBS_MODE_PACKED)
    ring_emit(r, T_ORDERS_NEW, OBS_EK_BUS_PUBLISH, sz, 0);

  /* networked hop */
  uint64_t seq = atomic_fetch_add(&net_seq, 1);
  atomic_fetch_add_explicit(&CNT[cnt_binding[B_ORDERS_UNIX]].c[OBS_CB_SENT], 1, memory_order_relaxed);
  atomic_store_explicit(&CNT[cnt_binding[B_ORDERS_UNIX]].c[OBS_CB_SEQ_HW], seq, memory_order_relaxed);
  if (obs) ring_emit(r, T_ORDERS_NEW, OBS_EK_NET_SEND, sz,
                     obs_net_w1(B_ORDERS_UNIX, seq));

  int lost = cfg.loss_ppm &&
             (xorshift(&r->rng) % 1000000) < cfg.loss_ppm;
  if (lost) return; /* the message vanishes; seq gap is the evidence */

  atomic_fetch_add_explicit(&CNT[cnt_binding[B_ORDERS_UNIX]].c[OBS_CB_DELIVERED], 1, memory_order_relaxed);
  if (obs) ring_emit(r, T_ORDERS_NEW, OBS_EK_NET_DELIVER, sz,
                     obs_net_w1(B_ORDERS_UNIX, seq));

  /* router */
  atomic_store_explicit(&r->d->current_locus, LI_ROUTER, memory_order_relaxed);
  if (topic_mode(T_ORDERS_NEW) >= OBS_MODE_COUNTERS)
    atomic_fetch_add_explicit(&CNT[cnt_topic[T_ORDERS_NEW]].c[OBS_CT_DELIVERED], 1, memory_order_relaxed);
  if (obs && topic_mode(T_ORDERS_NEW) >= OBS_MODE_PACKED)
    ring_emit(r, T_ORDERS_NEW, OBS_EK_BUS_DELIVER, sz, 0);

  /* router -> risk.check -> worker */
  uint32_t w = (uint32_t)(xorshift(&r->rng) % N_WORKERS);
  if (topic_mode(T_RISK_CHECK) >= OBS_MODE_COUNTERS) {
    atomic_fetch_add_explicit(&CNT[cnt_topic[T_RISK_CHECK]].c[OBS_CT_PUBLISHED], 1, memory_order_relaxed);
    atomic_fetch_add_explicit(&CNT[cnt_topic[T_RISK_CHECK]].c[OBS_CT_DELIVERED], 1, memory_order_relaxed);
  }
  if (obs && topic_mode(T_RISK_CHECK) >= OBS_MODE_PACKED) {
    ring_emit(r, T_RISK_CHECK, OBS_EK_BUS_PUBLISH, 4, 0);
    atomic_store_explicit(&r->d->current_locus, workers[w], memory_order_relaxed);
    ring_emit(r, T_RISK_CHECK, OBS_EK_BUS_DELIVER, 4, 0);
  }

  /* worker -> orders.fill -> producer */
  if (topic_mode(T_ORDERS_FILL) >= OBS_MODE_COUNTERS) {
    atomic_fetch_add_explicit(&CNT[cnt_topic[T_ORDERS_FILL]].c[OBS_CT_PUBLISHED], 1, memory_order_relaxed);
    atomic_fetch_add_explicit(&CNT[cnt_topic[T_ORDERS_FILL]].c[OBS_CT_DELIVERED], 1, memory_order_relaxed);
  }
  if (obs && topic_mode(T_ORDERS_FILL) >= OBS_MODE_PACKED) {
    ring_emit(r, T_ORDERS_FILL, OBS_EK_BUS_PUBLISH, 5, 0);
    atomic_store_explicit(&r->d->current_locus, LI_PRODUCER, memory_order_relaxed);
    ring_emit(r, T_ORDERS_FILL, OBS_EK_BUS_DELIVER, 5, 0);
  }
  atomic_store_explicit(&r->d->current_locus, 0, memory_order_relaxed);
}

/* ---- worker churn (thread 0 only, ring 0) ------------------ */

static void churn_worker(ring_ctx *r, uint32_t slot_idx) {
  uint32_t old = workers[slot_idx];
  ring_emit(r, old, OBS_EK_LOCUS_DISSOLVE, 0, 1 /* reason: fault */);
  ring_emit(r, LI_SUP, OBS_EK_SUPERV_TRANS, 0, 0 /* on_failure */);
  uint32_t fresh = atomic_fetch_add(&next_instance, 1);
  workers[slot_idx] = fresh;
  ring_emit(r, fresh, OBS_EK_RESTART, 0, (1u << 0) /* attempt 1 */);
  ring_emit(r, fresh, OBS_EK_LOCUS_BIRTH, 0, obs_birth_w1(LI_SUP, LT_WORKER));
}

/* ---- producer threads -------------------------------------- */

static void *producer_thread(void *arg) {
  ring_ctx *r = (ring_ctx *)arg;
  uint32_t ring_idx = r->d->sched_id;
  uint64_t per_ring = cfg.rate / cfg.rings;
  uint64_t start = now_mono_ns();
  uint64_t emitted = 0;
  uint64_t last_churn = start, last_metrics = start;
  uint32_t churned = 0;

  while (running) {
    uint64_t now = now_mono_ns();
    uint64_t target = (now - start) / 1000000000.0 * per_ring;
    while (emitted < target && running) {
      simulate_order(r);
      emitted++;
    }
    if (ring_idx == 0) {
      if (cfg.churn_s && now - last_churn > (uint64_t)cfg.churn_s * 1000000000ull) {
        churn_worker(r, churned++ % N_WORKERS);
        last_churn = now;
      }
      /* late-registered topic: appears after 2 s, then ticks 1/s
       * (exercises consumer manifest re-scan) */
      if (now - start > 2000000000ull && now - last_metrics > 1000000000ull) {
        if (MODE[T_METRICS_TICK] == 0xFF) { /* unregistered sentinel */
          /* counter line = 1 + count of topic/binding entries
           * registered before this one (manifest entry order) */
          uint32_t line = 1, n = atomic_load(&MH->entry_count);
          for (uint32_t i2 = 0; i2 < n; i2++)
            if (ME[i2].kind == OBS_MK_TOPIC || ME[i2].kind == OBS_MK_BINDING)
              line++;
          cnt_topic[T_METRICS_TICK] = line;
          manifest_add(T_METRICS_TICK, OBS_MK_TOPIC, 0, "metrics.tick", 0,
                       shape_hash("metrics.tick", "{at:U64}"), 0);
          MODE[T_METRICS_TICK] = OBS_MODE_PACKED;
        }
        if (topic_mode(T_METRICS_TICK) >= OBS_MODE_COUNTERS)
          atomic_fetch_add_explicit(&CNT[cnt_topic[T_METRICS_TICK]].c[OBS_CT_PUBLISHED], 1, memory_order_relaxed);
        if (emitting() && topic_mode(T_METRICS_TICK) >= OBS_MODE_PACKED)
          ring_emit(r, T_METRICS_TICK, OBS_EK_BUS_PUBLISH, 3, 0);
        last_metrics = now;
      }
      if (want_dump) { want_dump = 0; goto dump; }
      if (cfg.duration && now - start > (uint64_t)cfg.duration * 1000000000ull)
        running = 0;
    }
    struct timespec ts = { 0, 200000 }; /* 200 µs tick */
    nanosleep(&ts, NULL);
    continue;
  dump: {
      char path[128];
      snprintf(path, sizeof path, "hale-obs-%d.dump", (int)getpid());
      int fd = open(path, O_CREAT | O_TRUNC | O_WRONLY, 0644);
      if (fd >= 0) {
        obs_dump_hdr dh = { OBS_DUMP_MAGIC, 1, 0 };
        uint64_t f = atomic_load(&H->flags);
        atomic_store(&H->flags, f | OBS_FLAG_POSTMORTEM);
        ssize_t r1 = write(fd, &dh, sizeof dh);
        ssize_t r2 = write(fd, seg, seg_len);
        atomic_store(&H->flags, f);
        close(fd);
        fprintf(stderr, "synth: dump -> %s (%zd+%zd bytes)\n", path, r1, r2);
      }
    }
  }
  return NULL;
}

/* ---- setup ------------------------------------------------- */

static void on_signal(int sig) {
  if (sig == SIGUSR1) want_dump = 1;
  else running = 0;
}

static size_t page_up(size_t n) { return (n + OBS_PAGE - 1) & ~((size_t)OBS_PAGE - 1); }

int main(int argc, char **argv) {
  for (int i = 1; i < argc; i++) {
    if (!strcmp(argv[i], "--rate") && i + 1 < argc) cfg.rate = strtoull(argv[++i], 0, 10);
    else if (!strcmp(argv[i], "--rings") && i + 1 < argc) cfg.rings = (uint32_t)atoi(argv[++i]);
    else if (!strcmp(argv[i], "--slots") && i + 1 < argc) cfg.slots = (uint32_t)atoi(argv[++i]);
    else if (!strcmp(argv[i], "--duration") && i + 1 < argc) cfg.duration = (uint32_t)atoi(argv[++i]);
    else if (!strcmp(argv[i], "--loss") && i + 1 < argc) cfg.loss_ppm = (uint32_t)atoi(argv[++i]);
    else if (!strcmp(argv[i], "--churn") && i + 1 < argc) cfg.churn_s = (uint32_t)atoi(argv[++i]);
    else { fprintf(stderr, "usage: %s [--rate N] [--rings N] [--slots P2] [--duration S] [--loss PPM] [--churn S]\n", argv[0]); return 2; }
  }
  if (cfg.slots & (cfg.slots - 1)) { fprintf(stderr, "--slots must be a power of two\n"); return 2; }

  /* --- layout --- */
  uint32_t entry_cap = 256;
  size_t manifest_len = page_up(sizeof(obs_manifest_hdr) + entry_cap * 32 + 8192);
  size_t modemask_len = page_up(entry_cap); /* sized to id capacity, not u20 */
  size_t counters_len = page_up((1 + entry_cap) * 64);
  size_t rings_hdr = page_up(cfg.rings * sizeof(obs_ring_desc));
  size_t ring_bytes = (size_t)cfg.slots * 16;
  size_t off = 0;
  size_t header_off = off;            off += OBS_PAGE;
  size_t control_off = off;           off += OBS_PAGE;
  size_t manifest_off = off;          off += manifest_len;
  size_t modemask_off = off;          off += modemask_len;
  size_t counters_off = off;          off += counters_len;
  size_t rings_off = off;             off += rings_hdr + (size_t)cfg.rings * ring_bytes;
  seg_len = off;

  snprintf(shm_name, sizeof shm_name, OBS_SHM_NAME_FMT, (int)getpid());
  int fd = shm_open(shm_name, O_CREAT | O_EXCL | O_RDWR, 0600);
  if (fd < 0) { perror("shm_open"); return 1; }
  if (ftruncate(fd, (off_t)seg_len) < 0) { perror("ftruncate"); return 1; }
  seg = mmap(NULL, seg_len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (seg == MAP_FAILED) { perror("mmap"); return 1; }
  close(fd);

  H = (obs_header *)((char *)seg + header_off);
  CTRL = (obs_control *)((char *)seg + control_off);
  MH = (obs_manifest_hdr *)((char *)seg + manifest_off);
  ME = (obs_manifest_entry *)((char *)seg + manifest_off + sizeof(obs_manifest_hdr));
  MODE = (uint8_t *)seg + modemask_off;
  CNT = (obs_counter_line *)((char *)seg + counters_off);
  RD = (obs_ring_desc *)((char *)seg + rings_off);

  MH->entry_cap = entry_cap;
  MH->pool_off = (uint32_t)(sizeof(obs_manifest_hdr) + entry_cap * 32);
  POOL = (char *)MH + MH->pool_off;

  *H = (obs_header){
    .magic = OBS_MAGIC,
    .proto_major = OBS_PROTO_MAJOR, .proto_minor = OBS_PROTO_MINOR,
    .header_len = sizeof(obs_header),
    .total_len = seg_len, .pid = (uint32_t)getpid(),
    .ring_count = cfg.rings, .ring_slots = cfg.slots,
    .ts_shift = 4, /* 16 ns units */
    .started_mono_ns = now_mono_ns(), .started_wall_ns = now_wall_ns(),
    .control_off = control_off,
    .manifest_off = manifest_off, .manifest_len = manifest_len,
    .modemask_off = modemask_off,
    .counters_off = counters_off, .counters_len = counters_len,
    .rings_off = rings_off,
  };
  atomic_store(&H->flags, OBS_FLAG_ALIVE);

  /* mode mask defaults; 0xFF marks metrics.tick unregistered */
  memset(MODE, OBS_MODE_PACKED, entry_cap);
  MODE[T_METRICS_TICK] = 0xFF;

  /* --- manifest: topics, locus types, binding, schedulers --- */
  manifest_add(T_ORDERS_NEW, OBS_MK_TOPIC, OBS_MF_NETWORKED, "orders.new", 0,
               shape_hash("orders.new", "{id:U64,qty:U32,px:Decimal}"), 0);
  manifest_add(T_ORDERS_FILL, OBS_MK_TOPIC, 0, "orders.fill", 0,
               shape_hash("orders.fill", "{id:U64,px:Decimal}"), 0);
  manifest_add(T_RISK_CHECK, OBS_MK_TOPIC, 0, "risk.check", 0,
               shape_hash("risk.check", "{id:U64,limit:U32}"), 0);
  manifest_add(LT_MAIN, OBS_MK_LOCUS_TYPE, 0, "Main", 0, 0, 0);
  manifest_add(LT_SUPERVISOR, OBS_MK_LOCUS_TYPE, 0, "Supervisor", 0, 0, 0);
  manifest_add(LT_PRODUCER, OBS_MK_LOCUS_TYPE, 0, "Producer", 0, 0, 0);
  manifest_add(LT_ROUTER, OBS_MK_LOCUS_TYPE, 0, "Router", 0, 0, 0);
  manifest_add(LT_WORKER, OBS_MK_LOCUS_TYPE, 0, "Worker", 0, 0, 0);
  manifest_add(B_ORDERS_UNIX, OBS_MK_BINDING, OBS_MF_NETWORKED, "unix:/tmp/orders.sock",
               OBS_TRANSPORT_UNIX, 0, T_ORDERS_NEW);
  for (uint32_t i = 0; i < cfg.rings; i++) {
    char nm[16]; snprintf(nm, sizeof nm, "sched%u", i);
    manifest_add(i, OBS_MK_SCHEDULER, 0, nm, 0, 0, i);
  }

  /* counter line indices: [0] global, then topic/binding entries
   * in manifest order */
  {
    uint32_t line = 1;
    uint32_t n = atomic_load(&MH->entry_count);
    for (uint32_t i = 0; i < n; i++) {
      if (ME[i].kind == OBS_MK_TOPIC) cnt_topic[ME[i].id] = line++;
      else if (ME[i].kind == OBS_MK_BINDING) cnt_binding[ME[i].id] = line++;
    }
  }

  /* --- rings --- */
  RC = calloc(cfg.rings, sizeof *RC);
  for (uint32_t i = 0; i < cfg.rings; i++) {
    RD[i] = (obs_ring_desc){ .data_off = rings_off + rings_hdr + (uint64_t)i * ring_bytes,
                             .sched_id = i };
    RC[i].d = &RD[i];
    RC[i].slots = (obs_record *)((char *)seg + RD[i].data_off);
    RC[i].rng = 0x9E3779B97F4A7C15ull ^ (i + 1);
  }

  /* --- registration file --- */
  const char *xdg = getenv("XDG_RUNTIME_DIR");
  char dir[192];
  if (xdg) snprintf(dir, sizeof dir, OBS_REG_DIR_FMT, xdg);
  else snprintf(dir, sizeof dir, "/tmp/hale-obs");
  mkdir(dir, 0700);
  snprintf(reg_path, sizeof reg_path, "%s/%d.json", dir, (int)getpid());
  char tmp[280]; snprintf(tmp, sizeof tmp, "%s.tmp", reg_path);
  FILE *rf = fopen(tmp, "w");
  if (rf) {
    fprintf(rf,
      "{\n  \"proto\": \"%d.%d\",\n  \"pid\": %d,\n  \"exe\": \"synth\",\n"
      "  \"shm\": \"%s\",\n  \"started_mono_ns\": %llu,\n"
      "  \"started_wall_ns\": %llu,\n  \"rings\": %u\n}\n",
      OBS_PROTO_MAJOR, OBS_PROTO_MINOR, (int)getpid(), shm_name,
      (unsigned long long)H->started_mono_ns,
      (unsigned long long)H->started_wall_ns, cfg.rings);
    fclose(rf);
    rename(tmp, reg_path);
  }

  signal(SIGINT, on_signal);
  signal(SIGTERM, on_signal);
  signal(SIGUSR1, on_signal);

  /* --- initial structural events (ring 0) --- */
  ring_ctx *r0 = &RC[0];
  ring_emit(r0, LI_MAIN, OBS_EK_LOCUS_BIRTH, 0, obs_birth_w1(0, LT_MAIN));
  ring_emit(r0, LI_SUP, OBS_EK_LOCUS_BIRTH, 0, obs_birth_w1(LI_MAIN, LT_SUPERVISOR));
  ring_emit(r0, LI_PRODUCER, OBS_EK_LOCUS_BIRTH, 0, obs_birth_w1(LI_SUP, LT_PRODUCER));
  ring_emit(r0, LI_ROUTER, OBS_EK_LOCUS_BIRTH, 0, obs_birth_w1(LI_SUP, LT_ROUTER));
  for (uint32_t w = 0; w < N_WORKERS; w++) {
    workers[w] = LI_WORKER0 + w;
    ring_emit(r0, workers[w], OBS_EK_LOCUS_BIRTH, 0, obs_birth_w1(LI_SUP, LT_WORKER));
  }
  ring_emit(r0, B_ORDERS_UNIX, OBS_EK_BINDING_UP, 0, 0);

  fprintf(stderr, "synth: pid=%d shm=%s rate=%llu rings=%u slots=%u loss=%uppm\n",
          (int)getpid(), shm_name, (unsigned long long)cfg.rate,
          cfg.rings, cfg.slots, cfg.loss_ppm);
  fprintf(stderr, "synth: registered at %s (SIGUSR1 = dump, Ctrl-C = exit)\n", reg_path);

  pthread_t *tids = calloc(cfg.rings, sizeof *tids);
  for (uint32_t i = 0; i < cfg.rings; i++)
    pthread_create(&tids[i], NULL, producer_thread, &RC[i]);
  for (uint32_t i = 0; i < cfg.rings; i++)
    pthread_join(tids[i], NULL);

  /* --- cleanup --- */
  atomic_store(&H->flags, 0);
  unlink(reg_path);
  munmap(seg, seg_len);
  shm_unlink(shm_name);
  fprintf(stderr, "synth: clean exit\n");
  return 0;
}
