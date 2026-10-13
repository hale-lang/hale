/* The IPv6 zone round trip of std::io::udp's source address (see
 * inet6_scope.rs): `lotus_inet6_format` writes a link-local address with
 * its zone, and `lotus_inet_resolve` -- what `std::io::udp::send` and
 * `std::io::tcp::connect` resolve a host with -- reads it back to the
 * same address and scope id. Both are static, so the runtime is included
 * here rather than linked. Prints one line per case. */
#include "../runtime/lotus_arena.c"

static void roundtrip(const char *tag, const char *addr, unsigned scope) {
    struct sockaddr_in6 a;
    memset(&a, 0, sizeof(a));
    a.sin6_family = AF_INET6;
    inet_pton(AF_INET6, addr, &a.sin6_addr);
    a.sin6_scope_id = scope;
    char text[64];
    if (lotus_inet6_format(&a, text, sizeof(text)) < 0) {
        printf("%s format failed\n", tag);
        return;
    }
    lotus_inet_addrs r;
    if (lotus_inet_resolve(text, 9, SOCK_DGRAM, 0, 1, "inet6_scope", &r) < 0) {
        printf("%s text=%s resolve failed\n", tag, text);
        return;
    }
    const struct sockaddr_in6 *b = (const struct sockaddr_in6 *)&r.addr[0];
    int same = r.addr[0].ss_family == AF_INET6
        && memcmp(&a.sin6_addr, &b->sin6_addr, sizeof(a.sin6_addr)) == 0
        && b->sin6_scope_id == scope
        && ntohs(b->sin6_port) == 9;
    printf("%s text=%s roundtrip=%s\n", tag, text, same ? "same" : "differs");
}

int main(void) {
    unsigned lo = if_nametoindex("lo");
    const char *lo_name = "lo";
    if (lo == 0) {
        lo = if_nametoindex("lo0");
        lo_name = "lo0";
    }
    printf("loopback=%s\n", lo_name);
    roundtrip("named", "fe80::1", lo);
    roundtrip("index", "fe80::1", 4000000);
    roundtrip("unscoped", "2001:db8::1", 0);
    return 0;
}
