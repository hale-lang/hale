# Part I: The Language

Part I covers the language itself: values and types, functions, control flow and pattern matching, records and enums, text, numbers and units, error handling, collections, generics, a first look at loci, and testing. Everything in this part works in standalone scripts, CLI tools, and long-lived services alike. The `std::*` modules for I/O, networking and telemetry are in [Part II](./stdlib.md).

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
| **Basics** | [Values, Types & Variables](../basics/values.md) · [Functions](../basics/functions.md) · [Control Flow & Pattern Matching](../basics/control-flow.md) · [Records, Enums & Set Types](../everyday/records.md) |
| **Data** | [Strings & Bytes](../basics/strings.md) · [Numbers, Money & Time](../basics/math.md) · [Units of Measure](../units.md) · [Error Handling (`fallible`)](../basics/fallible.md) · [Collections (`@form`)](../everyday/collections.md) · [Generics](../everyday/generics.md) |
| **Components & Testing** | [Introduction to Loci](../everyday/locus-gently.md) · [Testing & Benchmarking](../everyday/testing.md) |

Next: [Values, Types & Variables](../basics/values.md).
