// Hale syntax highlighting for the mdbook docs.
//
// mdbook ships a fixed highlight.js build that doesn't know `hale`, so
// the ```hale code blocks rendered as plain text. This registers a
// `hale` language with the global `hljs` (loaded by mdbook before this
// additional-js) and re-highlights any already-rendered hale blocks.
//
// The `keyword` list below is GENERATED from the compiler's canonical
// keyword set (crates/hale-syntax/src/keywords.rs) — do not edit it by
// hand. `cargo test -p hale-syntax --test keyword_sync` fails if it
// drifts; run that with UPDATE_KEYWORDS=1 to regenerate.
(function () {
  if (typeof hljs === "undefined") return;

  function haleLanguage(hljs) {
    const KEYWORDS = {
      // BEGIN GENERATED KEYWORDS — regen: `cargo test -p hale-syntax --test keyword_sync` (UPDATE_KEYWORDS=1 to bless). Source: crates/hale-syntax/src/keywords.rs.
      keyword:
        "accept adopt api approx as as_parent_for async avoiding await bindings birth birth_check block bound break bubble bulk bus cap capacity captures ceil chunked claims clamp closure connect const constitution consume continue contract cooperative core cores count cover cross_machine discard dissolve distinct domain drain drop duration during edges else epoch explicit export expose extends fail fallible fixed_cell floor fn for forbid gated group half_even half_up harmonic heap http if impl import in includes indexed_by inferred inline interface intra_machine intra_process l3 let listen locus macro main match may_be_empty mode module mut node of on on_failure on_full on_overflow on_unauthorized on_watch_full or origin params payload persists_through perspective pinned placement point pool principals prod projection publish quantity quarantine raise range reaches recognition refuse release reorganize reperspective replicas require requires reserve resets_on resets_per_epoch resolution restart restart_in_place return rich ring_layout role roles round rpc run schedule seed self serialize_as serve serves shared_slab shm_ring slot_count spillover stable_when subject subscribe sum summary_only terminate tick tier topic topology trait trunc type unit unix until via violate wait watch_bound where while with within wrap yield zero_copy",
      // END GENERATED KEYWORDS
      literal: "true false nil",
      built_in:
        "Int Uint Float Decimal Bool String Time Duration Bytes BytesView BytesMut StringView Unit",
    };

    // A unit name is coloured as a type (GH #1076). No highlighter knows
    // the catalogue, so the rule is positional: a quantity literal's
    // suffix, the names of a `unit` declaration, a denomination after
    // `in` or an `origin:`, and the target of `.in(…)` / `.split(…)`.
    const UNIT = { className: "type", begin: "[A-Za-z_]\\w*", relevance: 0 };
    const UNIT_LAST = hljs.inherit(UNIT, { endsParent: true });
    // A number: a prefixed radix, a Float or Decimal (`2.5`, `1e-5`,
    // `1.5d`), or a quantity, an integer with its unit written against
    // it (`500ms`, `3bp`, `1_250_000USD`), the unit a nested type.
    const NUMBER = {
      className: "number",
      begin:
        "\\b(?:0[xXoObB][0-9a-fA-F_]+\\b|\\d[\\d_]*(?:(?:\\.\\d[\\d_]*)?(?:[eE][+-]?\\d+)?d?\\b|(?=[A-Za-z])))",
      contains: [UNIT_LAST],
    };
    // What follows `in`, `origin:` or `.in(`: a unit, `100 ms` or
    // `100ms`. Its unit ends the enclosing rule.
    const DENOMINATION = [
      hljs.inherit(NUMBER, { begin: "\\d[\\d_]*(?=[A-Za-z])", endsParent: true }),
      { className: "number", begin: "\\d[\\d_]*" },
      UNIT_LAST,
    ];
    const DENOMINATION_AHEAD = "\\s*(?:\\d[\\d_]*\\s*)?[A-Za-z_]";

    return {
      name: "Hale",
      aliases: ["hl"],
      keywords: KEYWORDS,
      contains: [
        hljs.C_LINE_COMMENT_MODE,
        hljs.C_BLOCK_COMMENT_MODE,
        hljs.QUOTE_STRING_MODE,
        // `@form`, `@locality`, `@ffi` … annotations.
        { className: "meta", begin: "@\\w+" },
        // `unit USD = 100 cent;`, `unit bp = 1 / 10000;`, `unit tick;`:
        // a declaration starts its line, which a cursor's `unit bytes;`
        // clause inside a layout does not.
        {
          begin: "^[ \\t]*unit(?=\\s+[A-Za-z_]\\w*\\s*[=;])",
          end: ";",
          keywords: { keyword: "unit" },
          contains: [hljs.C_LINE_COMMENT_MODE, hljs.C_BLOCK_COMMENT_MODE, NUMBER, UNIT],
        },
        // A denomination: `quantity Int in cent`, `Duration in 100ms`.
        // Keyed on the type before `in`, so `for x in xs` is untouched.
        {
          begin: "\\b[A-Z][A-Za-z0-9_]*\\s+in\\b(?=" + DENOMINATION_AHEAD + ")",
          returnBegin: true,
          end: "[;{}()\\n]",
          returnEnd: true,
          contains: [
            { className: "type", begin: "[A-Z][A-Za-z0-9_]*", relevance: 0 },
            { begin: "\\bin\\b", keywords: { keyword: "in" } },
          ].concat(DENOMINATION),
        },
        // `origin: 273_150 mK`, `origin: 273_150mK`.
        {
          begin: "\\borigin\\s*:(?=\\s*-?\\s*\\d[\\d_]*\\s*[A-Za-z_])",
          end: "[;},\\n]",
          returnEnd: true,
          keywords: { keyword: "origin" },
          contains: DENOMINATION,
        },
        // A conversion's target: `d.in(s)`, `d.in(100ms)`, `d.split(s)`.
        {
          begin: "\\.(?:in|split)\\((?=" + DENOMINATION_AHEAD + "\\w*\\s*\\))",
          end: "\\)",
          returnEnd: true,
          keywords: { keyword: "in" },
          contains: DENOMINATION,
        },
        NUMBER,
        // Capitalized identifiers read as type / locus / topic names.
        { className: "type", begin: "\\b[A-Z][A-Za-z0-9_]*\\b", relevance: 0 },
      ],
    };
  }

  try {
    hljs.registerLanguage("hale", haleLanguage);
  } catch (e) {
    return;
  }

  // mdbook's book.js highlights all blocks on DOMContentLoaded. With
  // `hale` registered synchronously above, that pass highlights hale
  // blocks natively in most cases. This is the fallback for the case
  // where book.js's highlight ran *before* this script (so hale blocks
  // were left plain): re-highlight only blocks that aren't already
  // highlighted, so we never double-process one book.js handled
  // (double-highlighting mangles output under hljs 10.x).
  //
  // API-agnostic: highlightElement (11.x / 10.7+) or highlightBlock
  // (10.1, mdbook's current bundle) — so this survives an mdbook bump.
  var highlightOne = hljs.highlightElement
    ? function (el) { hljs.highlightElement(el); }
    : function (el) { hljs.highlightBlock(el); };

  function rehighlight() {
    document.querySelectorAll("code.language-hale").forEach(function (el) {
      // Already highlighted (by book.js, after our registration)?
      // hljs emits child spans with `hljs-*` classes — leave it alone.
      if (el.querySelector("[class^='hljs-'], [class*=' hljs-']")) return;
      el.classList.remove("hljs");
      if (el.dataset) delete el.dataset.highlighted;
      highlightOne(el);
    });
  }
  if (document.readyState === "loading") {
    // Registered after book.js's listener, so this runs after its pass.
    document.addEventListener("DOMContentLoaded", rehighlight);
  } else {
    rehighlight();
  }
})();
