//! Downstream handoff (2026-08-11), SOUNDNESS — a subscriber
//! handler's parameter type was never compared against the
//! subject's payload. Any type was accepted; the published value
//! was then reinterpreted field-by-field at the handler, and a
//! String field read through an Int parameter surfaced its heap
//! pointer from safe code, with `check` and `verify` both green.
//! The string-subject `of type` path already rejected the same
//! mismatch cross-site — and its message steered users toward the
//! unchecked `topic` construct.

use hale_types::check_program;

fn errors(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

const PRELUDE: &str = r#"
    type Greeting { text: String = ""; n: Int = 0; }
    type Other { a: Int = 0; b: Int = 0; }
    topic Hello { payload: Greeting; subject: "hello"; }
"#;

#[test]
fn subscribe_handler_payload_type_must_match_topic() {
    let src = format!(
        r#"{PRELUDE}
        locus Sub {{
            bus {{ subscribe Hello as on_hello; }}
            fn on_hello(msg: Other) {{ println(msg.a); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    let errs = errors(&src);
    assert!(
        errs.iter().any(|e| e.contains("takes `Other`")
            && e.contains("payload `Greeting`")),
        "the reinterpretation is rejected at check time: {:?}",
        errs
    );
}

#[test]
fn subscribe_handler_annotated_with_topic_name_is_a_type_error() {
    // The natural mistake (`subscribe Hello as on_hello` reads like
    // the handler gets a Hello). Used to survive check and die at
    // codegen as `unknown type name` — mangled and ungreppable
    // across a seed boundary.
    let src = format!(
        r#"{PRELUDE}
        locus Sub {{
            bus {{ subscribe Hello as on_hello; }}
            fn on_hello(msg: Hello) {{ println("x"); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    let errs = errors(&src);
    assert!(
        errs.iter().any(|e| e.contains("`Hello` is the topic")
            && e.contains("declare the parameter as `Greeting`")),
        "the topic-as-type mistake is named at check time: {:?}",
        errs
    );
}

#[test]
fn subscribe_handler_arity_must_be_one() {
    let two = format!(
        r#"{PRELUDE}
        locus Sub {{
            bus {{ subscribe Hello as on_hello; }}
            fn on_hello(a: Greeting, b: Int) {{ println(b); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    let errs = errors(&two);
    assert!(
        errs.iter()
            .any(|e| e.contains("exactly one parameter") && e.contains("takes 2")),
        "two params rejected: {:?}",
        errs
    );

    let zero = format!(
        r#"{PRELUDE}
        locus Sub {{
            bus {{ subscribe Hello as on_hello; }}
            fn on_hello() {{ println("x"); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    let errs = errors(&zero);
    assert!(
        errs.iter()
            .any(|e| e.contains("exactly one parameter") && e.contains("takes 0")),
        "zero params rejected: {:?}",
        errs
    );
}

#[test]
fn string_subject_of_type_handler_mismatch_is_also_caught() {
    // Same comparison, other subject form — the `of type` conflict
    // check was cross-site only; the handler boundary is covered by
    // the same new check.
    let src = r#"
        type Greeting { text: String = ""; n: Int = 0; }
        type Other { a: Int = 0; b: Int = 0; }
        locus Sub {
            bus { subscribe "hello" as on_hello of type Greeting; }
            fn on_hello(msg: Other) { println(msg.a); }
        }
        fn main() { Sub { }; }
    "#;
    let errs = errors(src);
    assert!(
        errs.iter().any(|e| e.contains("takes `Other`")
            && e.contains("payload `Greeting`")),
        "of-type handler mismatch rejected: {:?}",
        errs
    );
}

#[test]
fn matching_handler_and_unknown_payloads_stay_clean() {
    // Control: the correct spelling is untouched...
    let ok = format!(
        r#"{PRELUDE}
        locus Sub {{
            params {{ seen: Int = 0; }}
            bus {{ subscribe Hello as on_hello; }}
            fn on_hello(msg: Greeting) {{ self.seen = self.seen + 1; }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    assert!(errors(&ok).is_empty(), "control errs: {:?}", errors(&ok));

    // ...and a `Drain<T>` batch handler (which resolves Unknown at
    // this layer by design) is not flagged.
    let drain = format!(
        r#"{PRELUDE}
        locus Sub {{
            bus {{ subscribe Hello as on_batch; }}
            fn on_batch(d: Drain<Greeting>) {{ println("x"); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    let errs = errors(&drain);
    assert!(
        !errs.iter().any(|e| e.contains("carries payload")),
        "Drain batch handlers stay permissive: {:?}",
        errs
    );
}
