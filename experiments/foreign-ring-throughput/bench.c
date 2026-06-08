/*
 * Foreign-ring throughput microbench (Proposal B, 2026-06-08).
 *
 * Question this answers
 * ---------------------
 * The shipped `ring_layout` consumer/producer (PR3 / M3a) drive the
 * `byte_records` framing from a RUNTIME descriptor: the cursor offset,
 * data_at, capacity, len-prefix width, and alignment are read from a
 * `lotus_shm_layout_t` on the hot path, and the wrap uses `local %
 * capacity`. The design doc (OQ2) flagged that a codegen-SPECIALIZED
 * reader/writer — offsets/width/align baked as compile-time constants,
 * power-of-two capacity reduced to a mask — could be faster.
 *
 * Before building that specialization (and the A1 zero-copy producer
 * view), this bench measures whether the gap is real. The honest prior:
 * the descriptor fields are loop-invariant, so LLVM LICM should hoist
 * them and the two paths should be close. This bench tests that.
 *
 * Paths measured (per record; 80-byte flat L2Update-shaped payload)
 * -----------------------------------------------------------------
 * PRODUCER framing (the inner write loop, modelled — no subject lookup):
 *   pub-desc   : offsets/width/align from a runtime descriptor; `% cap`.
 *   pub-const  : offsets/width/align as constants; cap mask (pow-2);
 *                fixed-width store instead of memcpy(,,width).
 * CONSUMER walk (the inner read loop, modelled):
 *   read-desc  : descriptor-driven record walk.
 *   read-const : constant-folded record walk.
 * End-to-end shipped paths (real SHM, include the registry strcmp):
 *   pub-shipped : lotus_bus_publish_shm_ring_layout (the M3a path).
 *   pub-native  : lotus_bus_publish_shm_ring (LRSRNG1 one-memcpy ref).
 *
 * pub-const vs pub-desc (and read-const vs read-desc) = the headroom a
 * codegen specialization would capture. pub-shipped vs pub-desc =
 * the per-publish subject-lookup overhead (a separate, cheaply-cached
 * cost). pub-native = the existing fast-path reference point.
 *
 * Methodology: 7 rounds of 200k iterations; median ns/op per path.
 *
 * Build & run: see run.sh (links runtime/lotus_shm_ring.c).
 */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

/* --- shipped runtime symbols (linked from lotus_shm_ring.c) --- */
extern void lotus_bus_register_shm_ring_layout(
    const char *subject, const char *shm_name,
    const uint64_t *desc_words, uint64_t capacity);
extern int lotus_bus_publish_shm_ring_layout(
    const char *subject, const void *value, uint64_t value_size);
extern void lotus_bus_register_shm_ring(
    const char *subject, uint64_t slot_size, uint64_t slot_count,
    const char *shm_name, int32_t overflow_policy);
extern int lotus_bus_publish_shm_ring(
    const char *subject, const void *value, uint64_t value_size);

#define N_ROUNDS 7
#define ITER     200000

/* 80-byte flat payload — L2Update-shaped (matches k2-zero-copy). */
typedef struct {
    int64_t f[10];
} Payload;

/* byte_records layout constants (the ForeignRing shape). */
#define MAGIC        0x52494E47464D5431ULL
#define DATA_AT      128
#define CURSOR_OFF   64
#define LEN_WIDTH    4
#define ALIGN        8
#define CAP          (16u * 1024u * 1024u)   /* 16 MiB, power of two */

static int64_t now_ns(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (int64_t)t.tv_sec * 1000000000LL + t.tv_nsec;
}

static int cmp_i64(const void *a, const void *b) {
    int64_t x = *(const int64_t *)a, y = *(const int64_t *)b;
    return (x > y) - (x < y);
}
static int64_t median(int64_t *xs, int n) {
    qsort(xs, n, sizeof(int64_t), cmp_i64);
    return xs[n / 2];
}

