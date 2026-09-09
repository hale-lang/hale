/* iris observation protocol v0 — reference C header.
 *
 * This header is the executable form of ../PROTOCOL.md. Where
 * the two disagree, that's a bug: fix both in one commit.
 * Layout facts are pinned with static_asserts so drift fails
 * the build, not the consumer.
 */
#ifndef IRIS_PROTOCOL_H
#define IRIS_PROTOCOL_H

#include <stdint.h>
#include <stdatomic.h>
#include <assert.h>
#include <stddef.h>

#define OBS_PROTO_MAJOR 0
/* Minor history. Every bump here is purely additive at the tail of
 * the header, so a consumer at minor N reads a segment at minor
 * M > N correctly by ignoring what it does not know, and must
 * check proto_minor before reading a field it does know.
 *   0.2 (2026-08-12) + model_hash        @ 0x80
 *   0.3 (2026-08-24) + entity_id_digest  @ 0x88
 *   0.4 (2026-09-04)   manifest aux_b has ONE meaning for every
 *                      emitter: canonical entity id, or 0. v0's
 *                      binding->topic / scheduler->cpu reading is
 *                      retired; scheduler cpu index moves to aux_a.
 *                      No layout change (hale#525, handoff-14 P31).
 * For 0.2/0.3, "absent" and "0" are DIFFERENT answers: absent means
 * the emitter predates the field, 0 means the emitter has the
 * field and positively has nothing to report (a harness build). */
#define OBS_PROTO_MINOR 4

/* "HALEISBO" little-endian; doubles as endianness check. */
#define OBS_MAGIC 0x4F42534948414C45ULL

#define OBS_PAGE 4096

/* ---- header ------------------------------------------------ */

typedef struct {
  uint64_t magic;            /* 0x00 */
  uint16_t proto_major;      /* 0x08 */
  uint16_t proto_minor;      /* 0x0A */
  uint32_t header_len;       /* 0x0C */
  uint64_t total_len;        /* 0x10 */
  uint32_t pid;              /* 0x18 */
  uint32_t ring_count;       /* 0x1C */
  uint32_t ring_slots;       /* 0x20  power of two */
  uint32_t ts_shift;         /* 0x24  ts_delta unit = 2^ts_shift ns */
  uint64_t started_mono_ns;  /* 0x28 */
  uint64_t started_wall_ns;  /* 0x30 */
  uint64_t control_off;      /* 0x38 */
  uint64_t manifest_off;     /* 0x40 */
  uint64_t manifest_len;     /* 0x48 */
  uint64_t modemask_off;     /* 0x50 */
  uint64_t counters_off;     /* 0x58 */
  uint64_t counters_len;     /* 0x60 */
  uint64_t rings_off;        /* 0x68 */
  _Atomic uint64_t flags;    /* 0x70  bit0 alive; bit1 post-mortem */
  _Atomic uint64_t manifest_gen; /* 0x78 */
  uint64_t model_hash;       /* 0x80  proto >= 0.2; see below */
  uint64_t entity_id_digest; /* 0x88  proto >= 0.3; see below */
} obs_header;

/* model_hash: the topology artifact's `shape_hash` — the identity
 * of the MODEL this binary was compiled from, stamped at segment
 * creation. It answers the one question a consumer joining a live
 * manifest to a source-derived artifact cannot otherwise settle:
 * was the running process built from the model I am comparing it
 * against? A comment-only rebuild keeps the value; a model change
 * moves it.
 *
 * Complementary to, not a substitute for, the manifest's per-topic
 * shape_hash: model identity deliberately EXCLUDES payload field
 * shape (a payload edit changes the topic row and no model
 * identity), so the two together are what "in sync" means.
 *
 * 0 is a real value meaning "no model" — a synthetic or non-Hale
 * emitter (this repo's synth.c reads 0). Absent (proto 0.1) is
 * distinct from 0 and means "unknown"; consumers must not
 * conflate them. */

/* entity_id_digest (proto >= 0.3): the identity of the canonical
 * ENTITY ID TABLE whose ids a segment's manifest rows may carry in
 * `aux_b`. model_hash is structural model identity and does not
 * cover every table those ids index, so two builds can share a
 * model_hash while numbering entities differently. A consumer
 * recomputes this digest from the model it holds and may use the
 * ids ONLY on a match; on a mismatch it falls back to name
 * matching. 0 = unstamped. */

