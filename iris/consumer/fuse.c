/* fuse — multi-segment fusion: N processes, one system view.
 *
 * Discovers every live registration, attaches, joins topics
 * across segments by (shape_hash, name), matches NET_SEND ->
 * NET_DELIVER pairs across processes by (topic key, seq) to
 * derive cross-process edges with real latency, and renders a
 * top-style fused view once per second.
 *
 *   ./fuse            # attach everything, refresh until Ctrl-C
 *   ./fuse --frames N # render N frames then exit (for tests)
 */
#define _GNU_SOURCE
#include "obs_attach.h"

#include <dirent.h>
#include <inttypes.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define MAX_SEGS 16
#define MAX_TOPICS 64
#define SEQ_SLOTS 65536 /* per-topic in-flight seq match window */
#define EV_TAIL 8

static volatile sig_atomic_t running = 1;
static void on_int(int sig) { (void)sig; running = 0; }

/* ---- attached segments ------------------------------------- */

typedef struct {
  obs_seg *s;
  char reg[256];
  uint32_t pid;
  int dead;
  /* per-seg locus naming: instance -> type id, learned from births */
  uint32_t inst_type[4096];
  uint64_t restarts;
} seg_slot;

static seg_slot segs[MAX_SEGS];
static int nsegs;

/* ---- fused topic table ------------------------------------- */

typedef struct { uint64_t seq; uint64_t ts; int seg; int kind; } seq_ent; /* kind: 0 empty, 1 send, 2 dlv */

typedef struct {
  uint64_t shape;
  char name[64];
  int local_id[MAX_SEGS]; /* per-seg topic id, -1 if absent */
  /* directed edge stats [from][to] */
  uint64_t matched[MAX_SEGS][MAX_SEGS];
  uint64_t lat_sum[MAX_SEGS][MAX_SEGS];
  uint64_t lat_max[MAX_SEGS][MAX_SEGS];
  uint64_t sends[MAX_SEGS], delivers[MAX_SEGS];
  seq_ent *inflight; /* lazy alloc */
} topic_row;

static topic_row topics[MAX_TOPICS];
static int ntopics;

static topic_row *topic_for(uint64_t shape, const char *name) {
  for (int i = 0; i < ntopics; i++)
    if (topics[i].shape == shape && !strcmp(topics[i].name, name))
      return &topics[i];
  if (ntopics >= MAX_TOPICS) return NULL;
  topic_row *t = &topics[ntopics++];
  memset(t, 0, sizeof *t);
  t->shape = shape;
  snprintf(t->name, sizeof t->name, "%s", name);
  for (int i = 0; i < MAX_SEGS; i++) t->local_id[i] = -1;
  return t;
}

/* per-seg local topic id -> fused row (rebuilt on manifest growth) */
static topic_row *local_topic[MAX_SEGS][4096];

static void index_topics(int si) {
  obs_seg *s = segs[si].s;
  uint32_t n = obs_entry_count(s);
  for (uint32_t i = 0; i < n; i++) {
    const obs_manifest_entry *e = obs_entry(s, i);
    if (e->kind != OBS_MK_TOPIC || e->id >= 4096) continue;
    if (local_topic[si][e->id]) continue;
    const char *nm = obs_name(s, OBS_MK_TOPIC, e->id);
    if (!nm) continue; /* entry visible before name rescan; retry next pass */
    topic_row *t = topic_for(e->shape_hash, nm);
    if (!t) continue;
    t->local_id[si] = (int)e->id;
    local_topic[si][e->id] = t;
  }
}

/* ---- structural event tail --------------------------------- */

static char ev_tail[EV_TAIL][96];
static int ev_head;
static void tail_push(const char *line) {
  snprintf(ev_tail[ev_head++ % EV_TAIL], 96, "%s", line);
}

/* ---- event handling ---------------------------------------- */

