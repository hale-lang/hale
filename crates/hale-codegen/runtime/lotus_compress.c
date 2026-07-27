/*
 * Hale compression substrate — `std::compress::*` (GH #254).
 *
 * Lives in its own translation unit (same rationale as
 * lotus_tls.c) so helper test binaries that include
 * lotus_arena.c directly don't pick up a zlib dependency. The
 * main `hale build` link line compiles this TU and adds `-lz`.
 *
 * Surface (one-shot, Bytes -> Bytes):
 *   - lotus_compress_gzip(src)   — gzip container, zlib, level 6
 *   - lotus_compress_gunzip(src) — accepts gzip OR bare zlib
 *                                  streams (inflateInit2 15+32
 *                                  auto-detect)
 *   - lotus_compress_zstd(src)   — zstd frame, level 3
 *   - lotus_compress_unzstd(src) — zstd frame with a declared
 *                                  content size (our own frames
 *                                  always carry one; streaming
 *                                  frames without it are
 *                                  rejected EINVAL at v1)
 *
 * All four return a fresh Bytes blob anchored via
 * lotus_caller_or_global_bytes_create (caller-arena TLS when
 * set, capped global payload arena otherwise) or NULL with
 * errno set:
 *   EINVAL — corrupt / truncated / unsupported input
 *   ENOMEM — allocation failure (or arena cap)
 *   EFBIG  — decompressed output exceeds the 1 GiB guard
 *   ENOENT — zstd only: libzstd is not installed
 *
 * zlib is linked directly (-lz: universally present on Linux
 * and in the macOS SDK). zstd is loaded via dlopen at first
 * use, so `hale build` and emitted binaries carry NO link-time
 * libzstd dependency — programs that never call std::compress::
 * zstd/unzstd run anywhere, and ones that do get a clean
 * fallible "not_found" on machines without the library.
 */

#include <dlfcn.h>
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <zlib.h>

/* From lotus_arena.c — the Bytes blob layout ([i64 len][body])
 * stays private to these accessors. */
extern void *lotus_caller_or_global_bytes_create(int64_t len);
extern int64_t lotus_bytes_len(const void *b);
extern void *lotus_bytes_data(void *b);

/* Decompression output guard: one-shot Bytes land in an arena;
 * a zip-bomb (or corrupt length) must not bump-allocate the
 * process to death. Streaming (v2) is the path for bigger. */
#define LOTUS_COMPRESS_MAX_OUT ((size_t)1 << 30)

#define LOTUS_GZIP_LEVEL 6
#define LOTUS_ZSTD_LEVEL 3

/* Copy a finished [buf, len] into a fresh Bytes blob. */
static void *lotus_compress_finish(const void *buf, size_t len) {
    void *blob = lotus_caller_or_global_bytes_create((int64_t)len);
    if (!blob) {
        errno = ENOMEM;
        return NULL;
    }
    if (len > 0) {
        memcpy(lotus_bytes_data(blob), buf, len);
    }
    return blob;
}

void *lotus_compress_gzip(const void *src_blob) {
    if (!src_blob) {
        errno = EINVAL;
        return NULL;
    }
    int64_t slen = lotus_bytes_len(src_blob);
    const Bytef *sdata = (const Bytef *)lotus_bytes_data((void *)src_blob);
    z_stream zs;
    memset(&zs, 0, sizeof zs);
    /* windowBits 15+16 = gzip container. */
    if (deflateInit2(&zs, LOTUS_GZIP_LEVEL, Z_DEFLATED, 15 + 16, 8,
                     Z_DEFAULT_STRATEGY) != Z_OK) {
        errno = ENOMEM;
        return NULL;
    }
    uLong cap = deflateBound(&zs, (uLong)slen);
    Bytef *buf = (Bytef *)malloc(cap ? cap : 1);
    if (!buf) {
        deflateEnd(&zs);
        errno = ENOMEM;
        return NULL;
    }
    zs.next_in = (Bytef *)sdata;
    zs.avail_in = (uInt)slen;
    zs.next_out = buf;
    zs.avail_out = (uInt)cap;
    int rc = deflate(&zs, Z_FINISH);
    size_t out_len = zs.total_out;
    deflateEnd(&zs);
    if (rc != Z_STREAM_END) {
        /* deflateBound-sized single shot can only fail on
         * internal state errors. */
        free(buf);
        errno = EINVAL;
        return NULL;
    }
    void *blob = lotus_compress_finish(buf, out_len);
    free(buf);
    return blob;
}