static_assert(offsetof(obs_header, magic) == 0x00, "layout");
static_assert(offsetof(obs_header, proto_major) == 0x08, "layout");
static_assert(offsetof(obs_header, header_len) == 0x0C, "layout");
static_assert(offsetof(obs_header, total_len) == 0x10, "layout");
static_assert(offsetof(obs_header, pid) == 0x18, "layout");
static_assert(offsetof(obs_header, ring_count) == 0x1C, "layout");
static_assert(offsetof(obs_header, ring_slots) == 0x20, "layout");
static_assert(offsetof(obs_header, ts_shift) == 0x24, "layout");
static_assert(offsetof(obs_header, started_mono_ns) == 0x28, "layout");
static_assert(offsetof(obs_header, started_wall_ns) == 0x30, "layout");
static_assert(offsetof(obs_header, control_off) == 0x38, "layout");
static_assert(offsetof(obs_header, manifest_off) == 0x40, "layout");
static_assert(offsetof(obs_header, manifest_len) == 0x48, "layout");
static_assert(offsetof(obs_header, modemask_off) == 0x50, "layout");
static_assert(offsetof(obs_header, counters_off) == 0x58, "layout");
static_assert(offsetof(obs_header, counters_len) == 0x60, "layout");
static_assert(offsetof(obs_header, rings_off) == 0x68, "layout");
static_assert(offsetof(obs_header, flags) == 0x70, "layout");
static_assert(offsetof(obs_header, manifest_gen) == 0x78, "layout");
static_assert(offsetof(obs_header, model_hash) == 0x80, "layout");
static_assert(offsetof(obs_header, entity_id_digest) == 0x88, "layout");
static_assert(sizeof(obs_header) <= OBS_PAGE, "header fits its page");

#define OBS_FLAG_ALIVE      (1ULL << 0)
#define OBS_FLAG_POSTMORTEM (1ULL << 1)

/* ---- control region (only consumer-writable page) ---------- */

typedef struct {
  _Atomic uint32_t observer_count; /* 0 = dormant */
  _Atomic uint32_t sample_n;       /* 1-in-N for SAMPLED-RICH */
} obs_control;

/* ---- manifest ---------------------------------------------- */
/* Region layout: obs_manifest_hdr, then entry_cap entries,
 * then string pool to end of region. (PROTOCOL.md §4 —
 * manifest header added by v0 implementation; see spec
 * amendment.) */

typedef struct {
  _Atomic uint32_t entry_count;
  uint32_t entry_cap;
  uint32_t pool_off;   /* from manifest_off */
  _Atomic uint32_t pool_used;
} obs_manifest_hdr;

enum {
  OBS_MK_TOPIC = 0,
  OBS_MK_LOCUS_TYPE = 1,
  OBS_MK_BINDING = 2,
  OBS_MK_SCHEDULER = 3,
};

#define OBS_MF_NETWORKED  (1u << 0)
#define OBS_MF_FROM_TOPO  (1u << 1)

/* Field order chosen for natural alignment (u64s first);
 * PROTOCOL.md §4 amended to match. */
typedef struct {
  uint64_t shape_hash; /* topics; 0 otherwise */
  /* Canonical model ENTITY ID for this row (hale >= proto 0.3),
   * guarded by the header's entity_id_digest; 0 = no canonical id.
   * Resolved 2026-09-04 (proto 0.4, hale#525 / handoff-14 P31): the
   * v0 meaning — binding -> owning topic_id, scheduler -> cpu index
   * — is RETIRED for every emitter. A consumer gates on
   * entity_id_digest != 0; at proto_minor >= 4 it may additionally
   * trust that a nonzero value was never anything else. Segments
   * at 0.3 from iris's own emitters carry the old meaning and a
   * zero digest, so the digest gate alone is correct for them. */
  uint64_t aux_b;
  uint32_t id;
  uint32_t name_off;   /* into string pool */
  uint16_t name_len;
  uint16_t aux_a;      /* binding: transport enum; scheduler: cpu index (0.4) */
  uint8_t  kind;
  uint8_t  flags;
  uint16_t _pad;
} obs_manifest_entry;

