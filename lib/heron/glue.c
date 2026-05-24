// glue.c — flat C surface over tree-sitter's runtime API.
//
// Bridges Hale's @ffi shape (single-pointer args/returns) to
// tree-sitter's API, which uses TSNode by-value structs that
// don't translate cleanly to Hale's binding surface.
//
// Strategy: heap-allocate small TSNode wrappers via malloc.
// Long-lived; freed on Tree disposal or never (the leak is
// bounded by tree size, which is bounded by source size).
// Production-grade refinement (per-tree arena, ref-counted
// nodes) waits on a workload that measures the leak as
// load-bearing.

#include <tree_sitter/api.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

// The generated parser entry point.
extern const TSLanguage *tree_sitter_hale(void);

// ---- Tree wrapper holds the parsed tree + owned source copy ----
//
// tree-sitter's TSNode carries pointers into the source string;
// the source must outlive the tree. We keep a copy here.

typedef struct {
    TSTree *tree;
    char   *source;
    size_t  source_len;
} tsa_tree_t;

// ---- Node wrapper — heap-allocated TSNode + owning tree pointer ----
//
// We carry a pointer to the owning tree so we can hand back
// source-byte ranges (the tree wrapper owns the source string).

typedef struct {
    TSNode      node;
    tsa_tree_t *owner;
} tsa_node_t;

// ---- Parser ----

void *tsa_parser_new(void) {
    TSParser *parser = ts_parser_new();
    if (parser == NULL) { return NULL; }
    if (!ts_parser_set_language(parser, tree_sitter_hale())) {
        ts_parser_delete(parser);
        return NULL;
    }
    return parser;
}

void tsa_parser_delete(void *parser) {
    if (parser != NULL) {
        ts_parser_delete((TSParser *)parser);
    }
}

void *tsa_parser_parse(void *parser, const char *source, int64_t length) {
    if (parser == NULL || source == NULL || length < 0) { return NULL; }

    tsa_tree_t *wrapper = (tsa_tree_t *)malloc(sizeof(tsa_tree_t));
    if (wrapper == NULL) { return NULL; }

    wrapper->source = (char *)malloc((size_t)length + 1);
    if (wrapper->source == NULL) {
        free(wrapper);
        return NULL;
    }
    memcpy(wrapper->source, source, (size_t)length);
    wrapper->source[length] = '\0';
    wrapper->source_len = (size_t)length;

    wrapper->tree = ts_parser_parse_string(
        (TSParser *)parser, NULL, wrapper->source, (uint32_t)length);

    if (wrapper->tree == NULL) {
        free(wrapper->source);
        free(wrapper);
        return NULL;
    }

    return wrapper;
}

// ---- Tree ----

void tsa_tree_delete(void *tree) {
    if (tree == NULL) { return; }
    tsa_tree_t *wrapper = (tsa_tree_t *)tree;
    if (wrapper->tree != NULL) { ts_tree_delete(wrapper->tree); }
    if (wrapper->source != NULL) { free(wrapper->source); }
    free(wrapper);
}

void *tsa_tree_root(void *tree) {
    if (tree == NULL) { return NULL; }
    tsa_tree_t *wrapper = (tsa_tree_t *)tree;

    tsa_node_t *node = (tsa_node_t *)malloc(sizeof(tsa_node_t));
    if (node == NULL) { return NULL; }
    node->node = ts_tree_root_node(wrapper->tree);
    node->owner = wrapper;
    return node;
}

// ---- Node accessors ----

const char *tsa_node_kind(void *node) {
    if (node == NULL) { return ""; }
    return ts_node_type(((tsa_node_t *)node)->node);
}

int64_t tsa_node_start_byte(void *node) {
    if (node == NULL) { return -1; }
    return (int64_t)ts_node_start_byte(((tsa_node_t *)node)->node);
}

int64_t tsa_node_end_byte(void *node) {
    if (node == NULL) { return -1; }
    return (int64_t)ts_node_end_byte(((tsa_node_t *)node)->node);
}

int64_t tsa_node_start_row(void *node) {
    if (node == NULL) { return -1; }
    return (int64_t)ts_node_start_point(((tsa_node_t *)node)->node).row;
}

int64_t tsa_node_start_col(void *node) {
    if (node == NULL) { return -1; }
    return (int64_t)ts_node_start_point(((tsa_node_t *)node)->node).column;
}

int64_t tsa_node_child_count(void *node) {
    if (node == NULL) { return 0; }
    return (int64_t)ts_node_child_count(((tsa_node_t *)node)->node);
}

int64_t tsa_node_named_child_count(void *node) {
    if (node == NULL) { return 0; }
    return (int64_t)ts_node_named_child_count(((tsa_node_t *)node)->node);
}

// Returns a fresh tsa_node_t * for the i-th child. Caller takes
// ownership; release via tsa_node_delete when done. Returns NULL
// on out-of-bounds.
void *tsa_node_child(void *node, int64_t index) {
    if (node == NULL || index < 0) { return NULL; }
    tsa_node_t *parent = (tsa_node_t *)node;
    if ((uint32_t)index >= ts_node_child_count(parent->node)) { return NULL; }

    tsa_node_t *child = (tsa_node_t *)malloc(sizeof(tsa_node_t));
    if (child == NULL) { return NULL; }
    child->node = ts_node_child(parent->node, (uint32_t)index);
    child->owner = parent->owner;
    return child;
}

void *tsa_node_named_child(void *node, int64_t index) {
    if (node == NULL || index < 0) { return NULL; }
    tsa_node_t *parent = (tsa_node_t *)node;
    if ((uint32_t)index >= ts_node_named_child_count(parent->node)) { return NULL; }

    tsa_node_t *child = (tsa_node_t *)malloc(sizeof(tsa_node_t));
    if (child == NULL) { return NULL; }
    child->node = ts_node_named_child(parent->node, (uint32_t)index);
    child->owner = parent->owner;
    return child;
}