void *lotus_compress_gunzip(const void *src_blob) {
    if (!src_blob) {
        errno = EINVAL;
        return NULL;
    }
    int64_t slen = lotus_bytes_len(src_blob);
    const Bytef *sdata = (const Bytef *)lotus_bytes_data((void *)src_blob);
    if (slen <= 0) {
        errno = EINVAL;
        return NULL;
    }
    z_stream zs;
    memset(&zs, 0, sizeof zs);
    /* windowBits 15+32 = auto-detect gzip or zlib wrapper. */
    if (inflateInit2(&zs, 15 + 32) != Z_OK) {
        errno = ENOMEM;
        return NULL;
    }
    size_t cap = (size_t)slen * 4 + 64;
    if (cap < 4096) cap = 4096;
    if (cap > LOTUS_COMPRESS_MAX_OUT) cap = LOTUS_COMPRESS_MAX_OUT;
    Bytef *buf = (Bytef *)malloc(cap);
    if (!buf) {
        inflateEnd(&zs);
        errno = ENOMEM;
        return NULL;
    }
    zs.next_in = (Bytef *)sdata;
    zs.avail_in = (uInt)slen;
    size_t total = 0;
    int saved_errno = EINVAL;
    for (;;) {
        zs.next_out = buf + total;
        zs.avail_out = (uInt)(cap - total);
        int rc = inflate(&zs, Z_NO_FLUSH);
        total = zs.total_out;
        if (rc == Z_STREAM_END) {
            inflateEnd(&zs);
            void *blob = lotus_compress_finish(buf, total);
            free(buf);
            return blob;
        }
        if (rc == Z_OK || rc == Z_BUF_ERROR) {
            if (total == cap) {
                /* Output full — grow. */
                if (cap >= LOTUS_COMPRESS_MAX_OUT) {
                    saved_errno = EFBIG;
                    break;
                }
                size_t ncap = cap * 2;
                if (ncap > LOTUS_COMPRESS_MAX_OUT) {
                    ncap = LOTUS_COMPRESS_MAX_OUT;
                }
                Bytef *nbuf = (Bytef *)realloc(buf, ncap);
                if (!nbuf) {
                    saved_errno = ENOMEM;
                    break;
                }
                buf = nbuf;
                cap = ncap;
                continue;
            }
            if (zs.avail_in == 0) {
                /* Input exhausted before stream end: truncated. */
                saved_errno = EINVAL;
                break;
            }
            continue;
        }
        /* Z_DATA_ERROR / Z_NEED_DICT / Z_MEM_ERROR / ... */
        saved_errno = (rc == Z_MEM_ERROR) ? ENOMEM : EINVAL;
        break;
    }
    inflateEnd(&zs);
    free(buf);
    errno = saved_errno;
    return NULL;
}

/* ---- zstd via dlopen ------------------------------------------- */

typedef size_t (*lotus_zstd_bound_fn)(size_t);
typedef size_t (*lotus_zstd_compress_fn)(void *, size_t, const void *,
                                         size_t, int);
typedef unsigned long long (*lotus_zstd_content_size_fn)(const void *,
                                                          size_t);
typedef size_t (*lotus_zstd_decompress_fn)(void *, size_t, const void *,
                                           size_t);
typedef unsigned (*lotus_zstd_is_error_fn)(size_t);

static struct {
    void *handle;
    lotus_zstd_bound_fn bound;
    lotus_zstd_compress_fn compress;
    lotus_zstd_content_size_fn content_size;
    lotus_zstd_decompress_fn decompress;
    lotus_zstd_is_error_fn is_error;
} g_zstd;

static pthread_once_t g_zstd_once = PTHREAD_ONCE_INIT;

