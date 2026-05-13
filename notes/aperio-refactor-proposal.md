# Aperio refactor proposal — corpus extraction pass

> Captured 2026-05-10. One-shot artifact: once the recommended
> extractions execute, this becomes a historical record of why
> they happened. References `notes/aperio-seed.md` for the
> exportable-unit concept and
> `spec/styleguide.md` for the patterns the proposal applies.

## Executive summary

After the codebase-onboarder arc shipped (m96 / m97 / m100 /
m102 / m102.5 + onboard + tower-join + .aperio-overrides), the
`apps/` tree contains visible duplication and several free-fn
clusters that violate the styleguide's namespace-lotus
guidance. Specifically:

- **3 high-priority extractions** (clear duplication; clean
  vocabularies).
- **2 probable extractions** (smaller, but coherent enough to
  warrant promotion).
- **1 stdlib primitive addition** (`std::io::fs::extension`)
  that supersedes one of the extraction candidates entirely.
- **4 things correctly placed already** that the styleguide
  endorses; these stay as they are.
- **0 things misclassified.** No `type` declarations need
  promotion to `locus` and no `locus` declarations need
  demotion to `type`.

All recommended extractions land in the std seed
(`runtime/stdlib/*.ap`) at v0. Once user-defined seeds ship
(post v1; see `notes/aperio-seed.md`), shared utilities migrate
out of std into community seeds (`aperio-iter`, `aperio-tagged`,
etc.).

**Total LOC eliminated** (across all consumers, after refactor):
roughly 350-400 lines of duplicated helper code, against
roughly 150-200 LOC added to the std seed. Net: ~200 LOC saved
+ a much sharper pattern catalog.

## Recommended extractions — priority order

### 1. `std::iter::Lines` — newline-separated string iteration

**Status:** Definitely extract. Foundation for several other
extractions.

**Where it lives now:** the 6-line newline-iteration boilerplate
appears verbatim ~15 times across 7 apps:

| File | Line | Usage |
|------|------|-------|
| `apps/onboard/main.ap` | 96, 115, 137, 258, 282, 583, 614 | tagged accumulator + lookup helpers + drive |
| `apps/tower-join/main.ap` | 99, 126, 161, 226 | tagged accumulator + name proposal |
| `apps/operational-graph/main.ap` | 93 | tagged section collector |
| `apps/import-graph/main.ap` | 41, 61 | imports collector + array builder |
| `apps/domain-graph/main.ap` | 35, 55 | type spec recursion + name iteration |
| `apps/ssg/main.ap` | (multiple) | content iteration |

The pattern (taken from `apps/onboard/main.ap:96-110`):

```aperio
let mut from = 0;
while from < total {
    let rest = acc[from..total];
    let nl = std::str::index_of(rest, "\n");
    let mut line = "";
    if nl < 0 { line = rest; from = total; }
    else { line = rest[0..nl]; from = from + nl + 1; }
    if len(line) > 0 {
        // do something with line
    }
}
```

**Proposed surface** (`runtime/stdlib/iter.ap`, exposed as
`std::iter::Lines`):

The hard question is whether to ship the callback shape
(`each(s, fn)`) or the cursor shape (`next(s, from) → (line,
new_from)`). The callback shape requires fn-pointer state
sharing — currently a v0 friction point (callbacks can't capture
closure state; per `notes/aperio-friction.md`). The cursor
shape works with shipped surface today.

**Recommendation:** ship the cursor shape now. Migrate to
callback shape once closures-with-state lands.

```aperio
locus __StdIterLines {
    params { }

    // Returns -1 when there's nothing left to iterate.
    fn next_idx(s: String, from: Int) -> Int {
        let total = len(s);
        if from >= total { return -1; }
        let rest = s[from..total];
        let nl = std::str::index_of(rest, "\n");
        if nl < 0 { return total; }
        return from + nl + 1;
    }

    // Returns the line at `from`, stripped of its trailing newline.
    fn line_at(s: String, from: Int) -> String {
        let total = len(s);
        if from >= total { return ""; }
        let rest = s[from..total];
        let nl = std::str::index_of(rest, "\n");
        if nl < 0 { return rest; }
        return rest[0..nl];
    }

    // Convenience: skip blank/comment lines for config-style iteration.
    fn is_skippable(line: String) -> Bool {
        if len(line) == 0 { return true; }
        return line[0..1] == "#";
    }
}
```

Use site (replaces the 6-line boilerplate):

```aperio
let it = std::iter::Lines { };
let mut from = 0;
while from >= 0 {
    let line = it.line_at(s, from);
    from = it.next_idx(s, from);
    if it.is_skippable(line) { continue; }
    // do something with line
}
```

The use site is still ~5 lines but the parsing logic is gone;
the loop is just an iteration scaffold. This sets up the next
two extractions.