static void handle_event(int si, const obs_ev *ev) {
  seg_slot *g = &segs[si];
  obs_seg *s = g->s;
  char line[96];
  switch (ev->ekind) {
  /* Send/deliver matching is ORDER-INDEPENDENT: segments are
   * drained in attach order, so within one poll cycle the
   * consumer's deliver can be processed before the producer's
   * send for the same seq. Whichever side lands first is
   * stored; the second completes the pair. */
  case OBS_EK_NET_SEND: {
    topic_row *t = ev->id < 4096 ? local_topic[si][ev->id] : NULL;
    if (!t) break;
    if (!t->inflight) t->inflight = calloc(SEQ_SLOTS, sizeof(seq_ent));
    uint64_t seq = obs_net_seq(ev->w1);
    seq_ent *e = &t->inflight[seq & (SEQ_SLOTS - 1)];
    t->sends[si]++;
    if (e->kind == 2 && e->seq == seq && e->seg != si) {
      uint64_t lat = e->ts > ev->ts ? e->ts - ev->ts : 0;
      t->matched[si][e->seg]++;
      t->lat_sum[si][e->seg] += lat;
      if (lat > t->lat_max[si][e->seg]) t->lat_max[si][e->seg] = lat;
      e->kind = 0;
    } else {
      e->seq = seq; e->ts = ev->ts; e->seg = si; e->kind = 1;
    }
    break;
  }
  case OBS_EK_NET_DELIVER: {
    topic_row *t = ev->id < 4096 ? local_topic[si][ev->id] : NULL;
    if (!t) break;
    t->delivers[si]++;
    if (!t->inflight) t->inflight = calloc(SEQ_SLOTS, sizeof(seq_ent));
    uint64_t seq = obs_net_seq(ev->w1);
    seq_ent *e = &t->inflight[seq & (SEQ_SLOTS - 1)];
    if (e->kind == 1 && e->seq == seq && e->seg != si) {
      uint64_t lat = ev->ts > e->ts ? ev->ts - e->ts : 0;
      t->matched[e->seg][si]++;
      t->lat_sum[e->seg][si] += lat;
      if (lat > t->lat_max[e->seg][si]) t->lat_max[e->seg][si] = lat;
      e->kind = 0;
    } else {
      e->seq = seq; e->ts = ev->ts; e->seg = si; e->kind = 2;
    }
    break;
  }
  case OBS_EK_LOCUS_BIRTH:
    if (ev->id < 4096) g->inst_type[ev->id] = obs_birth_type(ev->w1);
    break;
  case OBS_EK_LOCUS_DISSOLVE: {
    const char *tn = ev->id < 4096
      ? obs_name(s, OBS_MK_LOCUS_TYPE, g->inst_type[ev->id]) : NULL;
    snprintf(line, sizeof line, "pid %u: %s#%u dissolved (reason %" PRIu64 ")",
             g->pid, tn ? tn : "locus", ev->id, ev->w1);
    tail_push(line);
    break;
  }
  case OBS_EK_RESTART:
    g->restarts++;
    snprintf(line, sizeof line, "pid %u: restart -> locus#%u (attempt %" PRIu64 ")",
             g->pid, ev->id, ev->w1 & 0xFFFF);
    tail_push(line);
    break;
  case OBS_EK_BINDING_DOWN:
    snprintf(line, sizeof line, "pid %u: binding %s DOWN err=%" PRIu64, g->pid,
             obs_name(s, OBS_MK_BINDING, ev->id), ev->w1);
    tail_push(line);
    break;
  default: break;
  }
}

/* ---- discovery --------------------------------------------- */

static void discover(void) {
  const char *xdg = getenv("XDG_RUNTIME_DIR");
  char dir[192];
  if (xdg) snprintf(dir, sizeof dir, OBS_REG_DIR_FMT, xdg);
  else snprintf(dir, sizeof dir, "/tmp/hale-obs");
  DIR *d = opendir(dir);
  if (!d) return;
  struct dirent *e;
  while ((e = readdir(d))) {
    if (!strstr(e->d_name, ".json")) continue;
    char p[512];
    snprintf(p, sizeof p, "%s/%s", dir, e->d_name);
    int known = 0;
    for (int i = 0; i < nsegs; i++)
      if (!strncmp(segs[i].reg, p, sizeof segs[i].reg)) { known = 1; break; }
    if (known || nsegs >= MAX_SEGS) continue;
    /* stale-registration GC (PROTOCOL Â§1): dead pid => remove
     * the file and its leaked segment, attach nothing */
    {
      long pid = atol(e->d_name);
      char proc[64];
      snprintf(proc, sizeof proc, "/proc/%ld", pid);
      struct stat pst;
      if (pid > 0 && stat(proc, &pst) != 0) {
        char shmp[128];
        snprintf(shmp, sizeof shmp, "/dev/shm/hale-obs-%ld", pid);
        unlink(p);
        unlink(shmp);
        continue;
      }
    }
    char err[128];
    obs_seg *s = obs_attach_reg(p, err, sizeof err);
    if (!s) continue;
    seg_slot *g = &segs[nsegs];
    memset(g->inst_type, 0, sizeof g->inst_type);
    g->s = s;
    g->pid = obs_pid(s);
    g->dead = 0;
    g->restarts = 0;
    snprintf(g->reg, sizeof g->reg, "%.*s", (int)sizeof g->reg - 1, p);
    index_topics(nsegs);
    nsegs++;
    fprintf(stderr, "fuse: attached pid %u (%s)\n", g->pid, p);
  }
  closedir(d);
}

/* ---- rendering --------------------------------------------- */

static uint64_t prev_matched[MAX_TOPICS][MAX_SEGS][MAX_SEGS];

