#define _GNU_SOURCE
#include "obs_attach.h"

#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#define NAME_MAX_ID 4096

typedef struct {
  const obs_record *slots;
  uint64_t cursor;
  uint64_t epoch_ns;
  uint64_t overruns;
  int have;
  obs_ev pending;
} ring_state;

struct obs_seg {
  void *map;
  size_t map_len;
  const obs_header *H;
  obs_control *CTRL;
  const obs_manifest_hdr *MH;
  const obs_manifest_entry *ME;
  const char *POOL;
  const obs_counter_line *CNT;
  const obs_ring_desc *RD;
  ring_state *rs;
  uint64_t seen_gen;
  char *names[4][NAME_MAX_ID];
  uint64_t shapes[NAME_MAX_ID];
  int cnt_line[4][NAME_MAX_ID]; /* -1 = none */
  char exe[128]; /* from the registration file; "" if absent */
};

const char *obs_exe(const obs_seg *s) { return s->exe; }

static void rescan(obs_seg *s) {
  uint32_t n = atomic_load_explicit((_Atomic uint32_t *)&s->MH->entry_count,
                                    memory_order_acquire);
  int line = 1;
  for (uint32_t i = 0; i < n; i++) {
    const obs_manifest_entry *e = &s->ME[i];
    int is_counted = (e->kind == OBS_MK_TOPIC || e->kind == OBS_MK_BINDING);
    int this_line = is_counted ? line++ : -1;
    if (e->kind > 3 || e->id >= NAME_MAX_ID) continue;
    if (!s->names[e->kind][e->id]) {
      char *nm = malloc((size_t)e->name_len + 1);
      memcpy(nm, s->POOL + e->name_off, e->name_len);
      nm[e->name_len] = 0;
      s->names[e->kind][e->id] = nm;
      if (e->kind == OBS_MK_TOPIC) s->shapes[e->id] = e->shape_hash;
      if (is_counted) s->cnt_line[e->kind][e->id] = this_line;
    }
  }
}

