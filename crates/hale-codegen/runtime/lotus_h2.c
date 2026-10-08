/*
 * lotus_h2.c -- an HTTP/2 server session, sans-I/O.
 *
 * nghttp2 (MIT, vendored under runtime/third_party/nghttp2) is the protocol:
 * frames, HPACK, flow-control windows, SETTINGS and PING acknowledgement,
 * GOAWAY. This file is the whole of the glue, and it owns no thread, no
 * socket and no blocking call: the session is bytes in (`lotus_h2_feed`),
 * bytes out (`lotus_h2_drain`) and a queue of events (`lotus_h2_poll`), all
 * of them computation on the thread that calls them. The socket stays an fd
 * the caller parks on the async_io pool; nothing here reads or writes one.
 *
 * Every function takes the session as an i64 (the pointer, as the
 * BytesBuilder's handle is) and a message as a Bytes blob ([i64 len][body]).
 *
 * Events (`lotus_h2_poll` returns the kind, 0 when the queue is empty, and
 * the accessors then describe that event until the next poll):
 *   1 OPEN    a request's headers are whole; bytes = "name: value\n" lines,
 *             pseudo-headers (":method", ":path", ...) first
 *   2 DATA    bytes of the stream's body
 *   3 END     the peer will send nothing more on the stream
 *   4 RESET   the peer reset the stream; code = its error code
 *   5 GOAWAY  the peer is going away; code = its error code, stream = the
 *             last stream it will see
 *   6 CLOSED  the stream is over (any cause); code = nghttp2's error code
 */

#include <nghttp2/nghttp2.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

extern void *lotus_caller_or_global_bytes_create(int64_t len);
extern int64_t lotus_bytes_len(const void *b);
extern void *lotus_bytes_data(void *b);

typedef struct h2_event {
    int kind;
    int32_t stream;
    int64_t code;
    char *data;
    size_t len;
    struct h2_event *next;
} h2_event;

/* What a stream keeps between its callbacks: the request's header text as it
 * arrives, and the response's body and trailers while they drain. */
typedef struct h2_stream {
    int32_t id;
    char *hdr;
    size_t hlen, hcap;
    char *body;
    size_t blen, boff;
    char *trailers;
    size_t tlen;
    struct h2_stream *next;
} h2_stream;

typedef struct h2_session {
    nghttp2_session *sess;
    uint8_t *out;
    size_t outlen, outcap;
    h2_event *head, *tail, *cur;
    h2_stream *streams;
} h2_session;

static h2_stream *stream_find(h2_session *s, int32_t id) {
    for (h2_stream *t = s->streams; t; t = t->next) {
        if (t->id == id) return t;
    }
    return NULL;
}

static h2_stream *stream_add(h2_session *s, int32_t id) {
    h2_stream *t = (h2_stream *)calloc(1, sizeof *t);
    if (!t) return NULL;
    t->id = id;
    t->next = s->streams;
    s->streams = t;
    return t;
}

static void stream_free(h2_stream *t) {
    free(t->hdr);
    free(t->body);
    free(t->trailers);
    free(t);
}

static void stream_drop(h2_session *s, int32_t id) {
    h2_stream **pp = &s->streams;
    while (*pp) {
        if ((*pp)->id == id) {
            h2_stream *t = *pp;
            *pp = t->next;
            stream_free(t);
            return;
        }
        pp = &(*pp)->next;
    }
}

static void event_push(h2_session *s, int kind, int32_t stream, int64_t code,
                       char *data, size_t len) {
    h2_event *e = (h2_event *)calloc(1, sizeof *e);
    if (!e) {
        free(data);
        return;
    }
    e->kind = kind;
    e->stream = stream;
    e->code = code;
    e->data = data;
    e->len = len;
    if (s->tail) s->tail->next = e; else s->head = e;
    s->tail = e;
}

static void event_free(h2_event *e) {
    if (!e) return;
    free(e->data);
    free(e);
}

static void *make_bytes(const void *p, size_t n) {
    void *b = lotus_caller_or_global_bytes_create((int64_t)n);
    if (b && n) memcpy(lotus_bytes_data(b), p, n);
    return b;
}

static int hdr_append(h2_stream *t, const uint8_t *name, size_t nl,
                      const uint8_t *val, size_t vl) {
    size_t need = t->hlen + nl + 2 + vl + 1;
    if (need > t->hcap) {
        size_t cap = t->hcap ? t->hcap : 256;
        while (cap < need) cap *= 2;
        char *p = (char *)realloc(t->hdr, cap);
        if (!p) return -1;
        t->hdr = p;
        t->hcap = cap;
    }
    memcpy(t->hdr + t->hlen, name, nl);
    t->hlen += nl;
    t->hdr[t->hlen++] = ':';
    t->hdr[t->hlen++] = ' ';
    memcpy(t->hdr + t->hlen, val, vl);
    t->hlen += vl;
    t->hdr[t->hlen++] = '\n';
    return 0;
}

