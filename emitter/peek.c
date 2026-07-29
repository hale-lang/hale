/* peek — minimal protocol consumer / smoke tool.
 *
 * Attaches to a synth (or any protocol-v0) segment, decodes
 * the manifest and rings, and streams human-readable events.
 * Tracks NET seq gaps per binding and reports loss. This is a
 * verification harness, not the consumer library — but its
 * attach/decode/merge shape is the library's seed.
 *
 *   ./peek                  # newest registration in $XDG_RUNTIME_DIR/hale
 *   ./peek <pid>            # specific registration
 *   ./peek --summary-only   # counters + loss, no event stream
 */
#define _GNU_SOURCE
#include "protocol.h"

#include <dirent.h>
#include <fcntl.h>
#include <inttypes.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static volatile sig_atomic_t running = 1;
static void on_int(int s) { (void)s; running = 0; }

/* ---- attached segment -------------------------------------- */

static void *seg;
static size_t seg_len;
static const obs_header *H;
static obs_control *CTRL;
static const obs_manifest_hdr *MH;
static const obs_manifest_entry *ME;
static const char *POOL;
static const obs_counter_line *CNT;
static const obs_ring_desc *RD;

/* name cache: (kind,id) -> string */
#define NAME_MAX_ID 4096
static char *names[4][NAME_MAX_ID];
static uint64_t seen_gen;

static void rescan_manifest(void) {
  uint32_t n = atomic_load_explicit((_Atomic uint32_t *)&MH->entry_count,
                                    memory_order_acquire);
  for (uint32_t i = 0; i < n; i++) {
    const obs_manifest_entry *e = &ME[i];
    if (e->kind > 3 || e->id >= NAME_MAX_ID) continue;
    if (!names[e->kind][e->id]) {
      char *s = malloc(e->name_len + 1);
      memcpy(s, POOL + e->name_off, e->name_len);
      s[e->name_len] = 0;
      names[e->kind][e->id] = s;
    }
  }
}

static const char *nm(int kind, uint32_t id) {
  if (id < NAME_MAX_ID && names[kind][id]) return names[kind][id];
  static char buf[32];
  snprintf(buf, sizeof buf, "unknown:%u", id);
  return buf;
}

/* locus instance -> type mapping learned from births */
static uint32_t inst_type[NAME_MAX_ID];
static const char *inst_nm(uint32_t id) {
  static char bufs[2][64];   /* rotate: safe for two uses per printf */
  static int which;
  char *buf = bufs[which ^= 1];
  if (id < NAME_MAX_ID && inst_type[id])
    snprintf(buf, 64, "%s#%u", nm(OBS_MK_LOCUS_TYPE, inst_type[id]), id);
  else
    snprintf(buf, 64, "locus#%u", id);
  return buf;
}

/* ---- per-ring consumer state ------------------------------- */

typedef struct {
  const obs_record *slots;
  uint64_t cursor;
  uint64_t epoch_ns;   /* from EPOCH records */
  uint64_t overruns;
  /* one decoded-but-unprinted record for k-way merge */
  int have;
  uint64_t ts, w0, w1;
} rstate;

static rstate *RS;