// Find a named field (e.g. "name", "type") on this node. Returns
// NULL if no such field. Caller owns the returned node.
void *tsa_node_field(void *node, const char *field_name) {
    if (node == NULL || field_name == NULL) { return NULL; }
    tsa_node_t *parent = (tsa_node_t *)node;

    TSNode child = ts_node_child_by_field_name(
        parent->node, field_name, (uint32_t)strlen(field_name));
    if (ts_node_is_null(child)) { return NULL; }

    tsa_node_t *wrapper = (tsa_node_t *)malloc(sizeof(tsa_node_t));
    if (wrapper == NULL) { return NULL; }
    wrapper->node = child;
    wrapper->owner = parent->owner;
    return wrapper;
}

// Return the source text covered by this node. Returns a copy
// the caller must free via tsa_string_free. Returns NULL on
// failure.
char *tsa_node_text(void *node) {
    if (node == NULL) { return NULL; }
    tsa_node_t *n = (tsa_node_t *)node;
    if (n->owner == NULL || n->owner->source == NULL) { return NULL; }

    uint32_t start = ts_node_start_byte(n->node);
    uint32_t end = ts_node_end_byte(n->node);
    if (end < start) { return NULL; }
    size_t length = (size_t)(end - start);

    char *result = (char *)malloc(length + 1);
    if (result == NULL) { return NULL; }
    memcpy(result, n->owner->source + start, length);
    result[length] = '\0';
    return result;
}

int tsa_node_is_null(void *node) {
    if (node == NULL) { return 1; }
    return ts_node_is_null(((tsa_node_t *)node)->node) ? 1 : 0;
}

int tsa_node_has_error(void *node) {
    if (node == NULL) { return 0; }
    return ts_node_has_error(((tsa_node_t *)node)->node) ? 1 : 0;
}

void tsa_node_delete(void *node) {
    if (node != NULL) { free(node); }
}

void tsa_string_free(char *s) {
    if (s != NULL) { free(s); }
}

// ============================================================
// Query — tree-sitter query API (queries/highlights.scm etc.)
// ============================================================
//
// A Query is a compiled .scm query document. Applying it
// against a Node produces a sequence of (node, capture_name)
// captures — the bridge to syntax highlighting / block
// spotlighting / structural navigation.
//
// We expose a simplified API: compile once, apply many times,
// return captures as a newline-separated text format the
// Hale caller parses inline:
//
//   <start_byte>:<end_byte>:<capture_name>\n
//
// For highlights.scm-style usage this is ~ a few KB per
// typical file. The string is malloc'd; caller must release
// via tsa_string_free when done (or accept the leak — bounded
// by the size of the input).

void *tsa_query_new(const char *source, int64_t length) {
    if (source == NULL || length < 0) { return NULL; }
    uint32_t error_offset = 0;
    TSQueryError error_type = TSQueryErrorNone;
    TSQuery *q = ts_query_new(
        tree_sitter_hale(),
        source,
        (uint32_t)length,
        &error_offset,
        &error_type);
    return q;
}

void tsa_query_delete(void *query) {
    if (query != NULL) {
        ts_query_delete((TSQuery *)query);
    }
}

// Apply the query to the given root node. Returns a malloc'd
// newline-separated string of "start:end:capture_name" rows.
// Caller frees via tsa_string_free.
char *tsa_query_apply(void *query, void *root_node) {
    if (query == NULL || root_node == NULL) {
        char *empty = (char *)malloc(1);
        if (empty != NULL) { empty[0] = '\0'; }
        return empty;
    }

    TSQuery *q = (TSQuery *)query;
    tsa_node_t *root = (tsa_node_t *)root_node;

    TSQueryCursor *cursor = ts_query_cursor_new();
    if (cursor == NULL) {
        char *empty = (char *)malloc(1);
        if (empty != NULL) { empty[0] = '\0'; }
        return empty;
    }
    ts_query_cursor_exec(cursor, q, root->node);

    // Grow a buffer as we emit rows. Start at 4KB.
    size_t cap = 4096;
    size_t len = 0;
    char *buf = (char *)malloc(cap);
    if (buf == NULL) {
        ts_query_cursor_delete(cursor);
        return NULL;
    }

    TSQueryMatch match;
    while (ts_query_cursor_next_match(cursor, &match)) {
        for (uint16_t i = 0; i < match.capture_count; i++) {
            TSQueryCapture cap_data = match.captures[i];
            uint32_t name_len = 0;
            const char *name = ts_query_capture_name_for_id(
                q, cap_data.index, &name_len);

            uint32_t start = ts_node_start_byte(cap_data.node);
            uint32_t end = ts_node_end_byte(cap_data.node);

            // Each row needs up to ~20 + 20 + name_len + 3 chars
            // for "start:end:name\n" with safe headroom.
            size_t needed = 64 + name_len;
            while (len + needed > cap) {
                size_t new_cap = cap * 2;
                char *new_buf = (char *)realloc(buf, new_cap);
                if (new_buf == NULL) {
                    free(buf);
                    ts_query_cursor_delete(cursor);
                    return NULL;
                }
                buf = new_buf;
                cap = new_cap;
            }

            int written = snprintf(buf + len, cap - len,
                "%u:%u:%.*s\n",
                start, end, (int)name_len, name);
            if (written < 0) { break; }
            len += (size_t)written;
        }
    }

    // NUL-terminate.
    if (len < cap) {
        buf[len] = '\0';
    } else {
        // Should never hit — we always reserved headroom.
        buf[cap - 1] = '\0';
    }

    ts_query_cursor_delete(cursor);
    return buf;
}
