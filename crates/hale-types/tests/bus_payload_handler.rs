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
            .any(|e| e.contains("optionally followed by `ctx: std::api::Context`") && e.contains("takes 2 parameters")),
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
            .any(|e| e.contains("optionally followed by `ctx: std::api::Context`") && e.contains("takes 0 parameters")),
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

// =====================================================================
// GH #876: the payload must be a type the bus can CARRY
// =====================================================================
//
// The rule above compares a handler's parameter against the declared
// payload. It never asked whether the declared payload is a payload
// at all — so `of type Int` typechecked clean and could not be
// lowered: `bus send payload must be a user-type or has-payload enum
// value; got Int` on the publish side, `missing or unsupported
// payload type (m60 requires a TypeRef, has-payload Enum, or
// BytesView)` on the subscribe side. Both from codegen, with no span,
// on a declaration an author writes early.
//
// These programs stay `format!` templates on purpose: the corpus
// harvester skips literals with unsubstituted placeholders, so a
// deliberately-unbuildable program here does not harvest itself into
// the check/build ratchet it exists to shrink.

/// The issue's shape, at both ends of it.
#[test]
fn a_primitive_bus_payload_is_refused_at_the_declaration() {
    let publisher = format!(
        r#"{PRELUDE}
        locus Pub {{
            bus {{ publish "org.metrics" of type Int; }}
            fn go(n: Int) {{ "org.metrics" <- n; }}
        }}
        fn main() {{ Pub {{ }}; }}
        "#
    );
    let errs = errors(&publisher);
    assert!(
        errs.iter().any(|e| e.contains("publish `org.metrics`")
            && e.contains("`Int` is not carried on the bus")
            && e.contains("an enum with a payload variant")),
        "a primitive publish payload is named at its declaration: {:?}",
        errs
    );

    let subscriber = format!(
        r#"{PRELUDE}
        locus Sub {{
            params {{ total: Int = 0; }}
            bus {{ subscribe "org.metrics" as on_m of type Int; }}
            fn on_m(n: Int) {{ self.total = self.total + n; }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    let errs = errors(&subscriber);
    assert!(
        errs.iter().any(|e| e.contains("subscribe `org.metrics`")
            && e.contains("`Int` is not carried on the bus")),
        "a primitive subscribe payload is named at its declaration: {:?}",
        errs
    );
}

/// The other uncarriable shapes reach the same rule: a structural
/// type expression has no name codegen can serialize, and a
/// no-payload enum has no storage struct — which codegen refuses by
/// name ("wrap in a struct or add a variant payload").
#[test]
fn structural_and_no_payload_enum_payloads_are_refused() {
    let structural = format!(
        r#"{PRELUDE}
        locus Sub {{
            bus {{ subscribe "pairs" as on_p of type (Int, Bool); }}
            fn on_p(p: (Int, Bool)) {{ println("x"); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    let errs = errors(&structural);
    assert!(
        errs.iter().any(|e| e.contains("is not carried on the bus")),
        "a tuple payload is refused: {:?}",
        errs
    );

    let empty_enum = format!(
        r#"{PRELUDE}
        type Flag = enum {{ On, Off }};
        locus Sub {{
            bus {{ subscribe "flags" as on_f of type Flag; }}
            fn on_f(f: Flag) {{ println("x"); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    let errs = errors(&empty_enum);
    assert!(
        errs.iter().any(|e| e.contains("`Flag` is not carried on the bus")
            && e.contains("give one variant a payload")),
        "a no-payload enum is refused with its own repair: {:?}",
        errs
    );
}

/// The accepted set, all three of it: a user `type`, an enum with a
/// payload variant, and `BytesView` — the raw-frame path a foreign
/// ring writes, which opts out of typed payloads entirely.
#[test]
fn the_carried_payload_kinds_stay_clean() {
    let user_type = format!(
        r#"{PRELUDE}
        locus Sub {{
            bus {{ subscribe "hello" as on_hello of type Greeting; }}
            fn on_hello(msg: Greeting) {{ println(msg.n); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    assert!(
        errors(&user_type).is_empty(),
        "a user type is carried: {:?}",
        errors(&user_type)
    );

    let payload_enum = format!(
        r#"{PRELUDE}
        type Cmd = enum {{ Stop, Go(Int) }};
        locus Sub {{
            bus {{ subscribe "cmds" as on_cmd of type Cmd; }}
            fn on_cmd(c: Cmd) {{ println("x"); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    assert!(
        errors(&payload_enum).is_empty(),
        "an enum with a payload variant is carried: {:?}",
        errors(&payload_enum)
    );

    let raw_frame = format!(
        r#"{PRELUDE}
        locus Sub {{
            bus {{ subscribe "recs" as on_rec of type BytesView; }}
            fn on_rec(b: BytesView) {{ println("x"); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    assert!(
        errors(&raw_frame).is_empty(),
        "BytesView is the raw-frame opt-out: {:?}",
        errors(&raw_frame)
    );
}

/// The permissive control. A payload named through a seed this
/// bundle does not hold resolves to `Unknown` by design — that is
/// what a single-file check of a multi-file seed sees — and "I
/// cannot see that name" must not be reported as "no such payload".
#[test]
fn an_unresolved_qualified_payload_stays_permissive() {
    let src = format!(
        r#"import "lib/shared" as shared;
        {PRELUDE}
        locus Sub {{
            bus {{ subscribe "org.metrics" as on_m of type shared::Metric; }}
            fn on_m(m: shared::Metric) {{ println("x"); }}
        }}
        fn main() {{ Sub {{ }}; }}
        "#
    );
    let errs = errors(&src);
    assert!(
        !errs.iter().any(|e| e.contains("is not carried on the bus")),
        "a cross-seed payload the bundle cannot resolve is left alone: {:?}",
        errs
    );
}