**Why now:** the cursor-shape API is honest about what's
available today (no closures-with-state). When closures land
(language milestone), `Lines.each(s, fn)` is a
backward-compatible addition.

### 2. `std::tagged::Accumulator` — tagged-string helpers

**Status:** Definitely extract. Code-identical duplication.

**Where it lives now:**

| File | Line | Method |
|------|------|--------|
| `apps/onboard/main.ap` | 96-113 | `__count_tag` |
| `apps/onboard/main.ap` | 115-135 | `__first_tag_body` |
| `apps/onboard/main.ap` | 137-161 | `__collect_tag_csv` |
| `apps/tower-join/main.ap` | 99-122 | `__count_tag` |
| `apps/tower-join/main.ap` | 126-158 | `__collect_tag_array` |
| `apps/tower-join/main.ap` | 161-185 | `__first_tag_body` |

`__count_tag` and `__first_tag_body` are byte-identical between
the two apps. `__collect_tag_csv` (onboard) and
`__collect_tag_array` (tower-join) differ only in how they emit
the result: one comma-joins bare strings, the other comma-joins
JSON-quoted strings.

**Proposed surface** (`runtime/stdlib/tagged.ap`, exposed as
`std::tagged::Accumulator`):

```aperio
locus __StdTaggedAccumulator {
    params { }

    fn count(acc: String, tag: String) -> Int { ... }
    fn first_body(acc: String, tag: String) -> String { ... }
    fn collect_csv(acc: String, tag: String) -> String { ... }
    fn collect_array(acc: String, tag: String) -> String { ... }
    fn each_body(acc: String, tag: String) -> String { ... }
        // Returns matching bodies "\n"-separated for the caller
        // to iterate via std::iter::Lines.
}
```

Internally builds on `std::iter::Lines` for the line-by-line
scan. Net: removes ~150 LOC of identical code from
`apps/onboard/main.ap` and `apps/tower-join/main.ap`.

### 3. `std::name::Convention` — file-stem ↔ CamelCase

**Status:** Definitely extract. Code-identical duplication.

**Where it lives now:**

| File | Line | Method |
|------|------|--------|
| `apps/onboard/main.ap` | 163-192 | `__propose_locus_name` |
| `apps/tower-join/main.ap` | 226-260 | `__propose_locus_name` |

Both implementations are byte-identical. They convert
`request_cache.go` → `RequestCacheL` by:

1. Stripping the `.go` suffix.
2. Splitting on underscores.
3. Capitalizing the first letter of each segment.
4. Concatenating.
5. Appending `L`.

**Proposed surface** (`runtime/stdlib/name.ap`, exposed as
`std::name::Convention`):

```aperio
locus __StdNameConvention {
    params {
        // Whether file-stem inputs include an extension that
        // should be stripped (".go", ".rs", ".py"). Default:
        // matches whatever flavor the lang locus uses.
        strip_extension: String = ".go";
    }

    fn snake_to_camel(s: String) -> String { ... }
    fn camel_to_snake(s: String) -> String { ... }
    fn propose_locus_name(file: String) -> String { ... }
        // Internally: strip extension, snake_to_camel, append "L".
    fn strip_extension(file: String, ext: String) -> String { ... }
}
```

Use site (in onboard / tower-join):

```aperio
let nc = std::name::Convention { strip_extension: ".go" };
let proposed = nc.propose_locus_name(name);
```

**Why a separate seed entry rather than rolling into
`std::lang`:** the naming convention is language-agnostic
(snake_case ↔ CamelCase is an English orthography concern,
not a per-tree-sitter-flavor concern). Lang knows
node-kind strings; Convention knows orthography. Keep them
separate.

## Probable extractions

### 4. `std::json::Builder` — small JSON-shape helpers

**Status:** Probably extract. Smaller, but coherent.

**Where it lives now:**

| File | Line | Pattern |
|------|------|---------|
| `apps/domain-graph/main.ap` | 82-86 | `__append_entry` |
| `apps/operational-graph/main.ap` | 38-42 | `__append_entry` |
| `apps/tower-join/main.ap` | 335-340 | `__append_entry` |
| `apps/import-graph/main.ap` | 61-91 | `__build_imports_array` |

Each app independently implements `arr.empty? ? entry : arr +
", " + entry`. Plus `import-graph` has a `__build_imports_array`
that combines line-iteration with the append pattern.

**Proposed surface** (`runtime/stdlib/json.ap`, exposed as
`std::json::Builder`):