static void lotus_zstd_load(void) {
    static const char *candidates[] = {
        "libzstd.so.1", "libzstd.so",       /* Linux */
        "libzstd.1.dylib", "libzstd.dylib", /* macOS */
        /* Homebrew keg paths — not on the default dyld search
         * path, and Homebrew is where macOS users have zstd. */
        "/opt/homebrew/lib/libzstd.1.dylib",  /* Apple Silicon */
        "/usr/local/lib/libzstd.1.dylib",     /* Intel */
    };
    void *h = NULL;
    for (size_t i = 0; i < sizeof(candidates) / sizeof(candidates[0]);
         i++) {
        h = dlopen(candidates[i], RTLD_NOW | RTLD_LOCAL);
        if (h) break;
    }
    if (!h) return;
    g_zstd.bound = (lotus_zstd_bound_fn)dlsym(h, "ZSTD_compressBound");
    g_zstd.compress = (lotus_zstd_compress_fn)dlsym(h, "ZSTD_compress");
    g_zstd.content_size =
        (lotus_zstd_content_size_fn)dlsym(h, "ZSTD_getFrameContentSize");
    g_zstd.decompress =
        (lotus_zstd_decompress_fn)dlsym(h, "ZSTD_decompress");
    g_zstd.is_error = (lotus_zstd_is_error_fn)dlsym(h, "ZSTD_isError");
    if (!g_zstd.bound || !g_zstd.compress || !g_zstd.content_size ||
        !g_zstd.decompress || !g_zstd.is_error) {
        /* Partial symbol table — treat as absent. */
        dlclose(h);
        memset(&g_zstd, 0, sizeof g_zstd);
        return;
    }
    g_zstd.handle = h;
}

static int lotus_zstd_ready(void) {
    pthread_once(&g_zstd_once, lotus_zstd_load);
    return g_zstd.handle != NULL;
}

void *lotus_compress_zstd(const void *src_blob) {
    if (!src_blob) {
        errno = EINVAL;
        return NULL;
    }
    if (!lotus_zstd_ready()) {
        errno = ENOENT;
        return NULL;
    }
    int64_t slen = lotus_bytes_len(src_blob);
    const void *sdata = lotus_bytes_data((void *)src_blob);
    size_t cap = g_zstd.bound((size_t)slen);
    void *buf = malloc(cap ? cap : 1);
    if (!buf) {
        errno = ENOMEM;
        return NULL;
    }
    size_t n =
        g_zstd.compress(buf, cap, sdata, (size_t)slen, LOTUS_ZSTD_LEVEL);
    if (g_zstd.is_error(n)) {
        free(buf);
        errno = EINVAL;
        return NULL;
    }
    void *blob = lotus_compress_finish(buf, n);
    free(buf);
    return blob;
}

void *lotus_compress_unzstd(const void *src_blob) {
    if (!src_blob) {
        errno = EINVAL;
        return NULL;
    }
    if (!lotus_zstd_ready()) {
        errno = ENOENT;
        return NULL;
    }
    int64_t slen = lotus_bytes_len(src_blob);
    const void *sdata = lotus_bytes_data((void *)src_blob);
    if (slen <= 0) {
        errno = EINVAL;
        return NULL;
    }
    unsigned long long content =
        g_zstd.content_size(sdata, (size_t)slen);
    /* ZSTD_CONTENTSIZE_UNKNOWN (-1): streaming frame without a
     * declared size — v1 rejects (our own frames always declare;
     * a growth-loop streaming decode is the v2 path).
     * ZSTD_CONTENTSIZE_ERROR (-2): not a zstd frame. */
    if (content == (unsigned long long)-1 ||
        content == (unsigned long long)-2 ||
        content > (unsigned long long)LOTUS_COMPRESS_MAX_OUT) {
        errno = (content > (unsigned long long)LOTUS_COMPRESS_MAX_OUT &&
                 content != (unsigned long long)-1 &&
                 content != (unsigned long long)-2)
                    ? EFBIG
                    : EINVAL;
        return NULL;
    }
    void *buf = malloc(content ? (size_t)content : 1);
    if (!buf) {
        errno = ENOMEM;
        return NULL;
    }
    size_t n = g_zstd.decompress(buf, (size_t)content, sdata,
                                 (size_t)slen);
    if (g_zstd.is_error(n) || n != (size_t)content) {
        free(buf);
        errno = EINVAL;
        return NULL;
    }
    void *blob = lotus_compress_finish(buf, n);
    free(buf);
    return blob;
}