static int cb_begin_headers(nghttp2_session *ng, const nghttp2_frame *f, void *ud) {
    h2_session *s = (h2_session *)ud;
    (void)ng;
    if (f->hd.type == NGHTTP2_HEADERS && f->headers.cat == NGHTTP2_HCAT_REQUEST) {
        if (!stream_find(s, f->hd.stream_id) && !stream_add(s, f->hd.stream_id)) {
            return NGHTTP2_ERR_CALLBACK_FAILURE;
        }
    }
    return 0;
}

static int cb_header(nghttp2_session *ng, const nghttp2_frame *f,
                     const uint8_t *name, size_t nl, const uint8_t *val,
                     size_t vl, uint8_t flags, void *ud) {
    h2_session *s = (h2_session *)ud;
    (void)ng;
    (void)flags;
    if (f->hd.type != NGHTTP2_HEADERS || f->headers.cat != NGHTTP2_HCAT_REQUEST) return 0;
    h2_stream *t = stream_find(s, f->hd.stream_id);
    if (!t) return 0;
    return hdr_append(t, name, nl, val, vl) ? NGHTTP2_ERR_CALLBACK_FAILURE : 0;
}

static int cb_data_chunk(nghttp2_session *ng, uint8_t flags, int32_t id,
                         const uint8_t *data, size_t len, void *ud) {
    h2_session *s = (h2_session *)ud;
    (void)ng;
    (void)flags;
    char *copy = (char *)malloc(len ? len : 1);
    if (!copy) return NGHTTP2_ERR_CALLBACK_FAILURE;
    if (len) memcpy(copy, data, len);
    event_push(s, 2, id, 0, copy, len);
    return 0;
}

static int cb_frame_recv(nghttp2_session *ng, const nghttp2_frame *f, void *ud) {
    h2_session *s = (h2_session *)ud;
    (void)ng;
    switch (f->hd.type) {
    case NGHTTP2_HEADERS:
        if (f->headers.cat == NGHTTP2_HCAT_REQUEST) {
            h2_stream *t = stream_find(s, f->hd.stream_id);
            char *text = NULL;
            size_t n = 0;
            if (t && t->hlen) {
                text = (char *)malloc(t->hlen);
                if (!text) return NGHTTP2_ERR_CALLBACK_FAILURE;
                memcpy(text, t->hdr, t->hlen);
                n = t->hlen;
            }
            event_push(s, 1, f->hd.stream_id, 0, text, n);
        }
        if (f->hd.flags & NGHTTP2_FLAG_END_STREAM) event_push(s, 3, f->hd.stream_id, 0, NULL, 0);
        break;
    case NGHTTP2_DATA:
        if (f->hd.flags & NGHTTP2_FLAG_END_STREAM) event_push(s, 3, f->hd.stream_id, 0, NULL, 0);
        break;
    case NGHTTP2_RST_STREAM:
        event_push(s, 4, f->hd.stream_id, (int64_t)f->rst_stream.error_code, NULL, 0);
        break;
    case NGHTTP2_GOAWAY:
        event_push(s, 5, f->goaway.last_stream_id, (int64_t)f->goaway.error_code, NULL, 0);
        break;
    default:
        break;
    }
    return 0;
}

static int cb_stream_close(nghttp2_session *ng, int32_t id, uint32_t code, void *ud) {
    h2_session *s = (h2_session *)ud;
    (void)ng;
    event_push(s, 6, id, (int64_t)code, NULL, 0);
    stream_drop(s, id);
    return 0;
}

int64_t lotus_h2_server_open(void) {
    h2_session *s = (h2_session *)calloc(1, sizeof *s);
    if (!s) return 0;
    nghttp2_session_callbacks *cbs = NULL;
    if (nghttp2_session_callbacks_new(&cbs)) {
        free(s);
        return 0;
    }
    nghttp2_session_callbacks_set_on_begin_headers_callback(cbs, cb_begin_headers);
    nghttp2_session_callbacks_set_on_header_callback(cbs, cb_header);
    nghttp2_session_callbacks_set_on_data_chunk_recv_callback(cbs, cb_data_chunk);
    nghttp2_session_callbacks_set_on_frame_recv_callback(cbs, cb_frame_recv);
    nghttp2_session_callbacks_set_on_stream_close_callback(cbs, cb_stream_close);
    int rv = nghttp2_session_server_new(&s->sess, cbs, s);
    nghttp2_session_callbacks_del(cbs);
    if (rv) {
        free(s);
        return 0;
    }
    /* a unary call's message is at most a few MiB; the default 64 KiB
     * window would stall every larger one on a round trip */
    nghttp2_settings_entry iv[3] = {
        {NGHTTP2_SETTINGS_MAX_CONCURRENT_STREAMS, 128},
        {NGHTTP2_SETTINGS_INITIAL_WINDOW_SIZE, 1048576},
        {NGHTTP2_SETTINGS_MAX_HEADER_LIST_SIZE, 65536},
    };
    nghttp2_submit_settings(s->sess, NGHTTP2_FLAG_NONE, iv, 3);
    nghttp2_session_set_local_window_size(s->sess, NGHTTP2_FLAG_NONE, 0, 1048576);
    return (int64_t)(intptr_t)s;
}

