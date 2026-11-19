# Part II: The Standard Library

Part II covers the `std::*` modules a program uses to reach the outside world: arguments and environment, files, JSON, HTTP, sockets and TLS, cryptography, and the telemetry a running program emits. Everything here builds on Part I and works the same in a short script as in a long-lived service.

```hale
fn main() {
    let user = std::env::var("USER");
    let name = if user == "" { "world" } else { user };
    println(f"hello, {name}");
}
```

## Chapters in Part II

| Section | Chapters |
|---|---|
| **Inputs** | [CLI Arguments & Environment](../everyday/cli-config.md) · [Filesystem I/O](../everyday/files.md) · [JSON Encoding & Decoding](../everyday/json.md) |
| **Networking** | [HTTP Servers & Clients](../everyday/http.md) · [TCP, UDP, Unix Sockets & TLS](../everyday/sockets.md) · [Cryptography, Hashing & Randomness](../everyday/crypto.md) |
| **Telemetry** | [Structured Logging](../everyday/logging.md) · [Metrics](../everyday/metrics.md) · [Observability & Probes](../everyday/observing.md) |

Next: [CLI Arguments & Environment](../everyday/cli-config.md).