static_assert(sizeof(obs_manifest_entry) == 32, "ManifestEntry is 32 B");

enum { OBS_TRANSPORT_UNIX = 0, OBS_TRANSPORT_UDP = 1, OBS_TRANSPORT_TCP = 2 };

/* ---- mode mask --------------------------------------------- */

enum {
  OBS_MODE_OFF = 0,
  OBS_MODE_COUNTERS = 1,
  OBS_MODE_PACKED = 2,   /* default */
  OBS_MODE_SAMPLED = 3,
  OBS_MODE_FIREHOSE = 4,
};

/* ---- counter table ----------------------------------------- */
/* One 64 B line per topic/binding manifest entry, in manifest
 * entry order (counting only topic/binding entries), preceded
 * by one global line. (Global-line-first added by v0
 * implementation; see spec amendment.) */

typedef struct {
  _Atomic uint64_t c[8];
} obs_counter_line;

static_assert(sizeof(obs_counter_line) == 64, "counter line is a cache line");

/* topic line indices */
enum { OBS_CT_PUBLISHED = 0, OBS_CT_DELIVERED = 1, OBS_CT_BYTES = 2, OBS_CT_DROPPED = 3 };
/* binding line indices */
enum { OBS_CB_SENT = 0, OBS_CB_DELIVERED = 1, OBS_CB_BYTES = 2, OBS_CB_QDEPTH = 3,
       OBS_CB_BLOCK_NS = 4, OBS_CB_RETRIES = 5, OBS_CB_SEQ_HW = 6 };
/* global line indices */
enum { OBS_CG_RING_DROPS = 0, OBS_CG_MANIFEST_OVERFLOW = 1, OBS_CG_RECORDS_TOTAL = 2 };

/* ---- records ----------------------------------------------- */

enum {
  OBS_EK_EPOCH = 0,
  OBS_EK_BUS_PUBLISH = 1,
  OBS_EK_BUS_DELIVER = 2,
  OBS_EK_NET_SEND = 3,
  OBS_EK_NET_DELIVER = 4,
  OBS_EK_LOCUS_BIRTH = 5,
  OBS_EK_LOCUS_DISSOLVE = 6,
  OBS_EK_RESTART = 7,
  OBS_EK_SUPERV_TRANS = 8,
  OBS_EK_PLACEMENT = 9,
  OBS_EK_ARENA_MARK = 10,
  OBS_EK_BINDING_UP = 11,
  OBS_EK_BINDING_DOWN = 12,
  OBS_EK_SAMPLE_RICH = 13,
  OBS_EK_DROP_MARK = 14,
  OBS_EK_CONT = 15,
  OBS_EK_LOCUS_ENTER = 16, /* reserved; not emitted in v0 */
  OBS_EK_LOCUS_EXIT = 17,  /* reserved; not emitted in v0 */
};

/* word0: id:20 | ekind:5 | size_class:8 | ts_delta:31 */
#define OBS_TS_DELTA_MAX ((1ULL << 31) - 1)

static inline uint64_t obs_word0(uint32_t id, uint32_t ekind,
                                 uint32_t size_class, uint64_t ts_delta) {
  return ((uint64_t)(id & 0xFFFFFu))
       | ((uint64_t)(ekind & 0x1Fu) << 20)
       | ((uint64_t)(size_class & 0xFFu) << 25)
       | ((ts_delta & OBS_TS_DELTA_MAX) << 33);
}

static inline uint32_t obs_w0_id(uint64_t w)         { return (uint32_t)(w & 0xFFFFFu); }
static inline uint32_t obs_w0_ekind(uint64_t w)      { return (uint32_t)((w >> 20) & 0x1Fu); }
static inline uint32_t obs_w0_size_class(uint64_t w) { return (uint32_t)((w >> 25) & 0xFFu); }
static inline uint64_t obs_w0_ts_delta(uint64_t w)   { return (w >> 33) & OBS_TS_DELTA_MAX; }

