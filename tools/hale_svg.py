#!/usr/bin/env python3
"""Render a Hale source snippet to a syntax-highlighted SVG.

GitHub renders READMEs with its own highlighter (Linguist), which has no
`hale` language and won't run our highlight.js — so a ```hale block shows
plain. This produces a colored SVG of a snippet to embed as an image, with
the same keyword set + theme as the docs site (docs/hale-highlight.js) and
the tree-sitter grammar in pond/heron.

    tools/hale_svg.py <input.hl> <output.svg>

Regenerate the README hero with
`python3 tools/hale_svg.py assets/readme/matchmaker.hl assets/readme/matchmaker.svg`.
The SVG is self-contained (its own dark background), so it reads well on
both GitHub light and dark themes.
"""
import html
import re
import sys

# Keyword set — keep in sync with docs/hale-highlight.js and the lexer
# (crates/hale-syntax/src/lexer.rs).
KEYWORDS = set(
    "locus perspective type const fn import export module topic ring_layout "
    "params contract bus capacity as_parent_for indexed_by bindings placement mode "
    "birth accept run drain dissolve on_failure "
    "bulk harmonic resolution projection rich chunked recognition "
    "cooperative pinned pool core "
    "closure epoch persists_through resets_on resets_per_epoch approx within inline captures "
    "restart restart_in_place quarantine reorganize bubble "
    "expose consume inferred "
    "subscribe publish on of stable_when serialize_as "
    "let mut if else match for in while return break continue tier self "
    "trait impl interface async await yield terminate release macro where with violate fail".split()
)
LITERALS = {"true", "false", "nil"}

# GitHub-dark theme (renders well over the SVG's own dark background on
# both GitHub light + dark).
THEME = {
    "bg": "#0d1117",
    "default": "#c9d1d9",
    "keyword": "#ff7b72",
    "literal": "#79c0ff",
    "type": "#ffa657",
    "string": "#a5d6ff",
    "number": "#79c0ff",
    "comment": "#8b949e",
    "meta": "#d2a8ff",
}

FONT = "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace"
FONT_SIZE = 13.5
CHAR_W = FONT_SIZE * 0.601  # monospace advance
LINE_H = 21.0
PAD_X = 16.0
PAD_Y = 14.0

TOKEN_RE = re.compile(
    r"""(?P<comment>//[^\n]*)
      | (?P<string>"(?:\\.|[^"\\])*")
      | (?P<meta>@[A-Za-z_][A-Za-z0-9_]*)
      | (?P<number>\b0[xXoObB][0-9a-fA-F_]+\b | \b\d[\d_]*(?:\.[\d_]+)?(?:ns|us|ms|s|m|h)?\b)
      | (?P<ident>[A-Za-z_][A-Za-z0-9_]*)
      | (?P<ws>\s+)
      | (?P<other>.)""",
    re.VERBOSE,
)


def classify(kind, text):
    if kind == "comment":
        return "comment"
    if kind == "string":
        return "string"
    if kind == "meta":
        return "meta"
    if kind == "number":
        return "number"
    if kind == "ident":
        if text in KEYWORDS:
            return "keyword"
        if text in LITERALS:
            return "literal"
        if text[:1].isupper():
            return "type"
    return "default"


def render(src):
    lines = src.rstrip("\n").split("\n")
    max_len = max((len(ln) for ln in lines), default=0)
    width = max_len * CHAR_W + 2 * PAD_X
    height = len(lines) * LINE_H + 2 * PAD_Y

    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" '
        f'height="{height:.0f}" viewBox="0 0 {width:.0f} {height:.0f}" '
        f'font-family="{FONT}" font-size="{FONT_SIZE}">',
        f'<rect width="{width:.0f}" height="{height:.0f}" rx="8" '
        f'fill="{THEME["bg"]}"/>',
    ]
    for i, line in enumerate(lines):
        y = PAD_Y + (i + 1) * LINE_H - 6
        spans = []
        for m in TOKEN_RE.finditer(line):
            kind = m.lastgroup
            text = m.group()
            if kind == "ws":
                spans.append(html.escape(text))
                continue
            color = THEME[classify(kind, text)]
            spans.append(f'<tspan fill="{color}">{html.escape(text)}</tspan>')
        out.append(
            f'<text xml:space="preserve" x="{PAD_X:.0f}" y="{y:.1f}">'
            + "".join(spans)
            + "</text>"
        )
    out.append("</svg>")
    return "\n".join(out) + "\n"


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: hale_svg.py <input.hl> <output.svg>")
    with open(sys.argv[1]) as f:
        src = f.read()
    with open(sys.argv[2], "w") as f:
        f.write(render(src))


if __name__ == "__main__":
    main()