/* Bytes received from the peer: the count the session consumed, or a
 * negative nghttp2 error (the connection is over). */
int64_t lotus_h2_feed(int64_t h, const void *blob) {
    h2_session *s = (h2_session *)(intptr_t)h;
    if (!s || !blob) return -1;
    int64_t n = lotus_bytes_len(blob);
    if (n == 0) return 0;
    return (int64_t)nghttp2_session_mem_recv2(s->sess, (const uint8_t *)lotus_bytes_data((void *)blob), (size_t)n);
}

/* Everything the session has to send, as one blob (empty when nothing). */
void *lotus_h2_drain(int64_t h) {
    h2_session *s = (h2_session *)(intptr_t)h;
    if (!s) return make_bytes("", 0);
    for (;;) {
        const uint8_t *p = NULL;
        nghttp2_ssize n = nghttp2_session_mem_send2(s->sess, &p);
        if (n <= 0) break;
        if (s->outlen + (size_t)n > s->outcap) {
            size_t cap = s->outcap ? s->outcap : 4096;
            while (cap < s->outlen + (size_t)n) cap *= 2;
            uint8_t *q = (uint8_t *)realloc(s->out, cap);
            if (!q) break;
            s->out = q;
            s->outcap = cap;
        }
        memcpy(s->out + s->outlen, p, (size_t)n);
        s->outlen += (size_t)n;
    }
    void *b = make_bytes(s->out, s->outlen);
    s->outlen = 0;
    return b;
}

int64_t lotus_h2_poll(int64_t h) {
    h2_session *s = (h2_session *)(intptr_t)h;
    if (!s) return 0;
    event_free(s->cur);
    s->cur = s->head;
    if (!s->cur) return 0;
    s->head = s->cur->next;
    if (!s->head) s->tail = NULL;
    s->cur->next = NULL;
    return s->cur->kind;
}

int64_t lotus_h2_ev_stream(int64_t h) {
    h2_session *s = (h2_session *)(intptr_t)h;
    return s && s->cur ? s->cur->stream : 0;
}

int64_t lotus_h2_ev_code(int64_t h) {
    h2_session *s = (h2_session *)(intptr_t)h;
    return s && s->cur ? s->cur->code : 0;
}

void *lotus_h2_ev_bytes(int64_t h) {
    h2_session *s = (h2_session *)(intptr_t)h;
    if (!s || !s->cur || !s->cur->data) return make_bytes("", 0);
    return make_bytes(s->cur->data, s->cur->len);
}

/* "name: value\n" lines to a name/value array pointing into `text` (which
 * is modified: each line's separators become NULs). */
static nghttp2_nv *parse_nv(char *text, size_t len, size_t *count) {
    size_t lines = 0;
    for (size_t i = 0; i < len; i++) if (text[i] == '\n') lines++;
    *count = 0;
    nghttp2_nv *nv = (nghttp2_nv *)calloc(lines ? lines : 1, sizeof *nv);
    if (!nv) return NULL;
    size_t at = 0;
    while (at < len) {
        char *line = text + at;
        char *nl = (char *)memchr(line, '\n', len - at);
        if (!nl) break;
        size_t ll = (size_t)(nl - line);
        at += ll + 1;
        /* the separator: the first ": " past a leading ':' of a pseudo-header */
        size_t sep = 0;
        int found = 0;
        for (size_t i = 1; i + 1 < ll; i++) {
            if (line[i] == ':' && line[i + 1] == ' ') { sep = i; found = 1; break; }
        }
        if (!found) continue;
        nv[*count].name = (uint8_t *)line;
        nv[*count].namelen = sep;
        nv[*count].value = (uint8_t *)(line + sep + 2);
        nv[*count].valuelen = ll - sep - 2;
        nv[*count].flags = NGHTTP2_NV_FLAG_NONE;
        (*count)++;
    }
    return nv;
}

