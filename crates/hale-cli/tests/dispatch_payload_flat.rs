//! The direct-call gate's third leg is a column of the gate (F.40 phase
//! 3, P3 3 of 3; `notes/f40-capability-matrix.md` § 2.8).
//!
//! `payload_flat` is computed in the frontend from the subject's
//! resolved payload type, by codegen's `bus_payload_is_flat` rule moved
//! verbatim, and `DispatchFlavor::of` takes it as the third leg: a
//! direct-eligible subject with a managed payload is a static bucket in
//! the plan, as lowering has always emitted it (the plan used to say
//! `static_direct`). The codec keeps its own flatness over the lowered
//! payload, and lowering refuses a plan whose column disagrees with it.
//!
//! - every wire payload alternative, through both publish arms (a
//!   declared topic, a literal subject), checked against the codec at
//!   the publish (`HALE_DISPATCH_TRACE` prints both), with the flavor
//!   it gets and the run delivering;
//! - the corpus: at every publish to a literal subject, the column
//!   equals the codec's flatness;
//! - the compatibility change: `--dump-model`'s plan row, and a
//!   recording made by a compiler before the change (its execution
//!   digest is not this compile's) is refused by name, replays with
//!   `--allow-unverified-model`, and a recording made after it replays.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn workdir(name: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("hale_payload_flat_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

/// What `HALE_DISPATCH_TRACE` printed: each plan row's flavor and
/// column, and each publish's codec flatness.
#[derive(Default, Debug)]
struct Traced {
    rows: BTreeMap<String, (String, bool)>,
    publishes: Vec<(String, bool)>,
}

fn parse_trace(stderr: &str) -> Traced {
    let mut t = Traced::default();
    for l in stderr.lines() {
        let Some(rest) = l.strip_prefix("[hale-dispatch] ") else { continue };
        if let Some(p) = rest.strip_prefix("publish ") {
            if let Some((subj, flat)) = p.rsplit_once(" flat=") {
                t.publishes.push((subj.to_string(), flat == "true"));
            }
            continue;
        }
        let mut words = rest.split(' ');
        let (Some(subj), Some(flavor), Some(col)) = (words.next(), words.next(), words.next()) else { continue };
        if let Some(c) = col.strip_prefix("payload_flat=") {
            t.rows.insert(subj.to_string(), (flavor.to_string(), c == "true"));
        }
    }
    t
}

/// Build `prog` with the dispatch trace; the trace, and the binary when
/// the build succeeded.
fn build_traced(prog: &Path, out: &Path) -> (Traced, Option<PathBuf>, String) {
    let o = hale()
        .arg("build")
        .arg(prog)
        .arg("-o")
        .arg(out)
        .env("HALE_DISPATCH_TRACE", "1")
        .output()
        .expect("hale build");
    let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
    (parse_trace(&stderr), o.status.success().then(|| out.to_path_buf()), stderr)
}

/// The wire payload alternatives: (name, declarations, the payload
/// struct's fields — none when the declarations give `P` itself — the
/// published value, the codec's answer). An enum field is not a wire
/// payload (codegen refuses it), so the enum alternatives are the whole
/// payload.
const ALTERNATIVES: &[(&str, &str, &str, &str, bool)] = &[
    ("int", "", "a: Int = 0;", "P { }", true),
    ("float", "", "a: Float = 0.0;", "P { }", true),
    ("bool", "", "a: Bool = false;", "P { }", true),
    ("decimal", "", "a: Decimal = 1.5d;", "P { }", true),
    ("duration", "", "a: Duration = 1ms;", "P { }", true),
    ("alias_int", "type Px = Int;\n", "a: Px = 0;", "P { }", true),
    ("scalars", "", "a: Int = 0; b: Float = 0.0; c: Bool = false; d: Duration = 1ms; e: Decimal = 2.5d;", "P { }", true),
    ("string", "", "a: String = \"\";", "P { }", false),
    ("bytes", "", "a: Bytes = b\"\";", "P { }", false),
    ("time", "", "a: Time = std::time::from_nanos(0);", "P { }", false),
    ("nested_struct", "type Q { n: Int = 0; }\n", "a: Q = Q { };", "P { }", false),
    ("fixed_array", "", "a: [Int; 2] = [1, 2];", "P { }", false),
    ("int_and_string", "", "a: Int = 0; b: String = \"x\";", "P { }", false),
    ("payload_enum", "type P = enum { Tick(Int), Halt };\n", "", "P::Tick(1)", false),
    ("generic_int", "type Box<T> { v: T; }\ntype P = Box<Int>;\n", "", "let b: Box<Int> = Box { v: 1 }; b", true),
    ("generic_string", "type Box<T> { v: T; }\ntype P = Box<String>;\n", "", "let b: Box<String> = Box { v: \"x\" }; b", false),
];