/* ---- tar (ustar) ------------------------------------------------
 *
 * One-shot ustar reader/writer over Bytes (GH #254). Read side:
 * indexed accessors that scan the archive per call (O(n) each —
 * archives an infra tool unpacks are list-then-extract, and the
 * scan is a memcmp-light header walk). Write side: append-style —
 * pack(archive, name, data) returns archive + one file entry;
 * finish(archive) appends the two terminating zero blocks.
 *
 * Subset: regular files ('0'/NUL) and directories ('5'); ustar
 * prefix field honored on read and used on write for names over
 * 100 bytes (up to 155+'/'+100); octal sizes only (no GNU
 * base-256 — EINVAL); mtime written as 0 for reproducible
 * archives. Other entry types are visible to entry_type/name/size
 * and skippable by the caller.
 */

#define LOTUS_TAR_BLOCK 512

extern void *lotus_bus_payload_arena_alloc(uint64_t size, uint64_t align);

static int64_t lotus_tar_parse_octal(const char *p, size_t n) {
    int64_t v = 0;
    size_t i = 0;
    while (i < n && (p[i] == ' ' || p[i] == '0')) i++;
    for (; i < n; i++) {
        char c = p[i];
        if (c == '\0' || c == ' ') break;
        if (c < '0' || c > '7') return -1;
        if (v > (INT64_MAX >> 3)) return -1;
        v = (v << 3) | (int64_t)(c - '0');
    }
    return v;
}

/* Walk headers. Returns the number of entries, or -1 on a
 * malformed archive. When `want >= 0`, stops at that entry and
 * fills the out params instead (returns `want` on success). */
static int64_t lotus_tar_walk(const uint8_t *data, size_t len,
                              int64_t want,
                              const uint8_t **out_hdr,
                              const uint8_t **out_body,
                              int64_t *out_size) {
    size_t off = 0;
    int64_t idx = 0;
    while (off + LOTUS_TAR_BLOCK <= len) {
        const uint8_t *hdr = data + off;
        /* Terminating zero block? */
        int all_zero = 1;
        for (size_t i = 0; i < LOTUS_TAR_BLOCK; i++) {
            if (hdr[i] != 0) { all_zero = 0; break; }
        }
        if (all_zero) break;
        /* magic at offset 257: "ustar" (POSIX "ustar\0" or GNU
         * "ustar "). */
        if (memcmp(hdr + 257, "ustar", 5) != 0) return -1;
        int64_t size = lotus_tar_parse_octal((const char *)hdr + 124, 12);
        if (size < 0) return -1;
        size_t body_blocks =
            ((size_t)size + LOTUS_TAR_BLOCK - 1) / LOTUS_TAR_BLOCK;
        if (off + LOTUS_TAR_BLOCK + body_blocks * LOTUS_TAR_BLOCK > len) {
            return -1; /* truncated body */
        }
        if (want == idx) {
            if (out_hdr) *out_hdr = hdr;
            if (out_body) *out_body = hdr + LOTUS_TAR_BLOCK;
            if (out_size) *out_size = size;
            return idx;
        }
        off += LOTUS_TAR_BLOCK + body_blocks * LOTUS_TAR_BLOCK;
        idx++;
    }
    if (want >= 0) return -1; /* index out of range */
    return idx;
}

int64_t lotus_tar_entries(const void *archive_blob) {
    if (!archive_blob) { errno = EINVAL; return -1; }
    const uint8_t *data = lotus_bytes_data((void *)archive_blob);
    int64_t len = lotus_bytes_len(archive_blob);
    int64_t n = lotus_tar_walk(data, (size_t)len, -1, NULL, NULL, NULL);
    if (n < 0) errno = EINVAL;
    return n;
}

/* Entry name: prefix field (if set) + '/' + name field, anchored
 * as a NUL-terminated String in the caller/payload arena. */
