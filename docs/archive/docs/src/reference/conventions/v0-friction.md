# v0 friction

Workarounds for current language gaps. Each will go away as
the language fills in; until then, this page documents the
v0 idiom so contributors don't repeatedly rediscover them.

## Lifecycle bodies reject `return`

`birth()`, `run()`, and `dissolve()` cannot use `return`.
Short-circuit logic factors into a free helper:

```aperio
locus AppL {
    params { /* ... */ }
    run() {
        __drive(self.dir);   // helper does the early-return work
    }
}

fn __drive(dir: String) {
    if dir == "" { return; }
    // ...
}
```

## No user-defined seeds yet

Only `std::*` exists at v0. Shared loci between apps must live
in the std seed (bundled at codegen) or get duplicated in apps.
See [seeds and exports](./seeds.md) for the v1+ plan.

## No multi-file Aperio modules

An app is a single `apps/<name>/main.ap` file; cross-app shared
code goes through the std seed.

## No `List<T>` generic

Manual newline-separated `String` accumulators are the v0 idiom
for "list of things." The tagged-accumulator pattern in apps
like `apps/onboard/main.ap` is the canonical shape. Generics
are tracked, not shipped.

## No methods on `type` records

Use a locus with empty `params { }` instead. The cost is one
allocation per instantiation. Negligible.

## Empty `if` bodies parse-fail

Put a `// note` comment inside, or refactor to a positive
condition.

## `aperio run` rejects qualified-name literals

`std::ts::*` and `std::lang::*` qualified literals don't work
under `aperio run`. Use `aperio build` then run the resulting
binary directly. Tracked friction.

## No char-level access on `String`

There is no `s[i]` for a single character. Use `s[i..i+1]` for
a one-char slice; compare via `==` or `std::str::index_of`.

## No `std::str::trim` or `to_lower` builtins

Open-code these in a namespace lotus method until the stdlib
adds them.

## Fn-pointer callbacks can't capture state

Options:

1. Route state through bus subjects.
2. Reconstruct state inside the callback from its inputs.
3. Factor into a locus method that has its own `self`.

## See Also

- [Pattern catalog](./patterns.md) — the patterns these
  workarounds preserve.
- [Seeds and exports](./seeds.md) — the v1+ direction for
  modules and exports.