/* pull next raw record from ring i into RS[i]; returns 0 if none */
static int pull(uint32_t i) {
  rstate *s = &RS[i];
  if (s->have) return 1;
  const obs_ring_desc *d = &RD[i];
  for (;;) {
    uint64_t h1 = atomic_load_explicit((_Atomic uint64_t *)&d->head,
                                       memory_order_acquire);
    if (s->cursor >= h1) return 0;
    /* Live window given published head h is (h - ring_slots, h]
     * — the in-flight record h clobbers index h - ring_slots, so
     * the boundary is <=, not < (hale#244 finding 1); the full
     * cursor jump is counted as overrun (finding 2). */
    if (h1 > (uint64_t)H->ring_slots && s->cursor <= h1 - H->ring_slots) {
      uint64_t nc = h1 - H->ring_slots + 1;
      s->overruns += nc - s->cursor;
      s->cursor = nc;
    }
    obs_record rec = s->slots[s->cursor & (H->ring_slots - 1)];
    /* Consumer acquire fence before the h2 re-read (hale#244
     * finding 3, Boehm seqlock pair): observing any of record
     * h's bytes then forces h2 >= h, placing h - ring_slots
     * inside the <= discard window below. Required of every
     * external reader by PROTOCOL §10. */
    atomic_thread_fence(memory_order_acquire);
    uint64_t h2 = atomic_load_explicit((_Atomic uint64_t *)&d->head,
                                       memory_order_acquire);
    if (h2 > (uint64_t)H->ring_slots && s->cursor <= h2 - H->ring_slots) {
      uint64_t nc = h2 - H->ring_slots + 1;
      s->overruns += nc - s->cursor;
      s->cursor = nc;
      continue;
    }
    s->cursor++;
    uint32_t ek = obs_w0_ekind(rec.word0);
    if (ek == OBS_EK_EPOCH) { s->epoch_ns = rec.word1; continue; }
    s->ts = s->epoch_ns + (obs_w0_ts_delta(rec.word0) << H->ts_shift);
    s->w0 = rec.word0; s->w1 = rec.word1; s->have = 1;
    return 1;
  }
}

/* ---- loss tracking ----------------------------------------- */

/* Reorder-tolerant loss estimate: sends interleave across rings,
 * so merged order is not seq order (and real UDP reorders too).
 * Estimate = observed seq span minus delivers seen in that span. */
#define MAX_BINDINGS 64
static uint64_t seq_min[MAX_BINDINGS], seq_max[MAX_BINDINGS];
static uint64_t dlv_seen[MAX_BINDINGS];
static int seq_seen[MAX_BINDINGS];

static void track_deliver_seq(uint32_t b, uint64_t seq) {
  if (b >= MAX_BINDINGS) return;
  if (!seq_seen[b]) { seq_min[b] = seq_max[b] = seq; seq_seen[b] = 1; }
  if (seq < seq_min[b]) seq_min[b] = seq;
  if (seq > seq_max[b]) seq_max[b] = seq;
  dlv_seen[b]++;
}

/* ---- printing ---------------------------------------------- */

static void print_event(uint64_t ts, uint64_t w0, uint64_t w1) {
  uint32_t id = obs_w0_id(w0), ek = obs_w0_ekind(w0);
  uint32_t sc = obs_w0_size_class(w0);
  double t = (double)(ts - H->started_mono_ns) / 1e9;
  switch (ek) {
  case OBS_EK_BUS_PUBLISH:
    printf("%10.6f  pub   %-14s ~%uB locus=%u\n", t, nm(OBS_MK_TOPIC, id), 1u << sc,
           (unsigned)obs_bus_locus(w1)); break;
  case OBS_EK_BUS_DELIVER:
    printf("%10.6f  dlv   %-14s locus=%u\n", t, nm(OBS_MK_TOPIC, id),
           (unsigned)obs_bus_locus(w1)); break;
  case OBS_EK_NET_SEND:
    printf("%10.6f  net>  %-14s %s seq=%" PRIu64 "\n", t, nm(OBS_MK_TOPIC, id),
           nm(OBS_MK_BINDING, obs_net_binding(w1)), obs_net_seq(w1)); break;
  case OBS_EK_NET_DELIVER:
    track_deliver_seq(obs_net_binding(w1), obs_net_seq(w1));
    printf("%10.6f  net<  %-14s %s seq=%" PRIu64 "\n", t, nm(OBS_MK_TOPIC, id),
           nm(OBS_MK_BINDING, obs_net_binding(w1)), obs_net_seq(w1)); break;
  case OBS_EK_LOCUS_BIRTH:
    if (id < NAME_MAX_ID) inst_type[id] = obs_birth_type(w1);
    printf("%10.6f  birth %s parent=%s\n", t, inst_nm(id),
           obs_birth_parent(w1) ? inst_nm(obs_birth_parent(w1)) : "(root)");
    break;
  case OBS_EK_LOCUS_DISSOLVE:
    printf("%10.6f  wilt  %s reason=%" PRIu64 "\n", t, inst_nm(id), w1); break;
  case OBS_EK_RESTART:
    printf("%10.6f  restart %s attempt=%" PRIu64 "\n", t, inst_nm(id), w1 & 0xFFFF); break;
  case OBS_EK_SUPERV_TRANS:
    printf("%10.6f  superv %s transition=%" PRIu64 "\n", t, inst_nm(id), w1); break;
  case OBS_EK_BINDING_UP:
    printf("%10.6f  bind+ %s\n", t, nm(OBS_MK_BINDING, id)); break;
  case OBS_EK_BINDING_DOWN:
    printf("%10.6f  bind- %s err=%" PRIu64 "\n", t, nm(OBS_MK_BINDING, id), w1); break;
  default:
    printf("%10.6f  ek=%u id=%u w1=%" PRIx64 "\n", t, ek, id, w1);
  }
}