obs_seg *obs_attach_reg(const char *reg_path, char *err, size_t errlen) {
  char shm[128] = {0};
  char exe[128] = {0};
  {
    FILE *f = fopen(reg_path, "r");
    if (!f) { snprintf(err, errlen, "open %s failed", reg_path); return NULL; }
    char buf[1024];
    size_t n = fread(buf, 1, sizeof buf - 1, f);
    buf[n] = 0;
    fclose(f);
    char *p = strstr(buf, "\"shm\": \"");
    char *q = p ? strchr(p + 8, '"') : NULL;
    if (!q || (size_t)(q - (p + 8)) >= sizeof shm) {
      snprintf(err, errlen, "bad registration %s", reg_path); return NULL;
    }
    memcpy(shm, p + 8, (size_t)(q - (p + 8)));
    /* optional exe field (present since proto 0.1's writer) */
    char *e = strstr(buf, "\"exe\": \"");
    char *eq = e ? strchr(e + 8, '"') : NULL;
    if (eq && (size_t)(eq - (e + 8)) < sizeof exe)
      memcpy(exe, e + 8, (size_t)(eq - (e + 8)));
  }

  int fd = shm_open(shm, O_RDWR, 0);
  if (fd < 0) { snprintf(err, errlen, "shm_open %s failed", shm); return NULL; }
  struct stat st;
  fstat(fd, &st);

  obs_seg *s = calloc(1, sizeof *s);
  s->map_len = (size_t)st.st_size;
  s->map = mmap(NULL, s->map_len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  close(fd);
  if (s->map == MAP_FAILED) { free(s); snprintf(err, errlen, "mmap failed"); return NULL; }

  s->H = (const obs_header *)s->map;
  if (s->H->magic != OBS_MAGIC || s->H->proto_major != OBS_PROTO_MAJOR) {
    munmap(s->map, s->map_len); free(s);
    snprintf(err, errlen, "bad magic/version in %s", shm);
    return NULL;
  }
  s->CTRL = (obs_control *)((char *)s->map + s->H->control_off);
  s->MH = (const obs_manifest_hdr *)((char *)s->map + s->H->manifest_off);
  s->ME = (const obs_manifest_entry *)((char *)s->MH + sizeof(obs_manifest_hdr));
  s->POOL = (const char *)s->MH + s->MH->pool_off;
  s->CNT = (const obs_counter_line *)((char *)s->map + s->H->counters_off);
  s->RD = (const obs_ring_desc *)((char *)s->map + s->H->rings_off);
  s->rs = calloc(s->H->ring_count, sizeof *s->rs);
  for (uint32_t i = 0; i < s->H->ring_count; i++)
    s->rs[i].slots = (const obs_record *)((char *)s->map + s->RD[i].data_off);
  for (int k = 0; k < 4; k++)
    for (int i = 0; i < NAME_MAX_ID; i++) s->cnt_line[k][i] = -1;
  snprintf(s->exe, sizeof s->exe, "%s", exe);
  rescan(s);
  s->seen_gen = atomic_load((_Atomic uint64_t *)&s->H->manifest_gen);
  atomic_fetch_add(&s->CTRL->observer_count, 1);
  return s;
}

void obs_detach(obs_seg *s) {
  if (!s) return;
  atomic_fetch_sub(&s->CTRL->observer_count, 1);
  for (int k = 0; k < 4; k++)
    for (int i = 0; i < NAME_MAX_ID; i++) free(s->names[k][i]);
  free(s->rs);
  munmap(s->map, s->map_len);
  free(s);
}

int obs_alive(const obs_seg *s) {
  return (atomic_load((_Atomic uint64_t *)&s->H->flags) & OBS_FLAG_ALIVE) != 0;
}
uint32_t obs_pid(const obs_seg *s) { return s->H->pid; }
uint64_t obs_started_mono(const obs_seg *s) { return s->H->started_mono_ns; }
uint64_t obs_overruns(const obs_seg *s) {
  uint64_t o = 0;
  for (uint32_t i = 0; i < s->H->ring_count; i++) o += s->rs[i].overruns;
  return o;
}

static int pull(obs_seg *s, uint32_t i) {
  ring_state *r = &s->rs[i];
  if (r->have) return 1;
  const obs_ring_desc *d = &s->RD[i];
  uint32_t slots = s->H->ring_slots;
  for (;;) {
    uint64_t h1 = atomic_load_explicit((_Atomic uint64_t *)&d->head,
                                       memory_order_acquire);
    if (r->cursor >= h1) return 0;
    /* Live window given published head h is (h - ring_slots, h]
     * — the in-flight record h clobbers index h - ring_slots, so
     * the boundary is <=, not < (hale#244 finding 1); the full
     * cursor jump is counted as overrun (finding 2). */
    if (h1 > slots && r->cursor <= h1 - slots) {
      uint64_t nc = h1 - slots + 1;
      r->overruns += nc - r->cursor;
      r->cursor = nc;
    }
    obs_record rec = r->slots[r->cursor & (slots - 1)];
    /* Consumer acquire fence before the h2 re-read (hale#244
     * finding 3, Boehm seqlock pair): observing any of record
     * h's bytes then forces h2 >= h, placing h - ring_slots
     * inside the <= discard window below. Required of every
     * external reader by PROTOCOL §10. */
    atomic_thread_fence(memory_order_acquire);
    uint64_t h2 = atomic_load_explicit((_Atomic uint64_t *)&d->head,
                                       memory_order_acquire);
    if (h2 > slots && r->cursor <= h2 - slots) {
      uint64_t nc = h2 - slots + 1;
      r->overruns += nc - r->cursor;
      r->cursor = nc;
      continue;
    }
    r->cursor++;
    uint32_t ek = obs_w0_ekind(rec.word0);
    if (ek == OBS_EK_EPOCH) { r->epoch_ns = rec.word1; continue; }
    r->pending = (obs_ev){
      .ts = r->epoch_ns + (obs_w0_ts_delta(rec.word0) << s->H->ts_shift),
      .ekind = ek,
      .id = obs_w0_id(rec.word0),
      .size_class = obs_w0_size_class(rec.word0),
      .w1 = rec.word1,
    };
    r->have = 1;
    return 1;
  }
}

int obs_next(obs_seg *s, obs_ev *out) {
  uint64_t gen = atomic_load_explicit((_Atomic uint64_t *)&s->H->manifest_gen,
                                      memory_order_acquire);
  if (gen != s->seen_gen) { rescan(s); s->seen_gen = gen; }
  int best = -1;
  for (uint32_t i = 0; i < s->H->ring_count; i++)
    if (pull(s, i) &&
        (best < 0 || s->rs[i].pending.ts < s->rs[best].pending.ts))
      best = (int)i;
  if (best < 0) return 0;
  *out = s->rs[best].pending;
  s->rs[best].have = 0;
  return 1;
}

uint32_t obs_entry_count(obs_seg *s) {
  return atomic_load((_Atomic uint32_t *)&s->MH->entry_count);
}
const obs_manifest_entry *obs_entry(obs_seg *s, uint32_t i) { return &s->ME[i]; }

const char *obs_name(obs_seg *s, int kind, uint32_t id) {
  if (kind < 0 || kind > 3 || id >= NAME_MAX_ID) return NULL;
  return s->names[kind][id];
}
uint64_t obs_shape(obs_seg *s, uint32_t topic_id) {
  return topic_id < NAME_MAX_ID ? s->shapes[topic_id] : 0;
}
int obs_counter_line_of(obs_seg *s, int kind, uint32_t id) {
  if (kind < 0 || kind > 3 || id >= NAME_MAX_ID) return -1;
  return s->cnt_line[kind][id];
}
uint64_t obs_counter(const obs_seg *s, int line, int cell) {
  if (line < 0 || cell < 0 || cell > 7) return 0;
  return atomic_load_explicit((_Atomic uint64_t *)&s->CNT[line].c[cell],
                              memory_order_relaxed);
}
