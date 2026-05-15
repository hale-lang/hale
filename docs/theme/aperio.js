// Custom highlight.js mode for Aperio code blocks.
//
// mdbook's `additional-js` always loads AFTER book.js, which runs
// hljs.highlightBlock at top-level — so by the time this file runs,
// the initial highlight pass has already happened and aperio blocks
// were auto-detected (producing nothing useful, since 'aperio' wasn't
// registered yet). After registering the language below, we redo
// those blocks specifically: reset their content to plain text, strip
// any hljs-* classes the first pass added, and call highlightElement
// again — now the language IS registered so it produces real output.
//
// Aliased to `ap` so ```ap fences also work.

hljs.registerLanguage('aperio', function (hljs) {
  const KEYWORDS = {
    keyword: [
      // Declaration
      'locus', 'type', 'perspective', 'interface', 'topic',
      'import', 'const', 'fn', 'module', 'main',
      // Locus members
      'params', 'contract', 'bus', 'capacity', 'mode', 'closure', 'bindings',
      // Lifecycle
      'birth', 'accept', 'run', 'drain', 'dissolve', 'on_failure',
      // Statement / control flow
      'let', 'mut', 'if', 'else', 'match', 'for', 'in', 'while',
      'return', 'break', 'continue', 'yield', 'as',
      // Contract
      'expose', 'consume', 'inferred',
      // Bus
      'subscribe', 'publish', 'of',
      // Mode
      'bulk', 'harmonic', 'resolution',
      // Projection
      'projection', 'rich', 'chunked', 'recognition',
      'fixed_cell', 'shared_slab', 'spillover', 'summary_only',
      // Schedule
      'schedule', 'cooperative', 'pinned',
      // Closure
      'epoch', 'persists_through', 'resets_on', 'approx', 'within',
      // Recovery
      'restart', 'restart_in_place', 'quarantine', 'reorganize', 'bubble',
      // Perspective
      'stable_when', 'serialize_as',
      // Fallible
      'fallible', 'fail', 'or', 'raise',
      // Capacity slot
      'pool', 'heap', 'indexed_by', 'as_parent_for',
      // Reserved
      'trait', 'impl', 'async', 'await', 'macro', 'where', 'with',
      'tier', 'self',
      // Transport (binding spec)
      'in_memory', 'unix', 'tcp', 'nats', 'listen', 'connect'
    ],
    literal: ['true', 'false', 'nil'],
    type: [
      'Int', 'Uint', 'Float', 'Decimal', 'String', 'Bool',
      'Time', 'Duration', 'Bytes',
      'Rich', 'Chunked', 'Recognition'
    ],
    built_in: [
      'print', 'println', 'eprint', 'eprintln',
      'len', 'to_string', 'min', 'max', 'abs',
      'sum', 'prod',
      'starts_with', 'contains'
    ]
  };

  return {
    name: 'Aperio',
    aliases: ['ap'],
    keywords: KEYWORDS,
    contains: [
      hljs.C_LINE_COMMENT_MODE,
      hljs.C_BLOCK_COMMENT_MODE,

      // f-string with {expr} interpolation
      {
        className: 'string',
        begin: 'f"', end: '"',
        contains: [
          hljs.BACKSLASH_ESCAPE,
          {
            className: 'subst',
            begin: /\{/, end: /\}/,
            keywords: KEYWORDS
          }
        ]
      },
      // Raw string r"..."
      { className: 'string', begin: 'r"', end: '"' },
      // Triple-quoted string (multi-line)
      { className: 'string', begin: '"""', end: '"""' },
      // Bytes literal b"..."
      {
        className: 'string',
        begin: 'b"', end: '"',
        contains: [hljs.BACKSLASH_ESCAPE]
      },
      // Regular string "..."
      hljs.QUOTE_STRING_MODE,
      // Time literal: `2026-05-08T12:00:00Z`
      { className: 'string', begin: '`', end: '`' },

      // Decimal literal: 1.50d, 0.05d
      { className: 'number', begin: /\b\d[\d_]*\.\d+d\b/ },
      // Duration literal: 5s, 100ms, 1h30m, etc.
      { className: 'number', begin: /\b\d+(?:ns|us|ms|s|m|h|d)\b/ },
      // Hex / oct / bin / decimal / float / typed-suffix numbers
      {
        className: 'number',
        begin: /\b(?:0x[0-9a-fA-F_]+|0o[0-7_]+|0b[01_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)(?:[iuf](?:8|16|32|64|128))?\b/
      },

      // Annotation: @form(vec), @projection, etc.
      { className: 'meta', begin: /@[a-zA-Z_][a-zA-Z0-9_]*/ },

      // Aperio-specific operators
      { className: 'operator', begin: /<-|~~/ }
    ]
  };
});

// Re-highlight any aperio code blocks that book.js processed before
// our language was registered. This is the load-order workaround
// described in the file header.
(function rehighlightAperioBlocks() {
  if (typeof document === 'undefined' || typeof hljs === 'undefined') return;
  var blocks = document.querySelectorAll('pre code.language-aperio, pre code.language-ap');
  blocks.forEach(function (block) {
    // Reset to plain text: collapse any hljs-* spans the first pass added.
    var src = block.textContent;
    block.textContent = src;
    // Strip the .hljs class + any hljs-* state classes from the first pass.
    block.classList.remove('hljs');
    Array.prototype.slice.call(block.classList).forEach(function (c) {
      if (c.indexOf('hljs-') === 0) block.classList.remove(c);
    });
    // Mark as not-yet-highlighted so highlightElement runs cleanly.
    delete block.dataset.highlighted;
    try {
      hljs.highlightElement(block);
    } catch (e) {
      // If aperio still isn't resolvable for some reason, leave plain.
    }
  });
})();
