//! GH #2146: which `or` dispositions a bus send may carry, over every
//! topic shape that decides it.
//!
//! A send is refusal-fallible when its topic declares `on_full: fail` (a
//! subscriber at the topic's `bounded` capacity refuses it) or
//! `on_unmatched: fail` (a keyed send no filter matches is refused). Such
//! a send must carry a disposition, and only such a send may. Which ones:
//!
//! | topic                  | none | raise | discard | wait | handler(err) | fail err |
//! |------------------------|------|-------|---------|------|--------------|----------|
//! | neither                | yes  | no    | no      | no*  | no           | no       |
//! | `on_full: fail`        | no   | yes   | yes     | yes  | no           | no       |
//! | `on_unmatched: fail`   | no   | yes   | yes     | no   | yes          | yes      |
//! | both                   | no   | yes   | yes     | no   | no           | no       |
//!
//! `or wait` outwaits a full queue, never an unmatched key (*: on a topic
//! with neither, `or wait` needs a transport binding, which none of these
//! has). The err-payload dispositions are not wired for a full topic.
//!
//! Before GH #2146 the checker read the `on_unmatched` column alone, so
//! an unkeyed `on_full: fail` topic could not be published to at all: it
//! demanded a disposition and refused `or raise` and `or discard`, while
//! `hale build` (and `spec/semantics.md`) took them.

#[path = "support/entries.rs"]
mod entries;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Keys {
    /// no `keyed_by`
    Unkeyed,
    /// `keyed_by id;` and the default unmatched policy (swallow)
    Keyed,
    /// `keyed_by id; on_unmatched: fail;`
    KeyedFail,
}

const DISPOSITIONS: &[&str] = &["", " or raise", " or discard", " or wait", " or note(err)", " or fail err"];

fn program(full: bool, keys: Keys, disposition: &str) -> String {
    let mut topic = String::from("payload: Ev; subject: \"k\";");
    if keys != Keys::Unkeyed {
        topic.push_str(" keyed_by id;");
    }
    if keys == Keys::KeyedFail {
        topic.push_str(" on_unmatched: fail;");
    }
    if full {
        topic.push_str(" bounded(2); on_full: fail;");
    }
    // the send sits in a fallible fn, so `or fail err` has a channel of
    // the type it builds; on a keyed topic the subscriber takes key 1
    let filter = if keys == Keys::Unkeyed { "" } else { " where key == 1" };
    format!(
        r#"
type Ev {{ id: Int = 0; }}
topic K {{ {topic} }}
fn note(e: BusUnmatchedKey) {{ println(e.subject); }}
locus Sink {{
    bus {{ subscribe K as on_k{filter}; }}
    fn on_k(e: Ev) {{ println(e.id); }}
}}
locus Pusher {{
    bus {{ publish K; }}
    fn go() fallible(BusUnmatchedKey) {{
        K <- Ev {{ id: 1 }}{disposition};
    }}
    run() {{ self.go() or discard; }}
}}
main locus App {{ params {{ s: Sink = Sink {{ }}; p: Pusher = Pusher {{ }}; }} }}
fn main() {{ App {{ }}; }}
"#
    )
}

fn errors(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parses");
    entries::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

/// The rule the table above states.
fn admitted(full: bool, keys: Keys, disposition: &str) -> bool {
    let unmatched = keys == Keys::KeyedFail;
    match disposition {
        "" => !full && !unmatched,
        " or raise" | " or discard" => full || unmatched,
        " or wait" => full && !unmatched,
        _ => unmatched && !full,
    }
}

#[test]
fn the_matrix() {
    let mut wrong = Vec::new();
    for full in [false, true] {
        for keys in [Keys::Unkeyed, Keys::Keyed, Keys::KeyedFail] {
            for d in DISPOSITIONS {
                let src = program(full, keys, d);
                let errs = errors(&src);
                let want = admitted(full, keys, d);
                if want != errs.is_empty() {
                    wrong.push(format!(
                        "on_full: fail = {full}, {keys:?}, send `K <- ...{d}`: expected {}, got {errs:?}",
                        if want { "accepted" } else { "refused" }
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} cells disagree with the rule:\n{}", wrong.len(), wrong.join("\n"));
}

/// A refusal names what the topic would need, and a missing disposition
/// is asked for once even when both policies want one.
#[test]
fn the_refusals_say_why() {
    let errs = errors(&program(false, Keys::Unkeyed, " or raise"));
    assert!(
        errs.iter().any(|m| m.contains("only legal when the target topic declares `on_full: fail` or `on_unmatched: fail`")),
        "{errs:?}"
    );
    let errs = errors(&program(true, Keys::Unkeyed, ""));
    assert!(errs.iter().any(|m| m.contains("`on_full: fail` must carry an `or` disposition")), "{errs:?}");
    let errs = errors(&program(true, Keys::KeyedFail, ""));
    assert_eq!(errs.iter().filter(|m| m.contains("must carry an `or` disposition")).count(), 1, "{errs:?}");
    let errs = errors(&program(false, Keys::KeyedFail, " or wait"));
    assert!(errs.iter().any(|m| m.contains("`or wait` is not an unmatched-key disposition")), "{errs:?}");
    let errs = errors(&program(true, Keys::Unkeyed, " or note(err)"));
    assert!(errs.iter().any(|m| m.contains("on an `on_full: fail` topic")), "{errs:?}");
}
