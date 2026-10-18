//! GH #1530 step 2: a seam's hops are inlined into it. The chain is
//! static (one tower, direct calls), and lowering marks each hop call
//! `alwaysinline` at its site, and nothing else: after optimization the
//! seam calls none of its hops, and a program with no seam carries no
//! such mark. The dispatch on a set still reads the tag the producer
//! wrote (its return deep-copies the value into the caller's arena behind
//! runtime calls the optimizer cannot see through); folding it is the
//! follow-up. The pins read the IR after the pipeline
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
    assert_eq!(seam.matches("@Probe.__rpc_decode_1(").count(), 1, "one decoder hop:\n{seam}");
    assert!(!pre.contains("@Probe.__rpc_decode_pb"), "no second decoder");
}
