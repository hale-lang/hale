/*
 * m74: tiny C harness exercising lotus_fs_* from the runtime.
 * Built by tests/fs.rs into a single binary that is exec'd
 * with one of four modes:
 *
 *   read   <path>            -> read file, print bytes to stdout
 *   write  <path> <bytes>    -> write `bytes` to file
 *   size   <path>            -> print file size
 *   exists <path>            -> print 1 or 0
 *
 * Forward-declare the fs surface here rather than carry a
 * runtime header file: m74 keeps the C-runtime install-free
 * and the surface is small enough that forward decls + linking
 * lotus_arena.c into the test binary is the lightest path.
 */

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>

ssize_t lotus_fs_read_file(const char *path, void *out_buf, size_t out_cap);
int     lotus_fs_write_file(const char *path, const void *buf, size_t len);
int64_t lotus_fs_file_size(const char *path);
int     lotus_fs_file_exists(const char *path);

#define BUF_CAP (1024 * 1024)

static int run_read(const char *path) {
    char *buf = (char *)malloc(BUF_CAP);
    if (!buf) {
        fprintf(stderr, "read: alloc\n");
        return 1;
    }
    ssize_t n = lotus_fs_read_file(path, buf, BUF_CAP);
    if (n < 0) {
        fprintf(stderr, "read: failed\n");
        free(buf);
        return 1;
    }
    if (fwrite(buf, 1, (size_t)n, stdout) != (size_t)n) {
        fprintf(stderr, "read: stdout short\n");
        free(buf);
        return 1;
    }
    fflush(stdout);
    free(buf);
    return 0;
}

static int run_write(const char *path, const char *bytes) {
    size_t len = strlen(bytes);
    if (lotus_fs_write_file(path, bytes, len) != 0) {
        fprintf(stderr, "write: failed\n");
        return 1;
    }
    return 0;
}

static int run_size(const char *path) {
    int64_t s = lotus_fs_file_size(path);
    if (s < 0) {
        fprintf(stderr, "size: failed\n");
        return 1;
    }
    printf("%lld\n", (long long)s);
    return 0;
}

static int run_exists(const char *path) {
    printf("%d\n", lotus_fs_file_exists(path));
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr,
                "usage: %s read <path> | write <path> <bytes> | size <path> | exists <path>\n",
                argv[0]);
        return 2;
    }
    const char *cmd = argv[1];
    const char *path = argv[2];
    if (strcmp(cmd, "read") == 0) {
        return run_read(path);
    }
    if (strcmp(cmd, "write") == 0) {
        if (argc < 4) {
            fprintf(stderr, "write: need <bytes> argument\n");
            return 2;
        }
        return run_write(path, argv[3]);
    }
    if (strcmp(cmd, "size") == 0) {
        return run_size(path);
    }
    if (strcmp(cmd, "exists") == 0) {
        return run_exists(path);
    }
    fprintf(stderr, "unknown command: %s\n", cmd);
    return 2;
}