const char *lotus_tar_entry_name(const void *archive_blob, int64_t i) {
    if (!archive_blob || i < 0) { errno = EINVAL; return NULL; }
    const uint8_t *hdr = NULL;
    if (lotus_tar_walk(lotus_bytes_data((void *)archive_blob),
                       (size_t)lotus_bytes_len(archive_blob), i, &hdr,
                       NULL, NULL) < 0) {
        errno = EINVAL;
        return NULL;
    }
    char name[101], prefix[156];
    memcpy(name, hdr, 100);
    name[100] = '\0';
    memcpy(prefix, hdr + 345, 155);
    prefix[155] = '\0';
    size_t nl = strlen(name), pl = strlen(prefix);
    size_t total = pl ? pl + 1 + nl : nl;
    char *out = (char *)lotus_bus_payload_arena_alloc(total + 1, 1);
    if (!out) { errno = ENOMEM; return NULL; }
    if (pl) {
        memcpy(out, prefix, pl);
        out[pl] = '/';
        memcpy(out + pl + 1, name, nl + 1);
    } else {
        memcpy(out, name, nl + 1);
    }
    return out;
}

int64_t lotus_tar_entry_size(const void *archive_blob, int64_t i) {
    if (!archive_blob || i < 0) { errno = EINVAL; return -1; }
    int64_t size = 0;
    if (lotus_tar_walk(lotus_bytes_data((void *)archive_blob),
                       (size_t)lotus_bytes_len(archive_blob), i, NULL,
                       NULL, &size) < 0) {
        errno = EINVAL;
        return -1;
    }
    return size;
}

/* "file" / "dir" / "link" / "symlink" / "other" — static strings,
 * no allocation. */
const char *lotus_tar_entry_type(const void *archive_blob, int64_t i) {
    if (!archive_blob || i < 0) { errno = EINVAL; return NULL; }
    const uint8_t *hdr = NULL;
    if (lotus_tar_walk(lotus_bytes_data((void *)archive_blob),
                       (size_t)lotus_bytes_len(archive_blob), i, &hdr,
                       NULL, NULL) < 0) {
        errno = EINVAL;
        return NULL;
    }
    switch (hdr[156]) {
        case '0': case '\0': return "file";
        case '5':            return "dir";
        case '1':            return "link";
        case '2':            return "symlink";
        default:             return "other";
    }
}

void *lotus_tar_entry_data(const void *archive_blob, int64_t i) {
    if (!archive_blob || i < 0) { errno = EINVAL; return NULL; }
    const uint8_t *body = NULL;
    int64_t size = 0;
    if (lotus_tar_walk(lotus_bytes_data((void *)archive_blob),
                       (size_t)lotus_bytes_len(archive_blob), i, NULL,
                       &body, &size) < 0) {
        errno = EINVAL;
        return NULL;
    }
    return lotus_compress_finish(body, (size_t)size);
}

static void lotus_tar_write_octal(char *dst, size_t n, int64_t v) {
    /* n-1 octal digits + NUL, zero-padded. */
    dst[n - 1] = '\0';
    for (size_t i = n - 1; i-- > 0;) {
        dst[i] = (char)('0' + (v & 7));
        v >>= 3;
    }
}

/* Append one entry (header + padded body) to `archive_blob`
 * (empty Bytes starts a new archive) and return the new Bytes.
 * typeflag '0' = file (mode 644, data required), '5' = dir
 * (mode 755, data ignored). */
