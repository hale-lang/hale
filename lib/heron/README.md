# heron — iris snapshot

Tree-sitter grammar for [Hale](https://github.com/hale-lang/hale).

> **This is a snapshot of `pond/heron`** vendored into iris
> per [`iris/PLAN.md`](../../PLAN.md) § 4 ("Snapshots of pond
> libs we use are one-time copies into iris/lib/, treated as
> forks from that moment"). The canonical upstream lives at
> `hale-lang/pond/heron`; refresh by re-copying when an
> upstream change matters. Only consumption-needed files are
> vendored — no grammar.js / node_modules / test harness /
> editor integrations; those live in pond for regeneration
> and downstream consumption.

The structural-parsing substrate Hale's dev tools build on:
one grammar, multiple consumers. The grammar lives here as the
canonical truth; generated parser tables + the small C scanner
ship in `src/`; query files in `queries/` drive editor
highlighting + IDE-shaped tools that don't need a full LSP.

## Consumers

- **`iris/lib/lotus_viz`** — uses heron to extract the locus
  tree from `.hl` source for the lotus flower visualization.
  The Hale-side `@ffi` wrapper exposes
  `LocusTreeProvider` (lotus_viz's interface) backed by a
  walk over heron's AST.
- **`iris/source_pane`** — uses heron for syntax highlighting
  (per `queries/highlights.scm`) and for block spotlighting
  (find the AST node enclosing a cursor position; tint its
  background).
- **Editor extensions** (VSCode / Helix / Neovim / Zed /
  Emacs) — consume the generated `parser.c` + the same
  `queries/highlights.scm`. Users get Hale syntax
  highlighting in whatever editor they're using as their
  external editor while iris is summoned (per
  [iris/VISION.md §5](../../hale-lang/iris/VISION.md)).
  See [`integrations/`](./integrations/) for per-editor
  setup — Helix shipped today; others welcome as PRs.
- **Future LSP** — when an Hale LSP server lands, heron is
  the parser it builds on. Hover, completion, goto-def,
  references, rename, formatting — all read from heron's
  AST.

This is why heron is in pond rather than buried inside any
one tool: it's the shared substrate.

## Status

v0 grammar mature: covers all of `spec/grammar.ebnf` as
exercised by iris + hale stdlib (19/19 stdlib files
parse cleanly, 23/23 corpus tests pass). Hale @ffi wrapper
+ glue.c verified end-to-end against libtree-sitter. Query
API live (Parser / Tree / Node / Query). Helix integration
shipped upstream; SourcePane consumes via the snapshot.

scanner.c for the trickiest contextual keywords (`mode`,
`captures`, `inline`, `fail`, `or`, `raise`, `with`,
`fallible`) is deferred until specific parse failures
motivate it — current grammar treats them as keywords with
context-based parsing, which works for all iris + stdlib
code today.

## Build

heron uses the standard tree-sitter build flow:

```
npm install                  # installs tree-sitter CLI
npx tree-sitter generate     # regenerates src/parser.c from grammar.js
npx tree-sitter test         # runs test/corpus/*.txt
npx tree-sitter parse FILE   # parses a .hl file, prints tree
```

The generated `src/parser.c` IS checked into the repo so
consumers don't need the tree-sitter CLI installed — they
just compile `parser.c` (and `scanner.c` when it exists)
against `libtree-sitter` (system install).

## Hale-side wrapper

Lives at the top of this directory:

- `heron.hl` — the `@ffi` bindings + a thin `Parser` / `Tree`
  / `Node` / `Query` Hale surface
- `glue.c` — the C shim hiding tree-sitter's TSNode-by-value
  behind pointer returns

The wrapper is **domain-agnostic**. Consumer-specific
adapters (e.g. lotus_viz's `TreeSitterHaleParser` that
extracts Petals from the AST) live in the consuming
package, not here — heron stays general.

Build picks up the link surface from `hale.toml`:

```toml
[ffi]
link = ["tree-sitter"]
csrc = ["glue.c", "src/parser.c"]
```

Consumer code reads:

```hale
import "vendor/pond/heron" as heron;

let parser = heron::Parser { };
let tree = parser.parse(content);
let root = tree.root();
println(heron::node_kind(root));    // "source_file"
println(heron::node_child_count(root)); // N

// For syntax highlighting via .scm queries:
let scm = std::io::fs::read_file(
    "vendor/pond/heron/queries/highlights.scm") or "";
let q = heron::Query { source: scm };
let runs = q.apply(root);  // "start:end:capture_name\n..."
```

## Suggested import alias

`heron`. The grammar surface (`Parser`, `Tree`, `Node`,
`Query`) lives directly here; consumer adapters live in the
consuming package.

## Grammar shape

Matches [`spec/grammar.ebnf`](../../hale/spec/grammar.ebnf)
section numbering as closely as tree-sitter's DSL admits:

1. Top level — `program`, `import_decl`, `top_decl`
2. Locus declaration — `locus_decl`, `locus_member`
3. Params — `params_block`, `param_decl`
4. Contract — `contract_block`, `contract_member`
5. Bus — `bus_block`, `bus_subscribe`, `bus_publish`
6. Capacity — `capacity_block`, `capacity_slot`
7. Lifecycle — `lifecycle_decl`
8. Modes — `mode_decl`
9. Failure handler — `failure_decl`
10. Closures — `closure_decl`
11. Perspectives — `perspective_decl`
12. Types — `type_decl`
13. Type expressions — `type_expr`
14. Function decls — `function_decl`, `fallible_marker`
15. Statements — `let_stmt`, `assign_stmt`, `send_stmt`,
    `if_stmt`, `match_stmt`, `for_stmt`, `while_stmt`,
    `return_stmt`, `break_stmt`, `continue_stmt`,
    `yield_stmt`, `recovery_stmt`, `violate_stmt`,
    `fail_stmt`, `expr_stmt`
16. Expressions — precedence-ordered per
    [`spec/precedence.md`](../../hale/spec/precedence.md)

## Deferred to scanner.c

These need context-sensitive lexing — tree-sitter's external
scanner pattern. Working without them at v0 means a few
parses fail on programs using these constructs at the
ambiguous position; the consumers (lotus_viz, source_pane)
fall back to "good enough" behavior on parse error.

- `mode` as a contextual keyword (only at locus-member
  position; ordinary ident elsewhere)
- `captures` (only inside `closure { ... }` body)
- `inline` (only as `epoch inline` clause)
- `fail` (only at statement-leading position inside a
  fallible fn body)
- `or` (only as postfix on a fallible-typed expression)
- `raise` (only as RHS of `or`)
- `discard` (only as RHS of `or`)
- `with` (only in `violate NAME with EXPR;`)
- `fallible` (only after a fn return type, before body)
- `approx` / `within` (only inside `closure { ... }` body)
- `pool` / `heap` (only at slot-decl head inside
  `capacity { ... }`)
- `topic` / `main` / `bindings` / `birth_check` (top-level
  contextual ident positions)

Most of these can be approximated by treating the keyword as
an ordinary ident and relying on grammar context to
disambiguate. Hard cases (e.g. `fn fail(x: Int)` vs
`fail X;` in a fallible body) ship to scanner.c when they
hit real code.

## Versioning

Pinned to a specific Hale compiler revision via the
`tree_sitter` field in `package.json`. When `spec/grammar.ebnf`
changes upstream, heron's grammar gets a corresponding bump
+ regenerate + republish.

## Why "heron"

Birds nest in trees; heron is a wading bird that watches
patiently from the edge of a pond. The grammar watches the
shape of Hale source from a small distance — close enough
to parse, far enough not to interpret semantics. That's
exactly what tree-sitter does.
