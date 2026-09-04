/* fuse-hl/glue.c — FFI substrate for the Hale fuse.
 *
 * Thin scalar wrapper over obs_attach (the consumer attach/
 * decode library): attach handles, a latched event cursor, and
 * arena-copied name/shape strings. NO fusion logic lives here —
 * topic join, seq matching, the locus table, JSON, and serving
 * are pure Hale in main.hl. Same division as observe/glue.c on
 * the emitter side: C only where mmap'd shm can't be Hale yet.
 *
 * ABI per spec/ffi.md: Int -> int64_t, String -> const char*
 * (arena-copied on return so pointers outlive this frame).
 */
#define _GNU_SOURCE
#include "../../obs_attach.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

/* runtime arena (spec/ffi.md § returning heap Strings) */
typedef struct lotus_arena lotus_arena_t;
extern lotus_arena_t *lotus_caller_arena_or_global(void);
extern void *lotus_arena_alloc(lotus_arena_t *a, uint64_t size, uint64_t align);

#define FZ_MAX_SEGS 16

static obs_seg *segs[FZ_MAX_SEGS];
static obs_ev cur; /* latched by fz_next */

static const char *astr(const char *s) {
  if (!s) s = "";
  size_t n = strlen(s) + 1;
  char *p = lotus_arena_alloc(lotus_caller_arena_or_global(), n, 1);
  if (!p) return "";
  memcpy(p, s, n);
  return p;
}

int64_t fz_attach(const char *reg_path) {
  for (int i = 0; i < FZ_MAX_SEGS; i++) {
    if (segs[i]) continue;
    char err[128];
    obs_seg *s = obs_attach_reg(reg_path, err, sizeof err);
    if (!s) {
      fprintf(stderr, "fuse-hl: attach %s failed: %s\n", reg_path, err);
      return -1;
    }
    segs[i] = s;
    return i;
  }
  return -1;
}

void fz_detach(int64_t h) {
  if (h < 0 || h >= FZ_MAX_SEGS || !segs[h]) return;
  obs_detach(segs[h]);
  segs[h] = NULL;
}

static obs_seg *seg(int64_t h) {
  return (h >= 0 && h < FZ_MAX_SEGS) ? segs[h] : NULL;
}

int64_t fz_alive(int64_t h)    { obs_seg *s = seg(h); return s ? obs_alive(s) : 0; }
int64_t fz_pid(int64_t h)      { obs_seg *s = seg(h); return s ? obs_pid(s) : 0; }
int64_t fz_overruns(int64_t h) { obs_seg *s = seg(h); return s ? (int64_t)obs_overruns(s) : 0; }

/* merged event pull: 1 = event latched, 0 = nothing pending */
int64_t fz_next(int64_t h) {
  obs_seg *s = seg(h);
  return (s && obs_next(s, &cur)) ? 1 : 0;
}
int64_t fz_ev_ts(void)   { return (int64_t)cur.ts; }
int64_t fz_ev_kind(void) { return (int64_t)cur.ekind; }
int64_t fz_ev_id(void)   { return (int64_t)cur.id; }
int64_t fz_ev_seq(void)  { return (int64_t)obs_net_seq(cur.w1); }
int64_t fz_ev_birth_parent(void) { return (int64_t)obs_birth_parent(cur.w1); }
int64_t fz_ev_birth_type(void)   { return (int64_t)obs_birth_type(cur.w1); }
int64_t fz_ev_w1(void)   { return (int64_t)cur.w1; }
int64_t fz_ev_bus_locus(void) { return (int64_t)obs_bus_locus(cur.w1); }

/* ---- batch drain (2026-07-27) ------------------------------
 * One FFI call drains up to cap events into the caller's Bytes
 * payload as packed 32-byte records:
 *   [0] u64 ts   [8] u32 ekind   [12] u32 id
 *   [16] u64 w1  [24] u32 size_class  [28] pad
 * Replaces the per-event fz_next + field-getter round-trips,
 * which lost to the emitters' full-rate firehose (dropped
 * DISSOLVE events -> zombie loci). */
