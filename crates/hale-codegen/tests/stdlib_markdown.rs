//! m91 — Phase 4 v0.1 — std::text::md_to_html (block-level).
//!
//! v0.1 supports ATX headings, multi-line paragraphs, fenced
//! code blocks, and HTML escaping. Each test feeds a known
//! markdown source, runs the renderer, and checks the
//! resulting HTML for the expected fragments.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn render(source_md: &str) -> String {
    // Each test compiles a tiny Hale program that calls
    // std::text::md_to_html on the supplied markdown and
    // prints the result. Returning the rendered HTML keeps
    // assertions in Rust-flavored substring checks.
    let escaped = source_md
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let src = format!(
        r#"
        fn main() {{
            let md = "{}";
            let html = std::text::md_to_html(md);
            println(html);
        }}
        "#,
        escaped
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin("stdlib_markdown");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn h1_heading_renders_to_h1_tag() {
    let html = render("# Hello\n");
    assert!(
        html.contains("<h1>Hello</h1>"),
        "got: {:?}",
        html
    );
}

#[test]
fn six_levels_of_heading_render_correctly() {
    let html = render("# h1\n## h2\n### h3\n#### h4\n##### h5\n###### h6\n");
    for (level, text) in [(1, "h1"), (2, "h2"), (3, "h3"), (4, "h4"), (5, "h5"), (6, "h6")] {
        let tag = format!("<h{}>{}</h{}>", level, text, level);
        assert!(
            html.contains(&tag),
            "missing {}; got: {:?}",
            tag,
            html
        );
    }
}

#[test]
fn seven_hashes_is_not_a_heading() {
    // CommonMark: ATX headings are 1-6 hashes; 7+ falls
    // through to a paragraph.
    let html = render("####### too many\n");
    assert!(
        !html.contains("<h7>"),
        "rendered <h7>; got: {:?}",
        html
    );
    assert!(
        html.contains("<p>####### too many</p>"),
        "should be paragraph; got: {:?}",
        html
    );
}

#[test]
fn hash_without_space_is_not_a_heading() {
    // The required space after the hashes — a `#word` without
    // it stays a paragraph (avoids accidental headings from
    // hashtags etc.).
    let html = render("#hashtag\n");
    assert!(
        html.contains("<p>#hashtag</p>"),
        "got: {:?}",
        html
    );
    assert!(
        !html.contains("<h1>"),
        "shouldn't be a heading; got: {:?}",
        html
    );
}

#[test]
fn paragraphs_are_blank_line_separated() {
    let html = render("first paragraph\n\nsecond paragraph\n");
    assert!(
        html.contains("<p>first paragraph</p>"),
        "got: {:?}",
        html
    );
    assert!(
        html.contains("<p>second paragraph</p>"),
        "got: {:?}",
        html
    );
}

#[test]
fn multi_line_paragraph_joins_with_space() {
    let html = render("line one\nline two\nline three\n");
    assert!(
        html.contains("<p>line one line two line three</p>"),
        "got: {:?}",
        html
    );
}

#[test]
fn fenced_code_block_emits_pre_code_with_escaped_body() {
    let html = render("```\nlet x = 1;\n```\n");
    assert!(
        html.contains("<pre><code>"),
        "missing opener; got: {:?}",
        html
    );
    assert!(
        html.contains("</code></pre>"),
        "missing closer; got: {:?}",
        html
    );
    assert!(
        html.contains("let x = 1;"),
        "body missing; got: {:?}",
        html
    );
}

#[test]
fn code_block_body_html_escapes_unsafe_chars() {
    let html = render("```\n<script>alert(1)</script>\n```\n");
    assert!(
        html.contains("&lt;script&gt;"),
        "didn't escape <script>; got: {:?}",
        html
    );
    assert!(
        html.contains("&lt;/script&gt;"),
        "didn't escape </script>; got: {:?}",
        html
    );
    assert!(
        !html.contains("<script>"),
        "raw <script> leaked through; got: {:?}",
        html
    );
}

#[test]
fn paragraph_html_escapes_unsafe_chars() {
    let html = render("danger: <em>hi</em> & co\n");
    assert!(
        html.contains("&lt;em&gt;"),
        "got: {:?}",
        html
    );
    assert!(
        html.contains("&amp;"),
        "got: {:?}",
        html
    );
    assert!(
        !html.contains("<em>"),
        "raw <em> leaked; got: {:?}",
        html
    );
}

#[test]
fn ampersand_escaped_first_to_avoid_double_escape() {
    // If we escaped < first and & second, the `&` we
    // introduce inside `&lt;` would itself get escaped to
    // `&amp;lt;`. The order in __html_escape (& first, then
    // <, then >) prevents that.
    let html = render("a & b < c\n");
    assert!(
        html.contains("a &amp; b &lt; c"),
        "got: {:?}",
        html
    );
    assert!(
        !html.contains("&amp;lt;"),
        "double-escaped; got: {:?}",
        html
    );
}

#[test]
fn empty_input_renders_to_empty_html() {
    let html = render("");
    // Stripping the trailing newline that println adds:
    let stripped = html.trim_end();
    assert_eq!(stripped, "", "got: {:?}", html);
}

#[test]
fn heading_followed_by_paragraph_renders_both() {
    let html = render("# Introduction\n\nThis is the body of the doc.\n");
    assert!(
        html.contains("<h1>Introduction</h1>"),
        "got: {:?}",
        html
    );
    assert!(
        html.contains("<p>This is the body of the doc.</p>"),
        "got: {:?}",
        html
    );
}
