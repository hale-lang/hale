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
        "accept approx as as_parent_for async await bindings birth birth_check block break bubble bulk bus cap capacity captures chunked closure connect const consume continue contract cooperative core cores cross_machine dissolve drain drop duration else epoch explicit export expose fail fallible fixed_cell fn for harmonic heap if impl import in indexed_by inferred inline interface intra_machine intra_process l3 let listen locus macro main match mode module mut node of on on_failure on_overflow or params payload persists_through perspective pinned placement pool prod projection publish quarantine recognition release reorganize reperspective replicas reserve resets_on resets_per_epoch resolution restart restart_in_place return rich ring_layout role run schedule self serialize_as serves shared_slab shm_ring slot_count spillover stable_when subject subscribe sum summary_only terminate tick tier topic topology trait type unix until violate where while with within yield zero_copy",
      // END GENERATED KEYWORDS
      literal: "true false nil",
      built_in:
        "Int Float Bool String Bytes BytesView StringView Unit",
    };

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
        // Prefixed-radix + duration literals before the generic number.
        { className: "number", begin: "\\b0[xXoObB][0-9a-fA-F_]+\\b" },
        { className: "number", begin: "\\b\\d[\\d_]*(\\.[\\d_]+)?(ns|us|ms|s|m|h)?\\b" },
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
