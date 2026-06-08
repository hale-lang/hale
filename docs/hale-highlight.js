// Hale syntax highlighting for the mdbook docs.
//
// mdbook ships a fixed highlight.js build that doesn't know `hale`, so
// the ```hale code blocks rendered as plain text. This registers a
// `hale` language with the global `hljs` (loaded by mdbook before this
// additional-js) and re-highlights any already-rendered hale blocks.
//
// Keyword set tracks crates/hale-syntax/src/lexer.rs (hard keywords) +
// the contextual keywords the parser recognizes (topic, ring_layout,
// bindings, placement, mode, cooperative, pinned, pool, core, approx,
// within, inline, captures, with, violate, fail). The tree-sitter
// grammar in pond/heron is the editor-side counterpart.
(function () {
  if (typeof hljs === "undefined") return;

  function haleLanguage(hljs) {
    const KEYWORDS = {
      keyword:
        "locus perspective type const fn import export module topic ring_layout " +
        "params contract bus capacity as_parent_for indexed_by bindings placement mode " +
        "birth accept run drain dissolve on_failure " +
        "bulk harmonic resolution projection rich chunked recognition " +
        "cooperative pinned pool core " +
        "closure epoch persists_through resets_on resets_per_epoch approx within inline captures " +
        "restart restart_in_place quarantine reorganize bubble " +
        "expose consume inferred " +
        "subscribe publish on of stable_when serialize_as " +
        "let mut if else match for in while return break continue tier self " +
        "trait impl interface async await yield terminate release macro where with violate fail",
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
