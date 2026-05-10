# `std::text`

Text-processing utilities. Phase 4 v0.1 (m91) ships one
function: `md_to_html`, a block-level Markdown → HTML
renderer written purely in Aperio.

The renderer is what `examples/docs-server/main.ap` uses to
render `.md` files into the HTML it serves.

## Functions

### `std::text::md_to_html`

#### Synopsis

```aperio
fn md_to_html(md: String) -> String
```

Converts a Markdown source String to an HTML String.

#### Supported (Phase 4 v0.1)

- **ATX headings.** `# h1` through `###### h6`. Each requires
  a space after the leading `#`s. A bare `######` with no
  trailing space falls through to paragraph handling, matching
  CommonMark's strict ATX rule.
- **Paragraphs.** Consecutive non-blank lines joined with a
  single space, wrapped in `<p>...</p>`. A blank line
  terminates the paragraph.
- **Fenced code blocks.** Triple backticks on their own line
  open and close a block. Body is emitted verbatim (with HTML
  escaping) inside `<pre><code>...</code></pre>`.
- **HTML escaping.** `&` is rewritten first (to avoid
  double-escaping), then `<` and `>`. All rendered content
  passes through escaping, so untrusted markdown cannot inject
  raw HTML or scripts.

#### Not supported (Phase 4 v1.0 follow-ups)

- Inline formatting: `**bold**`, `*italic*`, `` `inline code` ``,
  `[link text](url)`.
- Lists (ordered / unordered).
- Blockquotes.
- Setext-style headings (`====` / `----` underlines).
- Reference-style links.
- Raw HTML pass-through (currently always escaped).

#### Examples

```aperio
fn main() {
    let md = "# Hello\n\nThis is a paragraph.\n";
    let html = std::text::md_to_html(md);
    println(html);
    // <h1>Hello</h1>
    // <p>This is a paragraph.</p>
}
```

Render a fenced code block:

```aperio
fn main() {
    let md = "Here is code:\n\n```\nlet x = 42;\n```\n";
    println(std::text::md_to_html(md));
    // <p>Here is code:</p>
    // <pre><code>let x = 42;
    // </code></pre>
}
```

HTML in input is escaped, not passed through:

```aperio
fn main() {
    let md = "Watch out for <script>alert('x')</script>.";
    println(std::text::md_to_html(md));
    // <p>Watch out for &lt;script&gt;alert('x')&lt;/script&gt;.</p>
}
```

## Composing into a doc server

The Phase 5 capstone (`examples/docs-server/main.ap`) wires
`md_to_html` into an HTTP path:

```aperio
fn render_doc(s: std::io::tcp::Stream, path: String) {
    let md = std::io::fs::read_file(path);
    let body = "<!doctype html><html><body>"
             + std::text::md_to_html(md)
             + "</body></html>";
    let resp = std::http::Response {
        status: 200,
        content_type: "text/html; charset=utf-8",
        body: body
    };
    std::http::write_response(s, resp);
}
```

This is the canonical pattern for any markdown-serving Aperio
program.

## Limitations (Phase 4 v0.1)

- **Block-level only.** Inline formatting waits on a
  paragraph-buffer state-machine pass.
- **No syntax highlighting.** Code blocks render plain.
- **No HTML pass-through.** All HTML special characters in the
  source are escaped — there is no way to embed raw HTML in
  Markdown processed by this renderer.
- **No standalone `html_escape`.** The escaping logic is
  internal. A future milestone may surface
  `std::text::html_escape(s)` as a path-call.

## See Also

- [Roadmap](./roadmap.md) — Phase 4 v1.0 plan.
- `examples/docs-server/main.ap` (in the language repo) —
  composes `md_to_html` with `read_file` and `write_response`.
- `crates/aperio-codegen/runtime/stdlib/text.ap` (in the
  language repo) — the renderer implementation.