/* NET_SEND / NET_DELIVER word1: origin_id:16 | seq:48  (PROTOCOL §8)
 *
 * The pair is the SENDER's, carried on the wire and echoed verbatim
 * by the receiver's NET_DELIVER — so `origin_id` names a stream in
 * the SENDING process's id space, NOT a binding in the manifest of
 * whichever segment the record was read from. Consumers match
 * deliveries to sends on (topic, origin_id, seq); they must never
 * resolve origin_id against the local binding name table.
 *
 * This accessor was called `obs_net_binding` until 2026-08-11 —
 * a name left behind by the pre-amendment layout, and one that
 * actively invited the mistake: peek dutifully looked the value up
 * in the local binding table and rendered `unknown:<origin>` for
 * every NET record. Renamed rather than aliased (no callers
 * anywhere), since a misnomer in the executable form of the
 * protocol is exactly the drift this header exists to prevent. */
static inline uint64_t obs_net_w1(uint32_t origin_id, uint64_t seq) {
  return ((uint64_t)(origin_id & 0xFFFFu)) | ((seq & 0xFFFFFFFFFFFFULL) << 16);
}
static inline uint32_t obs_net_origin(uint64_t w1)  { return (uint32_t)(w1 & 0xFFFFu); }
static inline uint64_t obs_net_seq(uint64_t w1)     { return (w1 >> 16) & 0xFFFFFFFFFFFFULL; }

/* LOCUS_BIRTH word1: parent:32 | type:20 */
static inline uint64_t obs_birth_w1(uint32_t parent, uint32_t type_id) {
  return ((uint64_t)parent) | ((uint64_t)(type_id & 0xFFFFFu) << 32);
}
static inline uint32_t obs_birth_parent(uint64_t w1) { return (uint32_t)(w1 & 0xFFFFFFFFu); }
static inline uint32_t obs_birth_type(uint64_t w1)   { return (uint32_t)((w1 >> 32) & 0xFFFFFu); }

/* BUS_PUBLISH / BUS_DELIVER: locus:20 | seq:44 (2026-07-27
 * amendment -- per-locus activity attribution; locus 0 means
 * unattributed). */
static inline uint64_t obs_bus_w1(uint32_t locus, uint64_t seq) {
  return ((uint64_t)(locus & 0xFFFFFu) << 44) | (seq & 0xFFFFFFFFFFFULL);
}
static inline uint32_t obs_bus_locus(uint64_t w1) { return (uint32_t)((w1 >> 44) & 0xFFFFFu); }
static inline uint64_t obs_bus_seq(uint64_t w1)   { return w1 & 0xFFFFFFFFFFFULL; }

typedef struct {
  uint64_t word0;
  uint64_t word1;
} obs_record;

static_assert(sizeof(obs_record) == 16, "one slot size, 16 B");

/* ---- rings ------------------------------------------------- */

/* Canonical lotus SPSC observation-ring descriptor (hale#244/#247
 * — lotus_spsc_* / std::ring::__spsc_*). tag_a/tag_b are user
 * fields; iris assigns tag_a = sched_id, tag_b = current_locus. */
typedef struct {
  uint64_t data_off;         /* slots array offset from SEGMENT base */
  _Atomic uint64_t head;     /* monotonic, never wraps, release-published */
  _Atomic uint64_t dropped;  /* producer-side drop accounting */
  uint32_t tag_a;            /* iris: sched_id */
  _Atomic uint32_t tag_b;    /* iris: current_locus gauge; 0 = idle */
  uint8_t  reserved[32];
} obs_ring_desc;

static_assert(sizeof(obs_ring_desc) == 64, "RingDesc is 64 B aligned");

/* ---- registration ------------------------------------------ */

#define OBS_REG_DIR_FMT  "%s/hale"          /* under $XDG_RUNTIME_DIR */
#define OBS_SHM_NAME_FMT "/hale-obs-%d"

/* ---- dump -------------------------------------------------- */

#define OBS_DUMP_MAGIC 0x504D5544454C4148ULL /* "HALEDUMP" LE */

typedef struct {
  uint64_t magic;
  uint32_t version;
  uint32_t reserved;
} obs_dump_hdr;

static_assert(sizeof(obs_dump_hdr) == 16, "dump header is 16 B");

#endif /* IRIS_PROTOCOL_H */