/* Runtime-descriptor mirror for the desc-driven variants. */
typedef struct {
    uint64_t data_at, cursor_off, len_width, align, cap;
} desc_t;

static void wr_uint(void *p, uint64_t width, uint64_t v) {
    memcpy(p, &v, (size_t)width);
}
static uint64_t rd_uint(const void *p, uint64_t width) {
    uint64_t v = 0; memcpy(&v, p, (size_t)width); return v;
}

int main(void) {
    char *buf = (char *)aligned_alloc(64, DATA_AT + CAP);
    if (!buf) { perror("alloc"); return 1; }
    memset(buf, 0, DATA_AT + CAP);
    Payload p;
    for (int i = 0; i < 10; i++) p.f[i] = i * 1009 + 7;

    int64_t r_pdesc[N_ROUNDS], r_pconst[N_ROUNDS], r_pincr[N_ROUNDS];
    int64_t r_rdesc[N_ROUNDS], r_rconst[N_ROUNDS], r_rincr[N_ROUNDS];
    int64_t r_ship[N_ROUNDS], r_nat[N_ROUNDS];
    volatile uint64_t sink = 0;

    /* Launder the descriptor so the optimizer treats its fields as
     * genuine runtime values (as the real runtime does — they come
     * from a `lotus_shm_layout_ring_t` in memory). Without this, a
     * `const` desc with literal initializers is constant-folded —
     * including strength-reducing `% cap` to a mask — which is
     * exactly the specialization we're trying to measure the ABSENCE
     * of. LICM may still hoist the loop-invariant field LOADS into
     * registers (the runtime gets that too); what it cannot do is
     * prove `cap` is a power of two. */
    desc_t d = { DATA_AT, CURSOR_OFF, LEN_WIDTH, ALIGN, CAP };
    __asm__ volatile("" : : "r"(&d) : "memory");
    const uint64_t psz = sizeof(Payload);

    /* Register the shipped layout producer + a native ring once. */
    uint64_t dw[16] = {0};
    dw[0]=MAGIC; dw[1]=1; dw[2]=8; dw[3]=4; dw[4]=1; dw[5]=1;
    dw[6]=12; dw[7]=4; dw[8]=1; dw[9]=DATA_AT; dw[10]=CURSOR_OFF;
    dw[11]=LEN_WIDTH; dw[12]=ALIGN; dw[13]=0xFFFFFFFF; dw[14]=1;
    char shipname[64], natname[64];
    snprintf(shipname, sizeof(shipname), "/frbench-ship-%d", (int)getpid());
    snprintf(natname, sizeof(natname), "/frbench-nat-%d", (int)getpid());
    lotus_bus_register_shm_ring_layout("frbench.ship", shipname, dw, CAP);
    lotus_bus_register_shm_ring("frbench.nat", psz, 4096, natname, 1);

    for (int round = 0; round < N_ROUNDS; round++) {
        int64_t t0, t1;
        uint64_t local;

        /* pub-desc: descriptor-driven framing. */
        local = 0;
        t0 = now_ns();
        for (int i = 0; i < ITER; i++) {
            uint64_t step = d.len_width + psz;
            if (d.align > 1) step = (step + d.align - 1) & ~(d.align - 1);
            if ((local % d.cap) + step > d.cap) local += d.cap - (local % d.cap);
            uint64_t off = d.data_at + (local % d.cap);
            wr_uint(buf + off, d.len_width, psz);
            memcpy(buf + off + d.len_width, &p, psz);
            local += step;
            atomic_store_explicit((_Atomic uint64_t *)(buf + d.cursor_off),
                                  local, memory_order_release);
        }
        t1 = now_ns(); r_pdesc[round] = t1 - t0; sink += local;

        /* pub-const: constants + pow-2 mask + fixed-width store. */
        local = 0;
        t0 = now_ns();
        for (int i = 0; i < ITER; i++) {
            uint64_t step = (LEN_WIDTH + psz + (ALIGN - 1)) & ~((uint64_t)ALIGN - 1);
            if ((local & (CAP - 1)) + step > CAP) local += CAP - (local & (CAP - 1));
            uint64_t off = DATA_AT + (local & (CAP - 1));
            *(uint32_t *)(buf + off) = (uint32_t)psz;
            memcpy(buf + off + LEN_WIDTH, &p, psz);
            local += step;
            atomic_store_explicit((_Atomic uint64_t *)(buf + CURSOR_OFF),
                                  local, memory_order_release);
        }
        t1 = now_ns(); r_pconst[round] = t1 - t0; sink += local;

        /* pub-incr: descriptor-driven (runtime cap, NO pow-2 assumption)
         * but the wrapped offset is maintained incrementally instead of
         * `% cap`. Records never straddle the wrap (pad guarantees it),
         * so `pos` stays in [0, cap) with a compare-subtract. */
        local = 0;
        {
            uint64_t pos = 0;
            t0 = now_ns();
            for (int i = 0; i < ITER; i++) {
                uint64_t step = d.len_width + psz;
                if (d.align > 1) step = (step + d.align - 1) & ~(d.align - 1);
                if (pos + step > d.cap) { local += d.cap - pos; pos = 0; }
                uint64_t off = d.data_at + pos;
                wr_uint(buf + off, d.len_width, psz);
                memcpy(buf + off + d.len_width, &p, psz);
                local += step; pos += step;
                atomic_store_explicit((_Atomic uint64_t *)(buf + d.cursor_off),
                                      local, memory_order_release);
            }
            t1 = now_ns(); r_pincr[round] = t1 - t0; sink += local;
        }

        /* read-desc: descriptor-driven record walk over what we wrote. */
        {
            uint64_t committed = atomic_load_explicit(
                (_Atomic uint64_t *)(buf + d.cursor_off), memory_order_acquire);
            uint64_t loc = 0; uint64_t acc = 0; int n = 0;
            t0 = now_ns();
            while (loc < committed && n < ITER) {
                uint64_t off = d.data_at + (loc % d.cap);
                uint64_t len = rd_uint(buf + off, d.len_width);
                if (len == 0xFFFFFFFF) { loc += d.cap - (loc % d.cap); continue; }
                acc += ((Payload *)(buf + off + d.len_width))->f[0];
                uint64_t step = d.len_width + len;
                if (d.align > 1) step = (step + d.align - 1) & ~(d.align - 1);
                loc += step; n++;
            }
            t1 = now_ns(); r_rdesc[round] = (t1 - t0) * ITER / (n ? n : 1);
            sink += acc;
        }

        /* read-incr: descriptor-driven walk, incremental offset (no %). */
        {
            uint64_t committed = atomic_load_explicit(
                (_Atomic uint64_t *)(buf + d.cursor_off), memory_order_acquire);
            uint64_t loc = 0, pos = 0, acc = 0; int n = 0;
            t0 = now_ns();
            while (loc < committed && n < ITER) {
                uint64_t off = d.data_at + pos;
                uint64_t len = rd_uint(buf + off, d.len_width);
                if (len == 0xFFFFFFFF) { loc += d.cap - pos; pos = 0; continue; }
                acc += ((Payload *)(buf + off + d.len_width))->f[0];
                uint64_t step = d.len_width + len;
                if (d.align > 1) step = (step + d.align - 1) & ~(d.align - 1);
                loc += step; pos += step; if (pos >= d.cap) pos -= d.cap;
                n++;
            }
            t1 = now_ns(); r_rincr[round] = (t1 - t0) * ITER / (n ? n : 1);
            sink += acc;
        }

        /* read-const: constant-folded walk. */
        {
            uint64_t committed = atomic_load_explicit(
                (_Atomic uint64_t *)(buf + CURSOR_OFF), memory_order_acquire);
            uint64_t loc = 0; uint64_t acc = 0; int n = 0;
            t0 = now_ns();
            while (loc < committed && n < ITER) {
                uint64_t off = DATA_AT + (loc & (CAP - 1));
                uint32_t len = *(uint32_t *)(buf + off);
                if (len == 0xFFFFFFFF) { loc += CAP - (loc & (CAP - 1)); continue; }
                acc += ((Payload *)(buf + off + LEN_WIDTH))->f[0];
                loc += (LEN_WIDTH + len + (ALIGN - 1)) & ~((uint64_t)ALIGN - 1);
                n++;
            }
            t1 = now_ns(); r_rconst[round] = (t1 - t0) * ITER / (n ? n : 1);
            sink += acc;
        }

        /* pub-shipped: the real M3a publish path (incl. subject strcmp). */
        t0 = now_ns();
        for (int i = 0; i < ITER; i++) {
            lotus_bus_publish_shm_ring_layout("frbench.ship", &p, psz);
        }
        t1 = now_ns(); r_ship[round] = t1 - t0;

        /* pub-native: the LRSRNG1 one-memcpy publish reference. */
        t0 = now_ns();
        for (int i = 0; i < ITER; i++) {
            lotus_bus_publish_shm_ring("frbench.nat", &p, psz);
        }
        t1 = now_ns(); r_nat[round] = t1 - t0;
    }

    double pdesc  = (double)median(r_pdesc,  N_ROUNDS) / ITER;
    double pconst = (double)median(r_pconst, N_ROUNDS) / ITER;
    double pincr  = (double)median(r_pincr,  N_ROUNDS) / ITER;
    double rdesc  = (double)median(r_rdesc,  N_ROUNDS) / ITER;
    double rconst = (double)median(r_rconst, N_ROUNDS) / ITER;
    double rincr  = (double)median(r_rincr,  N_ROUNDS) / ITER;
    double ship   = (double)median(r_ship,   N_ROUNDS) / ITER;
    double nat    = (double)median(r_nat,    N_ROUNDS) / ITER;

    printf("foreign-ring throughput (median over %d rounds x %d iter)\n",
           N_ROUNDS, ITER);
    printf("  payload: %zu bytes flat; capacity: %u bytes (pow2)\n\n",
           sizeof(Payload), CAP);
    printf("  PRODUCER framing (inner loop, no subject lookup):\n");
    printf("    pub-desc   %6.2f ns/op  (%6.1f M rec/s)  runtime cap, %% modulo\n", pdesc,  1000.0/pdesc);
    printf("    pub-incr   %6.2f ns/op  (%6.1f M rec/s)  runtime cap, incremental\n", pincr,  1000.0/pincr);
    printf("    pub-const  %6.2f ns/op  (%6.1f M rec/s)  const cap, pow-2 mask\n", pconst, 1000.0/pconst);
    printf("    incremental recovers: %.2f of %.2f ns/op headroom\n\n",
           pdesc - pincr, pdesc - pconst);
    printf("  CONSUMER walk (inner loop):\n");
    printf("    read-desc  %6.2f ns/op  (%6.1f M rec/s)  runtime cap, %% modulo\n", rdesc,  1000.0/rdesc);
    printf("    read-incr  %6.2f ns/op  (%6.1f M rec/s)  runtime cap, incremental\n", rincr,  1000.0/rincr);
    printf("    read-const %6.2f ns/op  (%6.1f M rec/s)  const cap, pow-2 mask\n", rconst, 1000.0/rconst);
    printf("    incremental recovers: %.2f of %.2f ns/op headroom\n\n",
           rdesc - rincr, rdesc - rconst);
    printf("  END-TO-END shipped publish (real SHM):\n");
    printf("    pub-shipped %6.2f ns/op  (incl. subject strcmp)\n", ship);
    printf("    pub-native  %6.2f ns/op  (LRSRNG1 one-memcpy ref)\n", nat);
    printf("    subject-lookup overhead vs pub-desc: %.2f ns/op\n", ship - pdesc);

    if (sink == 0x123456789) printf("");  /* keep sink live */
    free(buf);
    return 0;
}
