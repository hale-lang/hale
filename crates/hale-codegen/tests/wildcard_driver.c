/* Parity driver for `lotus_wildcard_match`.
 *
 * Reads `pattern\tsubject` lines on stdin, prints `0` or `1` per
 * line. The Rust side (`wildcard_match_parity.rs`) feeds the same
 * case table through `hale_types::wildcard_match` and requires the
 * two to agree on every row.
 *
 * Why this exists: the runtime enforces which subjects a locus may
 * publish to, and the MODEL reasons about the same question using
 * the Rust matcher. If the two drifted, a publish the model believed
 * impossible would be allowed at runtime — which is precisely the
 * defect the enforcement was added to close. */
#include <stdio.h>
#include <string.h>
#include <stdint.h>

extern int64_t lotus_wildcard_match(const char *pattern,
                                    const char *subject);

int main(void) {
    char line[1024];
    while (fgets(line, sizeof line, stdin)) {
        size_t n = strlen(line);
        while (n > 0 && (line[n - 1] == '\n' || line[n - 1] == '\r')) {
            line[--n] = '\0';
        }
        char *tab = strchr(line, '\t');
        if (!tab) continue;
        *tab = '\0';
        printf("%d\n", (int)lotus_wildcard_match(line, tab + 1));
    }
    return 0;
}
