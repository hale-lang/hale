# Part I: Language & Standard Library

Part I covers Hale's core syntax, type system, error handling, data structures, and built-in standard library (`std::*`). Everything in this part works in standalone scripts, CLI tools, and long-lived services alike.

```hale
fn main() {
    let total = 19.99d * 3.0d;
    println("Three at $19.99 is $", total);
}
```

## Key Design Differences

1. **First-class financial, temporal, and physical types:** `Decimal` (`19.99d`), `Duration` (`250ms`, `5s`), `Time`, and user-defined `dimension`/`unit`/`quantity` types are built into the language rather than bolted on as wrapper libraries.
2. **Value-level errors (`fallible(E)`):** Functions that can fail declare `fallible(ErrType)` in their signature. Callers must explicitly handle the error channel at the call site using `or` (`or default`, `or raise`, `or handler(err)`, `or { ... }`, `or discard`).
3. **Arena-backed collections (`@form`):** Instead of heap-allocated generic `Vec<T>` or `HashMap<K, V>` values, dynamic collections are loci annotated with `@form(vec)`, `@form(hashmap)`, `@form(set)`, `@form(ring_buffer)`, `@form(lru_cache)`, or `@form(heap)`.

## Chapters in Part I

| Section | Chapters |
|---|---|
| **Core Language** | [Values, Types & Variables](../basics/values.md) · [Numbers, Money & Time](../basics/math.md) · [Units of Measure](../units.md) · [Functions](../basics/functions.md) · [Control Flow & Pattern Matching](../basics/control-flow.md) · [Strings & Bytes](../basics/strings.md) · [Error Handling (`fallible`)](../basics/fallible.md) · [Program Structure & `main`](../basics/first-program.md) |
| **Data & Components** | [Introduction to Loci](../everyday/locus-gently.md) · [Records, Enums & Set Types](../everyday/records.md) · [Collections (`@form`)](../everyday/collections.md) |
| **I/O, Networking & Web** | [Filesystem I/O](../everyday/files.md) · [JSON Encoding & Decoding](../everyday/json.md) · [HTTP Servers & Clients](../everyday/http.md) · [TCP, UDP, Unix Sockets & TLS](../everyday/sockets.md) · [Cryptography, Hashing & Randomness](../everyday/crypto.md) |
| **CLI, Telemetry & Testing** | [CLI Arguments & Environment](../everyday/cli-config.md) · [Structured Logging](../everyday/logging.md) · [Metrics](../everyday/metrics.md) · [Observability & Probes](../everyday/observing.md) · [Testing & Benchmarking](../everyday/testing.md) |
