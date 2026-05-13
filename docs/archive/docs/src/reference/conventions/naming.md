# Naming

| Construct                | Convention                              | Example                  |
|--------------------------|-----------------------------------------|--------------------------|
| Locus (any kind)         | `<Name>L` suffix                        | `OnboardL`, `ChatServerL` |
| Type (shape record)      | PascalCase, no suffix                   | `Request`, `Response`    |
| Stdlib mangled internal  | `__Std<Domain><Name>`                   | `__StdHttpRequest`       |
| Locus method / type field | snake_case                             | `name_to_motion`         |
| Lifecycle method         | drop `fn` keyword                       | `run() { ... }`          |
| Free helper fn           | `__name` (leading underscores)          | `__drive`, `__walk`      |
| Bus subject              | dot-separated, lowercase                | `chat.message`           |
| Constants                | UPPER_SNAKE in stdlib; rare elsewhere   | `STDLIB_AP_SOURCE`       |

The leading `__` on free helpers is doing two jobs:

1. Marking them as implementation detail — don't call across
   seed boundaries.
2. Avoiding name conflicts with stdlib path-call dispatch in
   user code.

When user-defined seeds ship and the manifest becomes the
export source of truth, this convention may relax for
non-stdlib code.

## See Also

- [Pattern catalog](./patterns.md) — every pattern's example
  uses these conventions.
- [Seeds and exports](./seeds.md) — the seed boundary that
  the `__` prefix marks.
