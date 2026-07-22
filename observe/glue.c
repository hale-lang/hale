/* observe/glue.c — segment setup for the observation protocol.
 *
 * Self-contained (snapshot-friendly: no include of iris headers)
 * implementation of PROTOCOL.md's segment: header, control page,
 * manifest, mode mask, counter table, ring descriptors,
 * registration file. The HOT PATH does not live here — record
 * emission is pure Hale via std::ring::__spsc_* against the
 * descriptor/base pointers this glue hands out.
 *
 * One segment per process; state is static. All exported
 * functions use the hale @ffi("c") ABI: Int -> int64_t,
 * String -> const char*, Bool -> int32_t.
 */
#define _GNU_SOURCE
#include <fcntl.h>
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
#define PAGE 4096
#define ENTRY_CAP 256

/* header field offsets per PROTOCOL.md §3 */
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
} hdr_t;

typedef struct { _Atomic uint32_t observer_count, sample_n; } ctrl_t;
typedef struct { _Atomic uint32_t entry_count; uint32_t entry_cap, pool_off;
                 _Atomic uint32_t pool_used; } mh_t;
typedef struct { uint64_t shape_hash, aux_b; uint32_t id, name_off;
                 uint16_t name_len, aux_a; uint8_t kind, flags;
                 uint16_t _pad; } me_t;
typedef struct { _Atomic uint64_t c[8]; } cline_t;
typedef struct { uint64_t data_off; _Atomic uint64_t head, dropped;
                 uint32_t tag_a; _Atomic uint32_t tag_b;
                 uint8_t reserved[32]; } rdesc_t;

static void *seg; static size_t seg_len;
static hdr_t *H; static ctrl_t *C; static mh_t *MH; static me_t *ME;
static char *POOL; static uint8_t *MODE; static cline_t *CNT; static rdesc_t *RD;
static char shm_name[64], reg_path[280];
static int cnt_line_for[4][ENTRY_CAP]; /* kind x id -> counter line; -1 none */

static uint64_t mono_ns(void) {
  struct timespec ts; clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}
static uint64_t wall_ns(void) {
  struct timespec ts; clock_gettime(CLOCK_REALTIME, &ts);
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}
static size_t page_up(size_t n) { return (n + PAGE - 1) & ~((size_t)PAGE - 1); }