static void render(int frame) {
  printf("\033[2J\033[H");
  printf("iris fuse — %d segment%s, frame %d\n\n", nsegs, nsegs == 1 ? "" : "s", frame);

  printf("PROCESSES\n");
  printf("  %-8s %-6s %-12s %-10s %-8s\n", "pid", "state", "records", "overruns", "restarts");
  for (int i = 0; i < nsegs; i++) {
    seg_slot *g = &segs[i];
    printf("  %-8u %-6s %-12" PRIu64 " %-10" PRIu64 " %-8" PRIu64 "\n",
           g->pid, g->dead ? "dead" : "live",
           obs_counter(g->s, 0, OBS_CG_RECORDS_TOTAL),
           obs_overruns(g->s), g->restarts);
  }

  printf("\nTOPICS (fused on shape_hash)\n");
  printf("  %-16s %-18s %-12s %-12s %s\n", "topic", "shape", "pub", "dlv", "segments");
  for (int i = 0; i < ntopics; i++) {
    topic_row *t = &topics[i];
    uint64_t pub = 0, dlv = 0;
    char where[64] = "";
    for (int si = 0; si < nsegs; si++) {
      if (t->local_id[si] < 0) continue;
      int line = obs_counter_line_of(segs[si].s, OBS_MK_TOPIC, (uint32_t)t->local_id[si]);
      pub += obs_counter(segs[si].s, line, OBS_CT_PUBLISHED);
      dlv += obs_counter(segs[si].s, line, OBS_CT_DELIVERED);
      char b[16];
      snprintf(b, sizeof b, "%s%u", where[0] ? "," : "", segs[si].pid);
      strncat(where, b, sizeof where - strlen(where) - 1);
    }
    printf("  %-16s %016" PRIx64 " %-12" PRIu64 " %-12" PRIu64 " %s\n",
           t->name, t->shape, pub, dlv, where);
  }

  printf("\nEDGES (cross-process, seq-matched)\n");
  printf("  %-16s %-14s %-10s %-12s %-12s %s\n",
         "topic", "edge", "rate/s", "lat mean", "lat max", "unmatched");
  int any = 0;
  for (int i = 0; i < ntopics; i++) {
    topic_row *t = &topics[i];
    for (int a = 0; a < nsegs; a++)
      for (int b = 0; b < nsegs; b++) {
        if (!t->matched[a][b]) continue;
        any = 1;
        uint64_t m = t->matched[a][b];
        uint64_t rate = m - prev_matched[i][a][b];
        prev_matched[i][a][b] = m;
        char edge[32];
        snprintf(edge, sizeof edge, "%u->%u", segs[a].pid, segs[b].pid);
        uint64_t un = t->sends[a] > t->delivers[b] ? t->sends[a] - t->delivers[b] : 0;
        printf("  %-16s %-14s %-10" PRIu64 " %8.1f us  %8.1f us  %" PRIu64 "\n",
               t->name, edge, rate,
               (double)t->lat_sum[a][b] / (double)m / 1000.0,
               (double)t->lat_max[a][b] / 1000.0, un);
      }
  }
  if (!any) printf("  (none yet)\n");

  printf("\nEVENTS\n");
  for (int i = 0; i < EV_TAIL; i++) {
    const char *l = ev_tail[(ev_head + i) % EV_TAIL];
    if (l[0]) printf("  %s\n", l);
  }
  fflush(stdout);
}

int main(int argc, char **argv) {
  int frames = -1;
  for (int i = 1; i < argc; i++)
    if (!strcmp(argv[i], "--frames") && i + 1 < argc) frames = atoi(argv[++i]);

  signal(SIGINT, on_int);
  uint64_t last_render = 0, last_discover = 0;
  int frame = 0;

  while (running) {
    struct timespec now_ts;
    clock_gettime(CLOCK_MONOTONIC, &now_ts);
    uint64_t now = (uint64_t)now_ts.tv_sec * 1000000000ull + (uint64_t)now_ts.tv_nsec;

    if (now - last_discover > 1000000000ull) { discover(); last_discover = now; }

    int worked = 0;
    for (int i = 0; i < nsegs; i++) {
      if (segs[i].dead) continue;
      index_topics(i); /* cheap; picks up late manifest entries */
      obs_ev ev;
      int budget = 200000;
      while (budget-- > 0 && obs_next(segs[i].s, &ev)) {
        handle_event(i, &ev);
        worked = 1;
      }
      if (!obs_alive(segs[i].s)) segs[i].dead = 1;
    }

    if (now - last_render > 1000000000ull && nsegs > 0) {
      render(++frame);
      last_render = now;
      if (frames > 0 && frame >= frames) break;
    }
    if (!worked) {
      struct timespec ts = { 0, 2000000 };
      nanosleep(&ts, NULL);
    }
  }

  for (int i = 0; i < nsegs; i++) obs_detach(segs[i].s);
  fprintf(stderr, "fuse: done (%d segments, %d topics)\n", nsegs, ntopics);
  return 0;
}