/// One alternative through one publish arm: a quiet same-thread
/// subscriber (direct-eligible) and a main-locus publisher.
fn program(decls: &str, fields: &str, value: &str, topic_arm: bool) -> String {
    // A value may bind its payload first: `let b: T = …; b`.
    let (prelude, value) = match value.rsplit_once("; ") {
        Some((pre, expr)) => (format!("{pre}; "), expr),
        None => (String::new(), value),
    };
    let (decl, sub, publ, send) = if topic_arm {
        (
            "topic Evt { payload: P; subject: \"flat.alt\"; }\n",
            "subscribe Evt as on_e;",
            "publish Evt;",
            format!("Evt <- {value};"),
        )
    } else {
        (
            "",
            "subscribe \"flat.alt\" as on_e of type P;",
            "publish \"flat.alt\" of type P;",
            format!("\"flat.alt\" <- {value};"),
        )
    };
    // The publisher is the subscriber's sibling, not its owner: a send
    // to the publisher's own field is the intra-locus rewrite's, a
    // direct call before any publish is lowered.
    let payload = if fields.is_empty() { String::new() } else { format!("type P {{ {fields} }}\n") };
    format!(
        "{decls}{payload}{decl}\
         locus Sub {{\n    params {{ seen: Int = 0; }}\n    bus {{ {sub} }}\n    \
         fn on_e(p: P) {{ self.seen = self.seen + 1; }}\n    drain() {{ println(\"seen \", self.seen); }}\n}}\n\
         locus Pub {{\n    bus {{ {publ} }}\n    run() {{ {prelude}{send} }}\n}}\n\
         main locus App {{\n    params {{ s: Sub = Sub {{ }}; p: Pub = Pub {{ }}; }}\n}}\n\
         fn main() {{ App {{ }}; }}\n"
    )
}

