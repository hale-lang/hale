# `docs/theme/`

Files in this directory override mdbook's default theme files.

## `highlight.js`

A custom highlight.js bundle with an Aperio language module appended.
The base is mdbook's own bundled highlight.js (10.1.1) plus an
`hljs.registerLanguage('aperio', ...)` block at the end.

When you need to update the base (mdbook upgrade, language tweak):

1. Build once with the stock theme:
   ```bash
   mv theme/highlight.js theme/highlight.js.bak
   mdbook build .
   cp ../docs/book/highlight-*.js theme/highlight.js
   ```
   That copies the version of highlight.js currently shipped with the
   mdbook binary into `theme/highlight.js` as a fresh base.
2. Append the Aperio module from `theme/highlight.js.bak` (the trailing
   `hljs.registerLanguage('aperio', ...)` block).
3. Delete the backup. Rebuild and confirm Aperio blocks highlight in
   the browser (`mdbook serve .` then open a page with an `aperio`
   fenced block).

The Aperio module's keyword set is conservative — it matches the v0
surface from the reference's lexical chapter and the Conventions
section. Extend it when new keywords land.
