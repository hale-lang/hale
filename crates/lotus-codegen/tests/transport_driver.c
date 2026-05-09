/*
 * m57: tiny C harness exercising lotus_transport_* from the
 * runtime. Built by tests/transport.rs into a single binary that
 * is then exec'd twice — once as listener, once as connector —
 * to verify round-trip bytes over an AF_UNIX SOCK_SEQPACKET pair.
 *
 * Forward-declare the transport surface here rather than carry a
 * runtime header file: m57 keeps the C-runtime install-free and
 * the surface is small enough that forward decls + linking
 * lotus_arena.c into the test binary is the lightest path.
 *
 * argv:
 *   listen  <socket_path>          -> recv one message, print to stdout
 *   connect <socket_path> <bytes>  -> send <bytes> as one message
 */

#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>

#define LOTUS_TRANSPORT_LISTEN  0
#define LOTUS_TRANSPORT_CONNECT 1

typedef struct lotus_transport lotus_transport_t;

lotus_transport_t *lotus_transport_create(const char *path, int role);
int     lotus_transport_send(lotus_transport_t *t, const void *buf, size_t len);
ssize_t lotus_transport_recv(lotus_transport_t *t, void *buf, size_t cap);
void    lotus_transport_destroy(lotus_transport_t *t);

#define BUF_CAP 4096

static int run_listen(const char *path) {
    lotus_transport_t *t =
        lotus_transport_create(path, LOTUS_TRANSPORT_LISTEN);
    if (!t) {
        fprintf(stderr, "listener: create failed\n");
        return 1;
    }
    char buf[BUF_CAP];
    ssize_t n = lotus_transport_recv(t, buf, sizeof(buf));
    if (n < 0) {
        fprintf(stderr, "listener: recv failed\n");
        lotus_transport_destroy(t);
        return 1;
    }
    /* Bytes-only: do not append a newline so the test can compare
     * the entire stdout to the sent payload exactly. */
    if (fwrite(buf, 1, (size_t)n, stdout) != (size_t)n) {
        fprintf(stderr, "listener: stdout write short\n");
        lotus_transport_destroy(t);
        return 1;
    }
    fflush(stdout);
    lotus_transport_destroy(t);
    return 0;
}

static int run_connect(const char *path, const char *msg) {
    lotus_transport_t *t =
        lotus_transport_create(path, LOTUS_TRANSPORT_CONNECT);
    if (!t) {
        fprintf(stderr, "connector: create failed\n");
        return 1;
    }
    size_t len = strlen(msg);
    if (lotus_transport_send(t, msg, len) != 0) {
        fprintf(stderr, "connector: send failed\n");
        lotus_transport_destroy(t);
        return 1;
    }
    lotus_transport_destroy(t);
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr,
                "usage: %s listen <path> | connect <path> <bytes>\n",
                argv[0]);
        return 2;
    }
    if (strcmp(argv[1], "listen") == 0) {
        return run_listen(argv[2]);
    }
    if (strcmp(argv[1], "connect") == 0) {
        if (argc < 4) {
            fprintf(stderr, "connect: missing <bytes> argument\n");
            return 2;
        }
        return run_connect(argv[2], argv[3]);
    }
    fprintf(stderr, "unknown role: %s\n", argv[1]);
    return 2;
}