```aperio
locus __StdJsonBuilder {
    params { }

    fn append_entry(arr: String, entry: String) -> String {
        if len(arr) == 0 { return entry; }
        return arr + ", " + entry;
    }

    // Build a JSON array from a "\n"-separated list of items.
    // Each item is treated as a raw entry (caller quotes if needed).
    fn build_array(items: String) -> String { ... }

    // Quote a string value with " escaping. Currently a no-op
    // for most app values (Go names, paths) but exists as a
    // hook for proper escaping.
    fn quote(s: String) -> String { ... }

    // Wrap key:value pairs in {...}.
    fn wrap_object(fields: String) -> String { ... }
}
```

**Why "probable" not "definitely":** the per-app implementations
are 2-5 lines each. Extracting saves ~20 LOC total but adds a
seed entry. The trade is closer to break-even than the
high-priority three. Worth doing once the others land cleanly,
to demonstrate the pattern even at small scale.

### 5. `std::io::fs::extension` — stdlib primitive (NOT a locus extraction)

**Status:** Probable. Supersedes the duplicated
`__ends_with_source` / `__ends_with_go` helpers entirely.

**Where it lives now:** every app extractor has its own
`__ends_with_source` (or `__ends_with_go`) helper:

| File | Line |
|------|------|
| `apps/onboard/main.ap` | 194-202 |
| `apps/tower-join/main.ap` | 262-269 |
| `apps/operational-graph/main.ap` | 80-87 |
| `apps/import-graph/main.ap` | 125-131 |
| `apps/domain-graph/main.ap` | 73-79 |
| `apps/ssg/main.ap` | (similar) |

All ~7-line implementations check `name[n-3..n] == ".go"` (or
`.md`). Pure repetition.

**Proposed surface** (path-call primitive in codegen, like
`std::io::fs::file_exists`):

```aperio
// Returns the extension string after the last `.`, or "" if
// none. Examples: extension("main.go") == ".go";
// extension("Makefile") == "".
std::io::fs::extension(path: String) -> String;
```

Implementation: ~10 LOC in `lotus_arena.c` (find last `.`,
return slice from there to end via global payload arena). Plus
a `lower_std_io_fs_extension` arm in codegen.

Use site (replaces every `__ends_with_source`):

```aperio
if std::io::fs::extension(name) == ".go" { ... }
```

Cleaner than the helper because there's nothing to instantiate —
just a path-call primitive.

**Why this is preferable to a `FileFilterL` namespace lotus:**
the operation is a single primitive (find-last-dot + slice).
Wrapping it in a locus is overengineering. The styleguide's
free-fn-when-genuinely-isolated rule applies — and the
`std::io::fs::*` namespace already exists for these.

## Re-evaluations / sub-locus opportunities (lower priority)

### `apps/onboard/main.ap`'s `__render_per_file` is a 200+ LOC fn

**Where:** lines 302-490.

The fn has two distinct enrichment passes (lines 348-386 for
handler-route decoration; lines 387-437 for spawn-target
lookup) that each do their own cross-reference walk against
the `wires` and `fn_defs` aggregates.

**Possible split:**

- `HandlerEnricherL` — empty params, methods that take a
  handler-name + wires aggregate and produce a decorated
  string.
- `SpawnFormatterL` — empty params, methods that take a
  spawn-tag-line + fn_defs aggregate and produce a decorated
  string.

**Why "lower priority":** the inline form is still readable.
A reader scrolling through `__render_per_file` can see the
enrichment passes laid out; extracting them makes the file
shorter but adds two locus instantiations. Worth doing if
`__render_per_file` grows further; not yet urgent.

## Correctly placed (don't change)

The styleguide endorses these placements; the refactor leaves
them alone.

- **`crates/aperio-codegen/runtime/stdlib/core.ap`** — free fns
  `__html_escape`, `__replace_all`. These are pure string
  algebra used internally by `__md_to_html`. They don't form
  a coherent vocabulary outside of "things text.ap needs."
  Promoting them would create a stranded namespace.
- **`crates/aperio-codegen/runtime/stdlib/lang.ap:33`
  `__StdLangLang`** — exemplar namespace lotus.
- **`crates/aperio-codegen/runtime/stdlib/lang.ap:403`
  `__StdLangMorpheme`** — exemplar namespace lotus with both
  flavor and overrides params.
- **`crates/aperio-codegen/runtime/stdlib/log.ap`'s
  `__StdLogLogger`** — exemplar service locus with bus
  participation. No change.
- **`crates/aperio-codegen/runtime/stdlib/io_tcp.ap:23`
  `__StdIoTcpStream`** and **`...:92` `__StdIoTcpListener`**
  — exemplar service locus + spawned child. No change.
- **`crates/aperio-codegen/runtime/stdlib/http.ap:32-50`**
  `__StdHttpRequest` + `__StdHttpResponse` — exemplar shape
  types. No change.

## Things NOT to extract (anti-patterns to avoid)