int64_t obs_create(int64_t rings, int64_t slots) {
  if (seg) return 0; /* one segment per process */
  if (rings < 1 || rings > 64 || slots < 64 || (slots & (slots - 1))) return 0;
  size_t manifest_len = page_up(sizeof(mh_t) + ENTRY_CAP * 32 + 8192);
  size_t modemask_len = page_up(ENTRY_CAP);
  size_t counters_len = page_up((1 + ENTRY_CAP) * 64);
  size_t rings_hdr = page_up((size_t)rings * sizeof(rdesc_t));
  size_t ring_bytes = (size_t)slots * 16;
  size_t off = 0;
  size_t control_off  = (off += PAGE);
  size_t manifest_off = (off += PAGE);
  size_t modemask_off = (off += manifest_len);
  size_t counters_off = (off += modemask_len);
  size_t rings_off    = (off += counters_len);
  seg_len = rings_off + rings_hdr + (size_t)rings * ring_bytes;

  snprintf(shm_name, sizeof shm_name, "/hale-obs-%d", (int)getpid());
  shm_unlink(shm_name);
  int fd = shm_open(shm_name, O_CREAT | O_EXCL | O_RDWR, 0600);
  if (fd < 0) return 0;
  if (ftruncate(fd, (off_t)seg_len) < 0) { close(fd); return 0; }
  seg = mmap(NULL, seg_len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  close(fd);
  if (seg == MAP_FAILED) { seg = NULL; return 0; }

  H = (hdr_t *)seg;
  C = (ctrl_t *)((char *)seg + control_off);
  MH = (mh_t *)((char *)seg + manifest_off);
  ME = (me_t *)((char *)MH + sizeof(mh_t));
  MH->entry_cap = ENTRY_CAP;
  MH->pool_off = (uint32_t)(sizeof(mh_t) + ENTRY_CAP * 32);
  POOL = (char *)MH + MH->pool_off;
  MODE = (uint8_t *)seg + modemask_off;
  CNT = (cline_t *)((char *)seg + counters_off);
  RD = (rdesc_t *)((char *)seg + rings_off);
  memset(MODE, 2 /* PACKED */, ENTRY_CAP);
  memset(cnt_line_for, -1, sizeof cnt_line_for);

  *H = (hdr_t){ .magic = OBS_MAGIC, .proto_major = 0, .proto_minor = 1,
    .header_len = sizeof(hdr_t), .total_len = seg_len,
    .pid = (uint32_t)getpid(), .ring_count = (uint32_t)rings,
    .ring_slots = (uint32_t)slots, .ts_shift = 4,
    .started_mono_ns = mono_ns(), .started_wall_ns = wall_ns(),
    .control_off = control_off, .manifest_off = manifest_off,
    .manifest_len = manifest_len, .modemask_off = modemask_off,
    .counters_off = counters_off, .counters_len = counters_len,
    .rings_off = rings_off };
  for (int64_t i = 0; i < rings; i++)
    RD[i] = (rdesc_t){ .data_off = rings_off + rings_hdr + (uint64_t)i * ring_bytes,
                       .tag_a = (uint32_t)i };
  atomic_store(&H->flags, 1);

  const char *xdg = getenv("XDG_RUNTIME_DIR");
  char dir[192];
  if (xdg) snprintf(dir, sizeof dir, "%s/hale", xdg);
  else snprintf(dir, sizeof dir, "/tmp/hale-obs");
  mkdir(dir, 0700);
  snprintf(reg_path, sizeof reg_path, "%s/%d.json", dir, (int)getpid());
  char tmp[300]; snprintf(tmp, sizeof tmp, "%s.tmp", reg_path);
  FILE *f = fopen(tmp, "w");
  if (f) {
    char exe[128] = "hale-app";
    ssize_t n = readlink("/proc/self/exe", exe, sizeof exe - 1);
    if (n > 0) exe[n] = 0;
    fprintf(f, "{\n  \"proto\": \"0.1\",\n  \"pid\": %d,\n  \"exe\": \"%s\",\n"
      "  \"shm\": \"%s\",\n  \"started_mono_ns\": %llu,\n"
      "  \"started_wall_ns\": %llu,\n  \"rings\": %d\n}\n",
      (int)getpid(), exe, shm_name, (unsigned long long)H->started_mono_ns,
      (unsigned long long)H->started_wall_ns, (int)rings);
    fclose(f);
    rename(tmp, reg_path);
  }
  return 1;
}

static uint64_t fnv(const char *a, const char *b) {
  uint64_t h = 0xcbf29ce484222325ull;
  for (const char *p = a; *p; p++) { h ^= (uint8_t)*p; h *= 0x100000001b3ull; }
  h ^= ':'; h *= 0x100000001b3ull;
  for (const char *p = b; *p; p++) { h ^= (uint8_t)*p; h *= 0x100000001b3ull; }
  return h;
}

static int64_t next_id[4] = {1, 1, 0, 0}; /* topic, locus_type from 1; binding, sched from 0 */

static int64_t manifest_add(uint8_t kind, uint8_t flg, const char *name,
                            uint16_t aux_a, uint64_t shape, uint64_t aux_b) {
  if (!seg) return -1;
  uint32_t i = atomic_load(&MH->entry_count);
  if (i >= ENTRY_CAP) { atomic_fetch_add(&CNT[0].c[1], 1); return -1; }
  int64_t id = next_id[kind]++;
  if (id >= ENTRY_CAP) return -1;
  uint32_t len = (uint32_t)strlen(name);
  uint32_t noff = atomic_fetch_add(&MH->pool_used, len);
  memcpy(POOL + noff, name, len);
  ME[i] = (me_t){ .shape_hash = shape, .aux_b = aux_b, .id = (uint32_t)id,
                  .name_off = noff, .name_len = (uint16_t)len, .aux_a = aux_a,
                  .kind = kind, .flags = flg, ._pad = 0 };
  if (kind == 0 || kind == 2) {
    /* counter line = 1 + count of prior topic/binding entries */
    int line = 1;
    for (uint32_t j = 0; j < i; j++)
      if (ME[j].kind == 0 || ME[j].kind == 2) line++;
    cnt_line_for[kind][id] = line;
  }
  atomic_store_explicit(&MH->entry_count, i + 1, memory_order_release);
  atomic_fetch_add_explicit(&H->manifest_gen, 1, memory_order_release);
  return id;
}

int64_t obs_topic(const char *name, const char *shape, int64_t networked) {
  return manifest_add(0, networked ? 1 : 0, name, 0, fnv(name, shape), 0);
}
int64_t obs_locus_type(const char *name) { return manifest_add(1, 0, name, 0, 0, 0); }
int64_t obs_binding(const char *name, int64_t transport, int64_t topic) {
  return manifest_add(2, 1, name, (uint16_t)transport, 0, (uint64_t)topic);
}
int64_t obs_scheduler(const char *name, int64_t cpu) {
  return manifest_add(3, 0, name, 0, 0, (uint64_t)cpu);
}

int64_t obs_ring_desc_ptr(int64_t ring) {
  if (!seg || ring < 0 || (uint32_t)ring >= H->ring_count) return 0;
  return (int64_t)(intptr_t)&RD[ring];
}
int64_t obs_base_ptr(void) { return (int64_t)(intptr_t)seg; }
int64_t obs_slots(void) { return seg ? (int64_t)H->ring_slots : 0; }
int64_t obs_ts_shift(void) { return seg ? (int64_t)H->ts_shift : 4; }

int64_t obs_observed(void) {
  return seg && atomic_load_explicit(&C->observer_count, memory_order_relaxed) > 0;
}
int64_t obs_mode(int64_t topic) {
  if (!seg || topic < 0 || topic >= ENTRY_CAP) return 0;
  return MODE[topic];
}
void obs_count_topic(int64_t topic, int64_t cell, int64_t delta) {
  if (!seg || topic < 0 || topic >= ENTRY_CAP || cell < 0 || cell > 7) return;
  int line = cnt_line_for[0][topic];
  if (line > 0) atomic_fetch_add_explicit(&CNT[line].c[cell], (uint64_t)delta,
                                          memory_order_relaxed);
}
void obs_count_binding(int64_t binding, int64_t cell, int64_t delta) {
  if (!seg || binding < 0 || binding >= ENTRY_CAP || cell < 0 || cell > 7) return;
  int line = cnt_line_for[2][binding];
  if (line > 0) atomic_fetch_add_explicit(&CNT[line].c[cell], (uint64_t)delta,
                                          memory_order_relaxed);
}
void obs_records_total_add(int64_t n) {
  if (seg) atomic_fetch_add_explicit(&CNT[0].c[2], (uint64_t)n, memory_order_relaxed);
}

void obs_teardown(void) {
  if (!seg) return;
  atomic_store(&H->flags, 0);
  unlink(reg_path);
  munmap(seg, seg_len);
  shm_unlink(shm_name);
  seg = NULL;
}
