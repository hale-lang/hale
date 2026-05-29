//! F.36 Slice 2: `codec(L { ... })` binding clause + binding-
//! site assertion.
//!
//! Parser + AST recognize the clause; typecheck verifies the
//! codec locus exists, has encode/decode with the right
//! signatures (against the topic's payload type), and that both
//! methods are pure per F.36 Slice 1's inference. Slice 3 will
//! wire the codec dispatch into codegen; for now the typecheck
//! gate is the load-bearing piece.

fn typecheck_diags(source: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("main".to_string(), &program);
    let bundle = hale_types::Bundle { programs };
    let (scope, _) = hale_types::resolve::build_top_scope(&bundle);
    let diags = hale_types::check::check_bundle(&bundle, &scope, true);
    diags.into_iter().map(|d| d.message).collect()
}

#[test]
fn pure_codec_with_correct_signatures_typechecks() {
    let src = r#"
        type Tick { sym: String = ""; price: Int = 0; }
        type EncErr { kind: String = ""; }
        type DecErr { kind: String = ""; }

        topic TickTopic { payload: Tick; subject: "ticks"; }

        locus TickJsonCodec {
            fn encode(v: Tick) -> Bytes fallible(EncErr) {
                return std::bytes::from_string(v.sym);
            }
            fn decode(b: Bytes) -> Tick fallible(DecErr) {
                return Tick { sym: "x", price: 0 };
            }
        }

        main locus App {
            bus { publish TickTopic; }
            bindings {
                TickTopic: unix("/ticks.sock") codec(TickJsonCodec { });
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(diags.is_empty(), "expected no diagnostics, got: {:?}", diags);
}

#[test]
fn codec_with_self_mutation_in_encode_is_rejected() {
    let src = r#"
        type Tick { sym: String = ""; price: Int = 0; }
        type EncErr { kind: String = ""; }
        type DecErr { kind: String = ""; }

        topic TickTopic { payload: Tick; subject: "ticks"; }

        locus TickJsonCodec {
            params { call_count: Int = 0; }
            fn encode(v: Tick) -> Bytes fallible(EncErr) {
                self.call_count = self.call_count + 1;
                return std::bytes::from_string(v.sym);
            }
            fn decode(b: Bytes) -> Tick fallible(DecErr) {
                return Tick { sym: "x", price: 0 };
            }
        }

        main locus App {
            bindings {
                TickTopic: unix("/ticks.sock") codec(TickJsonCodec { });
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("TickJsonCodec.encode")
            && m.contains("self.call_count")
            && m.contains("stateless")),
        "expected self-mutation diag for encode, got: {:?}",
        diags
    );
}

#[test]
fn codec_with_println_in_decode_is_rejected() {
    let src = r#"
        type Tick { sym: String = ""; price: Int = 0; }
        type EncErr { kind: String = ""; }
        type DecErr { kind: String = ""; }

        topic TickTopic { payload: Tick; subject: "ticks"; }

        locus TickJsonCodec {
            fn encode(v: Tick) -> Bytes fallible(EncErr) {
                return std::bytes::from_string(v.sym);
            }
            fn decode(b: Bytes) -> Tick fallible(DecErr) {
                println("decoding...");
                return Tick { sym: "x", price: 0 };
            }
        }

        main locus App {
            bindings {
                TickTopic: unix("/ticks.sock") codec(TickJsonCodec { });
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("TickJsonCodec.decode")
            && m.contains("println")
            && m.contains("side effects")),
        "expected println diag for decode, got: {:?}",
        diags
    );
}

#[test]
fn codec_missing_encode_method_is_rejected() {
    let src = r#"
        type Tick { sym: String = ""; price: Int = 0; }
        type DecErr { kind: String = ""; }

        topic TickTopic { payload: Tick; subject: "ticks"; }

        locus TickJsonCodec {
            fn decode(b: Bytes) -> Tick fallible(DecErr) {
                return Tick { sym: "x", price: 0 };
            }
        }

        main locus App {
            bindings {
                TickTopic: unix("/ticks.sock") codec(TickJsonCodec { });
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("TickJsonCodec")
            && m.contains("missing required method")
            && m.contains("encode")),
        "expected missing-encode diag, got: {:?}",
        diags
    );
}

#[test]
fn codec_with_wrong_encode_return_type_is_rejected() {
    let src = r#"
        type Tick { sym: String = ""; price: Int = 0; }
        type EncErr { kind: String = ""; }
        type DecErr { kind: String = ""; }

        topic TickTopic { payload: Tick; subject: "ticks"; }

        locus TickJsonCodec {
            fn encode(v: Tick) -> String fallible(EncErr) {  // wrong: should be Bytes
                return v.sym;
            }
            fn decode(b: Bytes) -> Tick fallible(DecErr) {
                return Tick { sym: "x", price: 0 };
            }
        }

        main locus App {
            bindings {
                TickTopic: unix("/ticks.sock") codec(TickJsonCodec { });
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("TickJsonCodec")
            && m.contains("encode")
            && m.contains("must return")
            && m.contains("Bytes")),
        "expected encode-return-type diag, got: {:?}",
        diags
    );
}

#[test]
fn codec_locus_not_declared_is_rejected() {
    let src = r#"
        type Tick { sym: String = ""; price: Int = 0; }
        topic TickTopic { payload: Tick; subject: "ticks"; }

        main locus App {
            bindings {
                TickTopic: unix("/ticks.sock") codec(NoSuchCodec { });
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("unknown locus")
            && m.contains("NoSuchCodec")),
        "expected unknown-locus diag, got: {:?}",
        diags
    );
}

#[test]
fn binding_without_codec_clause_continues_to_work() {
    // Regression: pre-F.36 bindings (no codec clause) should
    // still typecheck cleanly.
    let src = r#"
        type Tick { sym: String = ""; price: Int = 0; }
        topic TickTopic { payload: Tick; subject: "ticks"; }

        main locus App {
            bus { publish TickTopic; }
            bindings {
                TickTopic: unix("/ticks.sock");
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(diags.is_empty(), "expected no diagnostics, got: {:?}", diags);
}