static void *lotus_tar_pack_entry(const void *archive_blob,
                                  const char *name,
                                  const void *data_blob,
                                  char typeflag) {
    if (!archive_blob || !name) { errno = EINVAL; return NULL; }
    size_t name_len = strlen(name);
    if (name_len == 0) { errno = EINVAL; return NULL; }
    /* Split into (prefix, name) when over 100 bytes. */
    const char *tail = name;
    size_t tail_len = name_len, prefix_len = 0;
    const char *prefix_start = NULL;
    if (name_len > 100) {
        /* Find a '/' so that the part after it is <= 100 and the
         * part before is <= 155. Prefer the shortest tail. */
        for (const char *p = name + (name_len > 155 ? name_len - 155 - 1
                                                    : 0);
             (p = strchr(p, '/')) != NULL; p++) {
            size_t pl = (size_t)(p - name);
            size_t tl = name_len - pl - 1;
            if (pl <= 155 && tl > 0 && tl <= 100) {
                prefix_start = name;
                prefix_len = pl;
                tail = p + 1;
                tail_len = tl;
                break;
            }
        }
        if (!prefix_start) { errno = ENAMETOOLONG; return NULL; }
    }
    int64_t body_len =
        (typeflag == '0' && data_blob) ? lotus_bytes_len(data_blob) : 0;
    size_t body_blocks =
        ((size_t)body_len + LOTUS_TAR_BLOCK - 1) / LOTUS_TAR_BLOCK;
    int64_t old_len = lotus_bytes_len(archive_blob);
    size_t add = LOTUS_TAR_BLOCK + body_blocks * LOTUS_TAR_BLOCK;
    void *out = lotus_caller_or_global_bytes_create(old_len + (int64_t)add);
    if (!out) { errno = ENOMEM; return NULL; }
    uint8_t *dst = lotus_bytes_data(out);
    if (old_len > 0) {
        memcpy(dst, lotus_bytes_data((void *)archive_blob),
               (size_t)old_len);
    }
    uint8_t *hdr = dst + old_len;
    memset(hdr, 0, add);
    memcpy(hdr, tail, tail_len);                       /* name */
    lotus_tar_write_octal((char *)hdr + 100, 8,
                          typeflag == '5' ? 0755 : 0644); /* mode */
    lotus_tar_write_octal((char *)hdr + 108, 8, 0);    /* uid */
    lotus_tar_write_octal((char *)hdr + 116, 8, 0);    /* gid */
    lotus_tar_write_octal((char *)hdr + 124, 12, body_len); /* size */
    lotus_tar_write_octal((char *)hdr + 136, 12, 0);   /* mtime */
    hdr[156] = (uint8_t)typeflag;
    memcpy(hdr + 257, "ustar", 6);                     /* magic + NUL */
    hdr[263] = '0';                                    /* version */
    hdr[264] = '0';
    if (prefix_len) {
        memcpy(hdr + 345, prefix_start, prefix_len);
    }
    /* Checksum: field treated as 8 spaces during the sum. */
    memset(hdr + 148, ' ', 8);
    uint64_t sum = 0;
    for (size_t k = 0; k < LOTUS_TAR_BLOCK; k++) sum += hdr[k];
    /* "%06o\0 " layout: 6 octal digits, NUL, space. */
    for (size_t k = 6; k-- > 0;) {
        hdr[148 + k] = (uint8_t)('0' + (sum & 7));
        sum >>= 3;
    }
    hdr[154] = '\0';
    hdr[155] = ' ';
    if (body_len > 0) {
        memcpy(hdr + LOTUS_TAR_BLOCK,
               lotus_bytes_data((void *)data_blob), (size_t)body_len);
    }
    return out;
}

void *lotus_tar_pack(const void *archive_blob, const char *name,
                     const void *data_blob) {
    if (!data_blob) { errno = EINVAL; return NULL; }
    return lotus_tar_pack_entry(archive_blob, name, data_blob, '0');
}

void *lotus_tar_pack_dir(const void *archive_blob, const char *name) {
    return lotus_tar_pack_entry(archive_blob, name, NULL, '5');
}

/* Append the two terminating zero blocks. */
void *lotus_tar_finish(const void *archive_blob) {
    if (!archive_blob) { errno = EINVAL; return NULL; }
    int64_t old_len = lotus_bytes_len(archive_blob);
    void *out = lotus_caller_or_global_bytes_create(
        old_len + 2 * LOTUS_TAR_BLOCK);
    if (!out) { errno = ENOMEM; return NULL; }
    uint8_t *dst = lotus_bytes_data(out);
    if (old_len > 0) {
        memcpy(dst, lotus_bytes_data((void *)archive_blob),
               (size_t)old_len);
    }
    memset(dst + old_len, 0, 2 * LOTUS_TAR_BLOCK);
    return out;
}