#[test]
fn every_payload_alternative_and_both_publish_arms_agree_with_the_codec() {
    let dir = workdir("alternatives");
    let cases: Vec<(String, String, bool)> = ALTERNATIVES
        .iter()
        .flat_map(|(name, decls, fields, value, flat)| {
            [true, false].into_iter().map(move |topic_arm| {
                let arm = if topic_arm { "topic" } else { "literal" };
                (format!("{name}/{arm}"), program(decls, fields, value, topic_arm), *flat)
            })
        })
        .collect();
    let failures: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = cases
            .chunks(cases.len().div_ceil(6))
            .enumerate()
            .map(|(c, chunk)| {
                let dir = &dir;
                s.spawn(move || {
                    let mut failures = Vec::new();
                    for (i, (id, src, flat)) in chunk.iter().enumerate() {
                        let prog = dir.join(format!("alt_{c}_{i}.hl"));
                        std::fs::write(&prog, src).unwrap();
                        let (t, bin, stderr) = build_traced(&prog, &dir.join(format!("alt_{c}_{i}")));
                        let Some(bin) = bin else {
                            failures.push(format!("{id}: the build failed:\n{stderr}\n{src}"));
                            continue;
                        };
                        let Some((flavor, column)) = t.rows.get("flat.alt") else {
                            failures.push(format!("{id}: no plan row for flat.alt:\n{stderr}"));
                            continue;
                        };
                        let codec: Vec<bool> = t.publishes.iter().filter(|(s, _)| s == "flat.alt").map(|(_, f)| *f).collect();
                        if codec.is_empty() || codec.iter().any(|c| c != column) {
                            failures.push(format!("{id}: column {column}, codec {codec:?}"));
                        }
                        if *column != *flat {
                            failures.push(format!("{id}: column {column}, the codec's rule says {flat}"));
                        }
                        let want = if *flat { "static_direct" } else { "static_bucket" };
                        if flavor != want {
                            failures.push(format!("{id}: flavor {flavor}, want {want}"));
                        }
                        let run = Command::new(&bin).output().expect("run");
                        let out = String::from_utf8_lossy(&run.stdout);
                        if !run.status.success() || !out.contains("seen 1") {
                            failures.push(format!("{id}: the run did not deliver: {out}"));
                        }
                    }
                    failures
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The corpus: every program that publishes, built with the dispatch
/// trace; at every publish to a literal subject the plan row's column
/// equals the codec's flatness.
#[test]
fn the_column_equals_the_codec_at_every_publish_over_the_corpus() {
    let dir = workdir("corpus");
    let programs: Vec<(String, String)> = hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
        .into_iter()
        .filter(|p| p.source.contains("<-") && p.source.contains("fn main"))
        .map(|p| (p.origin, p.source))
        .collect();
    let results: Vec<(usize, usize, Vec<String>)> = std::thread::scope(|s| {
        let handles: Vec<_> = programs
            .chunks(programs.len().div_ceil(8).max(1))
            .enumerate()
            .map(|(c, chunk)| {
                let dir = &dir;
                s.spawn(move || {
                    let (mut built, mut compared, mut diverged) = (0, 0, Vec::new());
                    for (i, (origin, src)) in chunk.iter().enumerate() {
                        let sub = dir.join(format!("p{c}_{i}"));
                        std::fs::create_dir_all(&sub).unwrap();
                        let prog = sub.join("main.hl");
                        std::fs::write(&prog, src).unwrap();
                        let (t, bin, _) = build_traced(&prog, &sub.join("main"));
                        let _ = std::fs::remove_dir_all(&sub);
                        if bin.is_none() {
                            continue;
                        }
                        built += 1;
                        for (subj, codec) in &t.publishes {
                            let Some((_, column)) = t.rows.get(subj) else { continue };
                            compared += 1;
                            if column != codec {
                                diverged.push(format!("{origin}: {subj}: column {column}, codec {codec}"));
                            }
                        }
                    }
                    (built, compared, diverged)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let _ = std::fs::remove_dir_all(&dir);
    let built: usize = results.iter().map(|r| r.0).sum();
    let compared: usize = results.iter().map(|r| r.1).sum();
    let diverged: Vec<&String> = results.iter().flat_map(|r| r.2.iter()).collect();
    eprintln!("payload_flat shadow: {} programs, {built} built, {compared} publish sites compared", programs.len());
    assert!(compared >= 100, "the shadow compared too few publish sites: {compared}");
    assert!(diverged.is_empty(), "the column and the codec disagree:\n{}", diverged.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
}

/// A direct-eligible subject with a managed payload, beside one with a
/// flat payload.
const MIXED: &str = r#"
type Pod { n: Int = 0; }
type Note { text: String = ""; }
topic PodEvt { payload: Pod; subject: "flat.pod"; }
topic NoteEvt { payload: Note; subject: "flat.managed"; }

locus Sub {
    params { pods: Int = 0; notes: Int = 0; }
    bus {
        subscribe PodEvt as on_pod;
        subscribe NoteEvt as on_note;
    }
    fn on_pod(p: Pod) { self.pods = self.pods + 1; }
    fn on_note(n: Note) { self.notes = self.notes + 1; }
    drain() { println("pods ", self.pods, " notes ", self.notes); }
}

locus Pub {
    bus { publish PodEvt; publish NoteEvt; }
    run() {
        PodEvt <- Pod { n: 1 };
        NoteEvt <- Note { text: "hello" };
    }
}

main locus App {
    params { s: Sub = Sub { }; p: Pub = Pub { }; }
}

fn main() { App { }; }
"#;

/// The recording header's execution identity (4×u64 at offset 56).
fn recorded_exec_digest(rec: &Path) -> [u64; 4] {
    let b = std::fs::read(rec).expect("recording");
    let mut out = [0u64; 4];
    for (i, part) in out.iter_mut().enumerate() {
        let o = 56 + i * 8;
        *part = u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    }
    out
}

#[test]
fn the_plan_change_is_a_recorded_compatibility_change() {
    let dir = workdir("compat");
    let prog = dir.join("app.hl");
    std::fs::write(&prog, MIXED).unwrap();

    // `--dump-model`'s plan: the managed payload's subject is a static
    // bucket, the flat one direct.
    let dump = hale().arg("check").arg(&prog).arg("--dump-model").output().expect("hale check --dump-model");
    let text = String::from_utf8_lossy(&dump.stdout);
    let row = |subject: &str| {
        text.lines()
            .skip_while(|l| !l.starts_with("dispatch_plan ("))
            .find(|l| l.trim_start().starts_with(&format!("{subject} ")))
            .map(str::trim)
    };
    assert!(row("flat.pod").is_some_and(|r| r.starts_with("flat.pod static_direct")), "{text}");
    assert!(row("flat.managed").is_some_and(|r| r.starts_with("flat.managed static_bucket")), "{text}");

    // A recording made after the change replays.
    let rec = dir.join("after.halerec");
    let out = hale().arg("run").arg(&prog).env("LOTUS_OBS_RECORD", &rec).output().expect("hale run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("pods 1 notes 1"));
    let replay = |rec: &Path, extra: &[&str]| {
        hale().arg("replay").arg(rec).arg(&prog).arg("--allow-live-effects").args(extra).output().expect("hale replay")
    };
    let out = replay(&rec, &[]);
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("pods 1 notes 1"),
        "a recording made after the change does not replay:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // A recording made by a compiler before the change carries another
    // execution digest (its plan digest framed `static_direct` for
    // flat.managed, and its toolchain is another): refused by name,
    // never replayed silently; admitted with --allow-unverified-model,
    // and since the emitted code is the same, it replays.
    let before = dir.join("before.halerec");
    let mut bytes = std::fs::read(&rec).unwrap();
    bytes[56] ^= 0xff;
    std::fs::write(&before, &bytes).unwrap();
    assert_ne!(recorded_exec_digest(&before), recorded_exec_digest(&rec));
    let out = replay(&before, &[]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && err.contains("recorded from different build inputs"), "{err}");
    let out = replay(&before, &["--allow-unverified-model"]);
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("pods 1 notes 1"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