static void print_summary(void) {
  uint32_t n = atomic_load((_Atomic uint32_t *)&MH->entry_count);
  uint32_t line = 1;
  fprintf(stderr, "\n---- counters ----\n");
  for (uint32_t i = 0; i < n; i++) {
    if (ME[i].kind == OBS_MK_TOPIC) {
      const obs_counter_line *c = &CNT[line++];
      fprintf(stderr, "topic %-14s pub=%" PRIu64 " dlv=%" PRIu64 " bytes=%" PRIu64 "\n",
              nm(OBS_MK_TOPIC, ME[i].id),
              atomic_load(&c->c[OBS_CT_PUBLISHED]),
              atomic_load(&c->c[OBS_CT_DELIVERED]),
              atomic_load(&c->c[OBS_CT_BYTES]));
    } else if (ME[i].kind == OBS_MK_BINDING) {
      const obs_counter_line *c = &CNT[line++];
      uint64_t sent = atomic_load(&c->c[OBS_CB_SENT]);
      uint64_t dlv = atomic_load(&c->c[OBS_CB_DELIVERED]);
      uint32_t b = ME[i].id;
      uint64_t est = (b < MAX_BINDINGS && seq_seen[b])
        ? (seq_max[b] - seq_min[b] + 1) - dlv_seen[b] : 0;
      fprintf(stderr, "bind  %-22s sent=%" PRIu64 " dlv=%" PRIu64 " LOST=%" PRIu64
              " (seq-estimated loss in observed span: %" PRIu64 ")\n",
              nm(OBS_MK_BINDING, b), sent, dlv, sent - dlv, est);
    }
  }
  uint64_t overruns = 0;
  for (uint32_t i = 0; i < H->ring_count; i++) overruns += RS[i].overruns;
  fprintf(stderr, "records_total=%" PRIu64 " consumer_overruns=%" PRIu64 "\n",
          atomic_load((_Atomic uint64_t *)&CNT[0].c[OBS_CG_RECORDS_TOTAL]), overruns);
}

/* ---- attach ------------------------------------------------ */

static int find_registration(char *out, size_t outlen, const char *pid_arg) {
  const char *xdg = getenv("XDG_RUNTIME_DIR");
  char dir[192];
  if (xdg) snprintf(dir, sizeof dir, OBS_REG_DIR_FMT, xdg);
  else snprintf(dir, sizeof dir, "/tmp/hale-obs");
  if (pid_arg) { snprintf(out, outlen, "%s/%s.json", dir, pid_arg); return 0; }
  DIR *d = opendir(dir);
  if (!d) return -1;
  struct dirent *e; time_t best = 0; int found = -1;
  while ((e = readdir(d))) {
    if (!strstr(e->d_name, ".json")) continue;
    char p[512]; snprintf(p, sizeof p, "%s/%s", dir, e->d_name);
    struct stat st;
    if (stat(p, &st) == 0 && st.st_mtime >= best) {
      best = st.st_mtime; snprintf(out, outlen, "%.*s", (int)outlen - 1, p); found = 0;
    }
  }
  closedir(d);
  return found;
}

