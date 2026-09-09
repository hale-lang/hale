/* obs_attach — consumer-side attach/decode library (seed).
 *
 * One obs_seg per attached segment: mmap, manifest decode with
 * generation rescan, per-ring cursor state with EPOCH timestamp
 * reconstruction, k-way merged event pull. This is the layer
 * iris proper builds on; fuse and hale-top-style tools are its
 * first callers.
 */
#ifndef IRIS_OBS_ATTACH_H
#define IRIS_OBS_ATTACH_H

#include "../emitter/protocol.h"
#include <stddef.h>

typedef struct {
  uint64_t ts;      /* reconstructed CLOCK_MONOTONIC ns */
  uint32_t ekind;
  uint32_t id;
  uint32_t size_class;
  uint64_t w1;
} obs_ev;

typedef struct obs_seg obs_seg;

/* Attach from a registration file; NULL + err on failure. */
obs_seg *obs_attach_reg(const char *reg_path, char *err, size_t errlen);
void obs_detach(obs_seg *s);

int obs_alive(const obs_seg *s);
uint32_t obs_pid(const obs_seg *s);
/* Model identity (proto >= 0.2). Returns 1 and writes *out when the
 * segment carries the field; returns 0 when it does not — a 0.1
 * emitter. Not folded into a plain getter on purpose: *out == 0 is
 * a real answer ("built without a model"), so absent and zero must
 * stay distinguishable at the call site. */
int obs_model_hash(const obs_seg *s, uint64_t *out);
const char *obs_exe(const obs_seg *s); /* registration exe path; "" if absent */
uint64_t obs_started_mono(const obs_seg *s);
uint64_t obs_overruns(const obs_seg *s);

/* Merged next event across the segment's rings (timestamp
 * order). Returns 0 when nothing is pending. Handles manifest
 * generation rescan internally. */
int obs_next(obs_seg *s, obs_ev *out);

/* Manifest access. Names are NUL-terminated, cached, stable
 * for the segment's lifetime. */
uint32_t obs_entry_count(obs_seg *s);
const obs_manifest_entry *obs_entry(obs_seg *s, uint32_t i);
const char *obs_name(obs_seg *s, int kind, uint32_t id); /* NULL if unknown */
uint64_t obs_shape(obs_seg *s, uint32_t topic_id);       /* 0 if unknown */

/* Counter access: line index for a topic/binding id (-1 if
 * absent), then cell reads. Line 0 is the global line. */
int obs_counter_line_of(obs_seg *s, int kind, uint32_t id);
uint64_t obs_counter(const obs_seg *s, int line, int cell);

#endif