- **Per-app tree walkers.** Every extractor has its own
  `__walk` / `__unified_walk` / `__collect_*_specs`. These are
  hyper-specific to each app's tagged-accumulator schema.
  Generalizing them would require a callback-with-state
  facility we don't have at v0; even with it, the per-app
  schemas differ enough that the abstraction would carry too
  much config.
- **A general "util" namespace** lumping everything together.
  The styleguide explicitly calls this out as an anti-pattern.
  Group by *vocabulary*, not by "small string helpers."

## Suggested execution ordering

Each step is a focused commit. Tests stay green throughout —
the refactor is mechanical (no JSON or text output should
change). Order minimizes intermediate breakage:

1. **Add `std::io::fs::extension` primitive.** Codegen change
   + path-call dispatch + 1 stdlib doc. Doesn't depend on
   anything; the apps don't switch to it yet.
2. **Extract `std::iter::Lines`.** Adds `runtime/stdlib/iter.ap`,
   updates `STDLIB_AP_SOURCE` + `STDLIB_PATH_RENAMES`. No
   consumers yet.
3. **Extract `std::tagged::Accumulator`.** Adds
   `runtime/stdlib/tagged.ap`. Internally uses `Lines`. No
   consumers yet.
4. **Extract `std::name::Convention`.** Adds
   `runtime/stdlib/name.ap`. No consumers yet.
5. **Extract `std::json::Builder`.** Adds
   `runtime/stdlib/json.ap`. No consumers yet.
6. **Migrate `apps/tower-join/main.ap`** to use the new
   surfaces. One commit; tests must stay green.
7. **Migrate `apps/onboard/main.ap`.** One commit.
8. **Migrate `apps/import-graph/main.ap`,
   `apps/operational-graph/main.ap`,
   `apps/domain-graph/main.ap`.** One commit each (they're
   simpler and don't all need every new surface).
9. **(Optional) Extract `HandlerEnricherL` /
   `SpawnFormatterL`** sub-loci within `apps/onboard/main.ap`.
   Defer if the inline form is still acceptable.

Steps 2-5 add to the std seed without changing any consumers,
so each lands cleanly. Steps 6-8 each strictly reduce LOC; all
tests should keep passing because the JSON / text outputs are
unchanged.

## Risks & trade-offs

- **Stdlib growth.** Each new namespace lotus adds to
  `STDLIB_PATH_RENAMES` and `STDLIB_AP_SOURCE`. After all five
  extractions: +5 entries in the renames table; +~150 LOC in
  bundled `.ap` source. Acceptable as an interim home until v1
  user-defined seeds ship and shared utilities migrate to
  community seeds (per `notes/aperio-seed.md`).
- **Per-app instance allocs.** Each app does
  `let r = SomeL { };` once per fn that uses the namespace.
  Each is one alloc. Total cost across all apps: roughly 10-20
  extra allocs at startup. Negligible at v0; potential
  optimization later (empty-params loci could compile to bare
  static methods).
- **Stdlib coupling.** A bug in `std::tagged::Accumulator`
  affects every consumer. Mitigation: tests for each new
  namespace lotus must cover the surface comprehensively
  before consumers migrate.
- **Migration cost.** Step 6 (migrating tower-join) is the
  largest single migration since tower-join uses every new
  surface. Estimated effort: ~half-day to migrate + verify
  tests.
- **Test surface.** Each extraction needs its own test
  module under `crates/aperio-codegen/tests/`. Roughly 4-5
  new test files; ~30-50 new test cases total. Worth it for
  the regression coverage.

## Verification (after refactor)

The proposal is approved when:

1. All 290+ existing workspace tests stay green.
2. New tests for each extracted namespace lotus pass.
3. JSON output of `apps/tower-join` is byte-identical against
   both fixtures (operational-graph, import-graph) before and
   after migration.
4. Text output of `apps/onboard` is byte-identical against the
   same fixtures (modulo the `overrides: ... loaded` line if
   the fixture happens to have one).
5. No app's `.ap` file grows in LOC; ideally all migrating
   apps shrink.

## Cross-references

- `notes/aperio-types-vs-loci.md` — the constraint "seeds
  export only types and loci."
- `notes/aperio-seed.md` — the v1+ direction the std seed
  growth feeds.
- `spec/styleguide.md` — the patterns this proposal applies.
  Pattern 2 (Namespace lotus) is the shape every recommended
  extraction takes.
- `apps/onboard/main.ap`, `apps/tower-join/main.ap`,
  `apps/import-graph/main.ap`, `apps/operational-graph/main.ap`,
  `apps/domain-graph/main.ap` — the migration targets.
- `crates/aperio-codegen/src/codegen.rs` —
  `STDLIB_AP_SOURCE` + `STDLIB_PATH_RENAMES` are the v0 std
  seed wiring; new entries land here.
