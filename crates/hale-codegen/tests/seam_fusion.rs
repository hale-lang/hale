//! GH #1530 step 2: a seam's hops are inlined into it. The chain is
//! static (one tower, direct calls), and lowering marks each hop call
//! `alwaysinline` at its site, and nothing else: after optimization the
//! seam calls none of its hops, and a program with no seam carries no
//! such mark.
//!
//! The follow-up, fusion: a seam that splits the set a hop returns calls
//! the hop's fused clone, which returns the set in the seam's frame, a
//! member deep-copied as itself beside a constant tag. Once inlined, each
//! of the producer's return sites goes straight to its arm: the seam
//! reads no tag back and switches on none. A program with no seam has no
//! clone. The pins read the IR after the pipeline
//! (`BuildOptions::dump_ir_optimized`) and before it (`dump_ir`).

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The body of the function `name` in the module text `ir`.
fn body<'a>(ir: &'a str, name: &str) -> &'a str {
    let needle = format!("@{name}(");
    let mut at = 0;
    let start = loop {
        let line_start = at;
        let line_end = ir[at..].find('\n').map_or(ir.len(), |e| at + e);
        let line = &ir[line_start..line_end];
        if line.starts_with("define ") && line.contains(&needle) {
            break line_start;
        }
        assert!(line_end < ir.len(), "no definition of `{name}` in the IR");
        at = line_end + 1;
    };
    let end = ir[start..].find("\n}\n").map_or(ir.len(), |e| start + e);
    &ir[start..end]
}

/// The IR of `src`, before and after optimization.
fn ir_of(src: &str, tag: &str) -> (String, String) {
    let bin = harness::unique_bin(tag);
    let (pre, post) = (bin.with_extension("ll"), bin.with_extension("opt.ll"));
    let options = hale_codegen::BuildOptions {
        dump_ir: Some(pre.clone()),
        dump_ir_optimized: Some(post.clone()),
        ..build_opts::options()
    };
    build_opts::build_source(src, &bin, &options).expect("build");
    let out = (std::fs::read_to_string(&pre).expect("the IR"), std::fs::read_to_string(&post).expect("the optimized IR"));
    for p in [&bin, &pre, &post] {
        let _ = std::fs::remove_file(p);
    }
    out
}

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/hale").join(name);
    std::fs::read_to_string(&path).expect("the fixture")
}

#[test]
fn a_seams_hops_are_inlined_into_it() {
    let (_, ir) = ir_of(&fixture("seam_decoder_handler_test.hl"), "seam_fusion_pair");
    let b = body(&ir, "Counter.answer");
    for hop in ["@Wire.decode(", "@Desk.new_order(", "@Desk.cancel(", "@Wire.encode("] {
        assert!(!b.contains(hop), "`Counter.answer` still calls {hop} instead of inlining it:\n{b}");
    }
}

#[test]
fn a_program_with_no_seam_marks_nothing_inline() {
    let src = "type A { n: Int; }\nlocus L { fn f(a: A) -> Int { return a.n; } }\n\
               main locus App { params { l: L = L { }; } run() { println(to_string(self.l.f(A { n: 2 }))); } }\n\
               fn main() { App { }; }\n";
    let (pre, _) = ir_of(src, "seam_fusion_none");
    assert!(!pre.contains("alwaysinline"), "a program with no seam carries an `alwaysinline` mark");
}

/// GH #1530 step 3: an exposure's calls are a seam on the receiver, and a
/// gRPC exposure's call is still one decoder hop: the seam calls
/// `__rpc_decode_<n>` once, which reads the body as protobuf or as JSON
/// inside, never one decoder per wire.
#[test]
fn a_grpc_exposure_decodes_each_call_in_one_hop() {
    let (pre, _) = ir_of(&fixture("api/pb_codec_test.hl"), "seam_fusion_grpc");
    let seam = body(&pre, "Probe.__rpc_seam_1");
    // the decoder's set is split at once, so the hop is its fused clone
    let calls = seam.matches("@Probe.__rpc_decode_1(").count() + seam.matches("@Probe.__rpc_decode_1.fused(").count();
    assert_eq!(calls, 1, "one decoder hop:\n{seam}");
    assert!(!pre.contains("@Probe.__rpc_decode_pb"), "no second decoder");
}


/// A producer whose returns are members, split into a locus hop whose
/// eaters return no set: once fused, the seam's every dispatch is gone.
const SPLIT: &str = "type A { n: Int; }\ntype B { s: String; }\ntype In = A | B;\n\
    locus P { fn parse(x: Int) -> In { if x > 0 { return A { n: x }; } return B { s: \"neg\" }; } }\n\
    locus H { fn a(v: A) -> Int { return v.n * 2; } fn b(v: B) -> Int { return len(v.s); } }\n\
    locus S { params { p: P = P { }; h: H = H { }; } seam go(x: Int) -> Int { x -> self.p.parse -> self.h; } }\n\
    fn main() { let s = S { }; println(to_string(s.go(3) + s.go(-1))); }\n";

#[test]
fn a_fused_seam_reads_no_tag_and_switches_on_none() {
    let (pre, post) = ir_of(SPLIT, "seam_fusion_split");
    assert!(pre.contains("@P.parse.fused("), "the seam calls the producer's fused clone:\n{}", body(&pre, "S.go"));
    let b = body(&post, "S.go");
    assert!(!b.contains("switch "), "the fused seam still switches:\n{b}");
    assert!(!b.contains("load i32"), "the fused seam still reads a tag back:\n{b}");
    assert!(!b.contains("@P.parse"), "the clone is inlined:\n{b}");
}

/// A fallible producer (its failure placed by the seam) fuses the same
/// way: the decoder's dispatch in the pair fixture reads no tag.
#[test]
fn a_fallible_producer_fuses_too() {
    let (pre, post) = ir_of(&fixture("seam_decoder_handler_test.hl"), "seam_fusion_fallible");
    assert!(pre.contains("@Wire.decode.fused("), "the seam calls the decoder's fused clone");
    let b = body(&post, "Counter.answer");
    assert!(!b.contains("seam.fused.set"), "the set's storage in the seam's frame is folded away:\n{b}");
}

#[test]
fn a_program_with_no_seam_has_no_fused_clone() {
    let src = "type A { n: Int; }\ntype B { s: String; }\ntype In = A | B;\n\
               locus P { fn parse(x: Int) -> In { if x > 0 { return A { n: x }; } return B { s: \"neg\" }; } }\n\
               fn main() { let p = P { }; let n = match p.parse(3) { A(a) -> a.n, B(b) -> len(b.s), }; println(to_string(n)); }\n";
    let (pre, _) = ir_of(src, "seam_fusion_unfused");
    assert!(!pre.contains(".fused("), "a program with no seam declares a fused clone");
}