static nghttp2_ssize body_read(nghttp2_session *ng, int32_t id, uint8_t *buf,
                               size_t length, uint32_t *flags,
                               nghttp2_data_source *src, void *ud) {
    h2_stream *t = (h2_stream *)src->ptr;
    (void)ud;
    size_t left = t->blen - t->boff;
    size_t n = left < length ? left : length;
    if (n) memcpy(buf, t->body + t->boff, n);
    t->boff += n;
    if (t->boff == t->blen) {
        *flags |= NGHTTP2_DATA_FLAG_EOF;
        if (t->tlen) {
            size_t cnt = 0;
            nghttp2_nv *nv = parse_nv(t->trailers, t->tlen, &cnt);
            if (!nv) return NGHTTP2_ERR_CALLBACK_FAILURE;
            *flags |= NGHTTP2_DATA_FLAG_NO_END_STREAM;
            int rv = nghttp2_submit_trailer(ng, id, nv, cnt);
            free(nv);
            if (rv) return NGHTTP2_ERR_CALLBACK_FAILURE;
        }
    }
    return (nghttp2_ssize)n;
}

static char *dup_blob(const void *blob, size_t *len) {
    *len = blob ? (size_t)lotus_bytes_len(blob) : 0;
    char *p = (char *)malloc(*len ? *len : 1);
    if (p && *len) memcpy(p, lotus_bytes_data((void *)blob), *len);
    return p;
}

/* Answer a stream: HEADERS (the ":status" line among them), then the body
 * as DATA, then the trailers as a closing HEADERS; with no body and no
 * trailers the HEADERS closes the stream (a trailers-only answer is the
 * caller's headers alone). 0 on success. */
int64_t lotus_h2_respond(int64_t h, int64_t stream, const void *headers,
                         const void *body, const void *trailers) {
    h2_session *s = (h2_session *)(intptr_t)h;
    if (!s) return -1;
    int32_t id = (int32_t)stream;
    h2_stream *t = stream_find(s, id);
    if (!t) t = stream_add(s, id);
    if (!t) return -2;
    free(t->body);
    free(t->trailers);
    t->body = dup_blob(body, &t->blen);
    t->trailers = dup_blob(trailers, &t->tlen);
    t->boff = 0;
    size_t hl = 0;
    char *htext = dup_blob(headers, &hl);
    if (!t->body || !t->trailers || !htext) {
        free(htext);
        return -2;
    }
    size_t cnt = 0;
    nghttp2_nv *nv = parse_nv(htext, hl, &cnt);
    if (!nv) {
        free(htext);
        return -2;
    }
    int rv;
    if (t->blen == 0 && t->tlen == 0) {
        rv = nghttp2_submit_response2(s->sess, id, nv, cnt, NULL);
    } else {
        nghttp2_data_provider2 prd;
        prd.source.ptr = t;
        prd.read_callback = body_read;
        rv = nghttp2_submit_response2(s->sess, id, nv, cnt, &prd);
    }
    free(nv);
    free(htext);
    return rv;
}

int64_t lotus_h2_reset(int64_t h, int64_t stream, int64_t code) {
    h2_session *s = (h2_session *)(intptr_t)h;
    if (!s) return -1;
    return nghttp2_submit_rst_stream(s->sess, NGHTTP2_FLAG_NONE, (int32_t)stream, (uint32_t)code);
}

/* GOAWAY for the streams the session has seen, queued behind every frame
 * already submitted: the answers in flight still go out. */
int64_t lotus_h2_goaway(int64_t h, int64_t code) {
    h2_session *s = (h2_session *)(intptr_t)h;
    if (!s) return -1;
    return nghttp2_submit_goaway(s->sess, NGHTTP2_FLAG_NONE,
                                 nghttp2_session_get_last_proc_stream_id(s->sess),
                                 (uint32_t)code, NULL, 0);
}

/* 1 while the session wants to read or has bytes to write. */
int64_t lotus_h2_alive(int64_t h) {
    h2_session *s = (h2_session *)(intptr_t)h;
    if (!s) return 0;
    return nghttp2_session_want_read(s->sess) || nghttp2_session_want_write(s->sess) ? 1 : 0;
}

int64_t lotus_h2_close(int64_t h) {
    h2_session *s = (h2_session *)(intptr_t)h;
    if (!s) return 0;
    nghttp2_session_del(s->sess);
    while (s->head) {
        h2_event *e = s->head;
        s->head = e->next;
        event_free(e);
    }
    event_free(s->cur);
    while (s->streams) {
        h2_stream *t = s->streams;
        s->streams = t->next;
        stream_free(t);
    }
    free(s->out);
    free(s);
    return 1;
}