extern int64_t lotus_bytes_len(const void *b);
extern void *lotus_bytes_data(void *b);

int64_t fz_next_batch(int64_t h, void *buf) {
  obs_seg *s = seg(h);
  if (!s || !buf) return 0;
  uint8_t *d = lotus_bytes_data(buf);
  int64_t cap = lotus_bytes_len(buf) / 32;
  int64_t n = 0;
  obs_ev ev;
  while (n < cap && obs_next(s, &ev)) {
    uint8_t *r = d + n * 32;
    memcpy(r, &ev.ts, 8);
    memcpy(r + 8, &ev.ekind, 4);
    memcpy(r + 12, &ev.id, 4);
    memcpy(r + 16, &ev.w1, 8);
    memcpy(r + 24, &ev.size_class, 4);
    memset(r + 28, 0, 4);
    n++;
  }
  return n;
}

/* manifest */
int64_t fz_entry_count(int64_t h) { obs_seg *s = seg(h); return s ? obs_entry_count(s) : 0; }
int64_t fz_entry_kind(int64_t h, int64_t i) {
  obs_seg *s = seg(h);
  const obs_manifest_entry *e = s ? obs_entry(s, (uint32_t)i) : NULL;
  return e ? e->kind : -1;
}
int64_t fz_entry_id(int64_t h, int64_t i) {
  obs_seg *s = seg(h);
  const obs_manifest_entry *e = s ? obs_entry(s, (uint32_t)i) : NULL;
  return e ? e->id : -1;
}
const char *fz_entry_shape_hex(int64_t h, int64_t i) {
  obs_seg *s = seg(h);
  const obs_manifest_entry *e = s ? obs_entry(s, (uint32_t)i) : NULL;
  char b[24];
  if (!e) return astr("");
  snprintf(b, sizeof b, "%016llx", (unsigned long long)e->shape_hash);
  return astr(b);
}
/* Model identity as hex, so it compares directly against the
 * topology artifact's `shape_hash` string with no Int width games
 * on the Hale side. "" means the emitter predates proto 0.2 and
 * said nothing — distinct from "0000000000000000", which is an
 * emitter that positively has no model (a synthetic harness). */
const char *fz_model_hash_hex(int64_t h) {
  obs_seg *s = seg(h);
  uint64_t m = 0;
  char b[24];
  if (!s || !obs_model_hash(s, &m)) return astr("");
  snprintf(b, sizeof b, "%016llx", (unsigned long long)m);
  return astr(b);
}
const char *fz_exe(int64_t h) {
  obs_seg *s = seg(h);
  return astr(s ? obs_exe(s) : "");
}
const char *fz_name(int64_t h, int64_t kind, int64_t id) {
  obs_seg *s = seg(h);
  return astr(s ? obs_name(s, (int)kind, (uint32_t)id) : "");
}

/* counters */
int64_t fz_counter_line(int64_t h, int64_t kind, int64_t id) {
  obs_seg *s = seg(h);
  return s ? obs_counter_line_of(s, (int)kind, (uint32_t)id) : -1;
}
int64_t fz_counter(int64_t h, int64_t line, int64_t cell) {
  obs_seg *s = seg(h);
  return s ? (int64_t)obs_counter(s, (int)line, (int)cell) : 0;
}

/* misc */
int64_t fz_now(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (int64_t)ts.tv_sec * 1000000000ll + ts.tv_nsec;
}
void fz_unlink(const char *path) { unlink(path); }

/* SSE guard: a stalled client must cost the fusion loop at most
 * this long per pump, not stall it forever (Stream.send blocks). */
void fz_set_sndtimeo(int64_t fd, int64_t ms) {
  struct timeval tv = { .tv_sec = ms / 1000, .tv_usec = (ms % 1000) * 1000 };
  setsockopt((int)fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
}

