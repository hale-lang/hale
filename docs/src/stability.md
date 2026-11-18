# Stability & Compatibility

Hale is pre-1.0. What a release may change, and how you hear about it,
is written down in [`STABILITY.md`](https://github.com/hale-lang/hale/blob/main/STABILITY.md).
In short:

- Before 1.0, **any release may change the language or the stdlib**.
  There are no deprecation windows: a form that goes away is refused
  in the release that removes it, and the error points at what to
  write instead.
- Every break is listed under the release's **Breaking** heading in
  the CHANGELOG, with a one-line migration, and the change migrates
  the project's own Hale code (the stdlib, DNA, the tests and this
  book) in the same PR.
- At **1.0**, the stable tier becomes a promise: a program that builds
  against 1.0 using only stable surface keeps building, with the same
  meaning, on every 1.x.

## Tiers

Every stdlib namespace and every file of the specification is marked
**stable** or **experimental**. Before 1.0 the mark is information: it
says which surface is settled and which is new or still moving (each
experimental row says why). Both may change in any release. At 1.0 the
stable tier is what the 1.x promise covers.

A function whose name begins with `__` is a compiler hook, not
surface, whatever its namespace's tier. The table below is generated
from [`spec/stability.toml`](https://github.com/hale-lang/hale/blob/main/spec/stability.toml).
A test keeps it in step with the stdlib and the spec, so a new
namespace or spec file cannot land without a tier.

<!-- BEGIN GENERATED from spec/stability.toml by crates/hale-types/tests/stability_tiers.rs -->
### Standard library

| namespace | tier | notes |
|---|---|---|
| `std::api` | experimental | the api binding's surface rows are still converging (GH #1417, R-series) |
| `std::bus` | stable |  |
| `std::bytes` | stable |  |
| `std::bytes::builder` | stable |  |
| `std::compress` | stable |  |
| `std::crypto` | stable | experimental: `keccak256`, `rsa_pss_sha256_sign`, `rsa_pss_sha256_verify`, `secp256k1_public_key`, `secp256k1_sign`, `secp256k1_verify`, `secp256k1_recover`. the chain-signing primitives are new (#1935) |
| `std::decimal` | stable |  |
| `std::diag` | experimental | counters for tests and leak hunts, not a program's interface |
| `std::env` | stable |  |
| `std::hex` | stable |  |
| `std::http` | stable |  |
| `std::i256` | experimental | new (GH #1934) |
| `std::io::file` | stable |  |
| `std::io::fs` | stable |  |
| `std::io::h2` | experimental | the HTTP/2 transport is young and its hooks are all internal |
| `std::io::mirror` | experimental | the double-mapped receive ring's hooks; internal to the transports |
| `std::io::sockopt` | stable |  |
| `std::io::stdin` | stable |  |
| `std::io::stdout` | stable |  |
| `std::io::tcp` | stable |  |
| `std::io::tls` | stable |  |
| `std::io::udp` | stable |  |
| `std::io::unix` | stable |  |
| `std::json` | stable |  |
| `std::json::Value` | stable |  |
| `std::log` | stable |  |
| `std::math` | stable |  |
| `std::metrics` | experimental | observability is becoming pluggable; the metric API will move with it |
| `std::net` | experimental | new (GH #1908): `resolve` only, its return shape may grow a pinned-address dial |
| `std::observe` | experimental | the observe backends are new (GH #1592) and pluggable observability is in progress |
| `std::os` | stable |  |
| `std::process` | stable |  |
| `std::rand` | stable |  |
| `std::regex` | stable |  |
| `std::ring` | experimental | the SPSC ring's hooks; internal to the shared-memory bus |
| `std::secret` | experimental | the local vault serves DNA and the habitat, whose shape still moves |
| `std::shm` | experimental | timestamps of the zero-copy bus's last record; tied to the shm_ring layout |
| `std::str` | stable | experimental: `cut`, `cut_last`, `eq_ignore_case`, `last_index_of`, `lines`, `parse_bool`, `split`, `trim_end`, `trim_prefix`, `trim_start`, `trim_suffix`, `words`. the splits and trims and the `for` sources `split`, `lines` and `words` are new (#1995); promoted after one minor |
| `std::tar` | experimental | added for DNA's packaging; the API is the minimum DNA needed |
| `std::term` | stable |  |
| `std::test` | stable |  |
| `std::text` | stable |  |
| `std::text::base64` | stable |  |
| `std::time` | stable |  |
| `std::ts` | experimental | the tree-sitter shim parses Go only, for DNA's ingest |
| `std::u256` | experimental | new (GH #1934) |
| `std::url` | stable |  |

### Specification

| file | tier | notes |
|---|---|---|
| [`spec/api-description.schema.json`](https://github.com/hale-lang/hale/blob/main/spec/api-description.schema.json) | experimental | the api description's schema moves with spec/api.md |
| [`spec/api.md`](https://github.com/hale-lang/hale/blob/main/spec/api.md) | experimental | the api binding's surfaces, transports and clients are still converging (GH #1417) |
| [`spec/decisions.md`](https://github.com/hale-lang/hale/blob/main/spec/decisions.md) | stable | non-normative record. |
| [`spec/design-rationale.md`](https://github.com/hale-lang/hale/blob/main/spec/design-rationale.md) | stable | non-normative record. |
| [`spec/dna.md`](https://github.com/hale-lang/hale/blob/main/spec/dna.md) | experimental | DNA is a library built on the language, and its surfaces move with its delivery |
| [`spec/ffi.md`](https://github.com/hale-lang/hale/blob/main/spec/ffi.md) | stable | experimental: § WASM host interface; § C→Hale re-entry: `@export fn` on native (Crumb batch 2, 2026-07-27). |
| [`spec/forms.md`](https://github.com/hale-lang/hale/blob/main/spec/forms.md) | stable |  |
| [`spec/glossary.md`](https://github.com/hale-lang/hale/blob/main/spec/glossary.md) | stable | non-normative record. |
| [`spec/grammar.ebnf`](https://github.com/hale-lang/hale/blob/main/spec/grammar.ebnf) | stable |  |
| [`spec/habitat.md`](https://github.com/hale-lang/hale/blob/main/spec/habitat.md) | experimental | the habitat's verbs, rows and backends move with DNA |
| [`spec/keywords.txt`](https://github.com/hale-lang/hale/blob/main/spec/keywords.txt) | stable |  |
| [`spec/memory.md`](https://github.com/hale-lang/hale/blob/main/spec/memory.md) | stable | experimental: § Allocator implementation outline; § What the compiler emits; § Codegen ABI (v0). |
| [`spec/model.md`](https://github.com/hale-lang/hale/blob/main/spec/model.md) | experimental | the canonical model's tables grow with every judgment family; its Reserved families have no producer |
| [`spec/packages.md`](https://github.com/hale-lang/hale/blob/main/spec/packages.md) | experimental | the manifest, the lockfile and `hale fetch` are a first cut; registries and version resolution are not designed |
| [`spec/precedence.md`](https://github.com/hale-lang/hale/blob/main/spec/precedence.md) | stable |  |
| [`spec/projects.md`](https://github.com/hale-lang/hale/blob/main/spec/projects.md) | stable |  |
| [`spec/registry.md`](https://github.com/hale-lang/hale/blob/main/spec/registry.md) | experimental | a generated index of the compiler's own families; it describes the implementation, not the language |
| [`spec/runtime.md`](https://github.com/hale-lang/hale/blob/main/spec/runtime.md) | stable | experimental: § Observe backends; § Diagnostic + tuning env vars; § Runtime size budget. |
| [`spec/semantics.md`](https://github.com/hale-lang/hale/blob/main/spec/semantics.md) | stable | experimental: § Probes; § Observe backends (GH #1592); § Perspective hot-load. |
| [`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md) | stable |  |
| [`spec/styleguide.md`](https://github.com/hale-lang/hale/blob/main/spec/styleguide.md) | stable |  |
| [`spec/testing.md`](https://github.com/hale-lang/hale/blob/main/spec/testing.md) | stable | experimental: § Property-based testing. |
| [`spec/tokens.md`](https://github.com/hale-lang/hale/blob/main/spec/tokens.md) | stable |  |
| [`spec/types.md`](https://github.com/hale-lang/hale/blob/main/spec/types.md) | stable | experimental: § U256 and I256 — 256-bit integers; § Scalar types (GH #1076); § Roles (GH #1109, #1417); § Working-set estimator (F.32-2). |
| [`spec/units.md`](https://github.com/hale-lang/hale/blob/main/spec/units.md) | experimental | the unit dialect is new (GH #1076) and its rows are still growing |
| [`spec/verification.md`](https://github.com/hale-lang/hale/blob/main/spec/verification.md) | stable | experimental: § Fleet composition (GH #408 Phase 1); § The requirements artifact (GH #1835). |

The graph registry's Reserved families (`runs_under`, `transitions`, `view`) are experimental until they have a producer.
<!-- END GENERATED -->

## Keywords

The keyword list is pinned: `spec/keywords.txt` holds every hard and
contextual keyword, and a test fails on any change to it, so a change
is deliberate. Before 1.0 a change needs no RFC: adding, removing or
reclassifying a keyword takes a decision entry in `spec/decisions.md`
and a CHANGELOG fragment. F.47 freed `trait`, `impl`, `async`,
`await`, `macro` and `export`: each is an ordinary name.
