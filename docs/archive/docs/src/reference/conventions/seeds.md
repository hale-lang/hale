# Seeds and exports

## Synopsis

A *seed* is Aperio's exportable unit — the equivalent of a
crate, package, or module in other languages. Seeds expose
types and loci across their boundary. Free `fn`s are
seed-internal implementation; they do not cross.

## What a seed exports

| Construct | Crosses seed boundary |
|---|---|
| `type` | Yes |
| `locus` | Yes |
| Free `fn` | **No** |

A free `fn` at the top of a seed-internal file is either:

1. A `return`-bearing helper called by a lifecycle method
   body (which cannot itself use `return` at v0).
2. An extension hook passed via fn-pointer params (e.g.,
   `on_connection: fn(Stream)`).
3. A genuinely-isolated helper that doesn't fit any
   namespace. Rare; usually a smell that a namespace lotus
   wants extracting.

In all three cases the function is *internal* to the seed.
Anything callers in another seed should call must live as a
method on a locus.

## v0 status

At v0, only the standard library (`std::*`) exists as a seed.
User code lives in `apps/<name>/main.ap`, single-file. Shared
loci between apps must currently live in the std seed
(bundled at codegen) or be duplicated per app.

User-defined seeds, manifest files (`seed.toml`-shaped),
multi-file modules, and a real package manager are post-v1
work. Until then, the seed boundary is implicit (the std
boundary), and the rules above apply by convention rather than
by tooling enforcement.

## Naming (v1+)

When user seeds ship, expected conventions:

- Seed names are lowercase, hyphen-separated. (Snake-case is
  reserved for identifiers; hyphen-case marks the package
  boundary, matching Cargo and Go-module conventions.)
- The `aperio-` prefix marks first-party seeds maintained by
  the Aperio language project. Example: `aperio-iter`.
- Community seeds use any prefix or no prefix. Domain-specific
  seeds may include the domain name: `acme-billing`.
- A seed's `std::` namespace is reserved for the standard
  library; user seeds expose via their own top-level namespace
  matching the seed name (or via path declared in the
  manifest).

How a seed *grows* once named is governed by
[Rolling the design](./rolling.md) — naming a seed is the
boundary act; rolling is what keeps the catalog inside that
boundary small enough to hold in head.

## See Also

- [Pattern catalog](./patterns.md) — what's eligible to live
  in a seed (types and loci, six shapes).
- [Naming](./naming.md) — surface conventions for the
  exported names.
- [Rolling the design](./rolling.md) — how the catalog grows
  without fragmenting.