int main(int argc, char **argv) {
  const char *pid_arg = NULL;
  int summary_only = 0;
  for (int i = 1; i < argc; i++) {
    if (!strcmp(argv[i], "--summary-only")) summary_only = 1;
    else pid_arg = argv[i];
  }

  char reg[256];
  if (find_registration(reg, sizeof reg, pid_arg) < 0) {
    fprintf(stderr, "peek: no registrations found\n"); return 1;
  }
  /* crude parse: find "shm": "..." */
  char shm[128] = {0};
  {
    FILE *f = fopen(reg, "r");
    if (!f) { perror(reg); return 1; }
    char buf[1024]; size_t n = fread(buf, 1, sizeof buf - 1, f);
    buf[n] = 0; fclose(f);
    char *p = strstr(buf, "\"shm\": \"");
    if (!p) { fprintf(stderr, "peek: bad registration %s\n", reg); return 1; }
    p += 8;
    char *q = strchr(p, '"');
    if (!q || (size_t)(q - p) >= sizeof shm) { fprintf(stderr, "peek: bad shm name\n"); return 1; }
    memcpy(shm, p, q - p);
  }

  int fd = shm_open(shm, O_RDWR, 0); /* RW for the control page; the
    consumer library proper does a split RO/RW mapping (PROTOCOL §5) */
  if (fd < 0) { perror(shm); return 1; }
  struct stat st; fstat(fd, &st);
  seg_len = (size_t)st.st_size;
  seg = mmap(NULL, seg_len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  close(fd);
  if (seg == MAP_FAILED) { perror("mmap"); return 1; }

  H = (const obs_header *)seg;
  if (H->magic != OBS_MAGIC) { fprintf(stderr, "peek: bad magic\n"); return 1; }
  if (H->proto_major != OBS_PROTO_MAJOR) {
    fprintf(stderr, "peek: protocol major mismatch (%u)\n", H->proto_major); return 1;
  }
  CTRL = (obs_control *)((char *)seg + H->control_off);
  MH = (const obs_manifest_hdr *)((char *)seg + H->manifest_off);
  ME = (const obs_manifest_entry *)((char *)MH + sizeof(obs_manifest_hdr));
  POOL = (const char *)MH + MH->pool_off;
  CNT = (const obs_counter_line *)((char *)seg + H->counters_off);
  RD = (const obs_ring_desc *)((char *)seg + H->rings_off);

  fprintf(stderr, "peek: attached %s pid=%u rings=%u slots=%u proto=%u.%u\n",
          shm, H->pid, H->ring_count, H->ring_slots,
          H->proto_major, H->proto_minor);

  rescan_manifest();
  seen_gen = atomic_load((_Atomic uint64_t *)&H->manifest_gen);

  RS = calloc(H->ring_count, sizeof *RS);
  for (uint32_t i = 0; i < H->ring_count; i++)
    RS[i].slots = (const obs_record *)((char *)seg + RD[i].data_off);

  signal(SIGINT, on_int);
  atomic_fetch_add(&CTRL->observer_count, 1); /* wake the emitter */

  while (running && (atomic_load((_Atomic uint64_t *)&H->flags) & OBS_FLAG_ALIVE)) {
    uint64_t gen = atomic_load_explicit((_Atomic uint64_t *)&H->manifest_gen,
                                        memory_order_acquire);
    if (gen != seen_gen) { rescan_manifest(); seen_gen = gen; }

    /* k-way merge by reconstructed timestamp */
    int emitted_any = 0;
    for (;;) {
      int best = -1;
      for (uint32_t i = 0; i < H->ring_count; i++)
        if (pull(i) && (best < 0 || RS[i].ts < RS[best].ts)) best = (int)i;
      if (best < 0) break;
      if (!summary_only) print_event(RS[best].ts, RS[best].w0, RS[best].w1);
      RS[best].have = 0;
      emitted_any = 1;
    }
    if (!emitted_any) {
      struct timespec ts = { 0, 2000000 }; /* 2 ms */
      nanosleep(&ts, NULL);
    }
  }

  atomic_fetch_sub(&CTRL->observer_count, 1);
  print_summary();
  munmap(seg, seg_len);
  return 0;
}
