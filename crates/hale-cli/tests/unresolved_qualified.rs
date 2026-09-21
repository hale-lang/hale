//! GH #803 (GH #911 B2) — a qualified path that resolves to nothing is
//! a located check error.
//!
//! `resolve_type_expr` maps an unknown multi-segment path to
//! `Ty::Unknown`, and the call, literal and variant positions carry
//! the matching tolerance, so a seed could write `zz::f()` while
//! importing nothing as `zz`, pass `hale check` and `hale verify`, and
//! then die in codegen:
//!
//! ```text
//! codegen error: unsupported in codegen v0: path call `zz::f` in
//! expression position
//! ```
//!
//! No location, a different layer, and after the gate had said yes.
//! Two mistakes hid there, and the rule answers both: a HEAD nothing
//! declares (`zz`), and a head an import DOES answer with a NAME the
//! library never declared (`b::Nope` — PR #819's `import_library_key`
//! had to run every such case through `build` precisely because
//! `check` could not see it).
//!
//! The controls are the other half: every head that is not an import
//! at all (an enum, an alias of one), every qualified path that DOES
//! resolve, and one file of a multi-file seed — whose `import` line
//! may live in a sibling — keep the tolerance they have always had.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The library seed every app below imports as `b`.
const LIB: &str = concat!(
    "type Greeting {\n",
    "    text: String = \"\";\n",
    "}\n",
    "\n",
    "type Mood = enum { Calm, Loud };\n",
    "\n",
    "type Tick {\n",
    "    n: Int = 0;\n",
    "}\n",
    "\n",
    "topic Ping {\n",
    "    payload: Tick;\n",
    "    subject: \"gh803.ping\";\n",
    "}\n",
    "\n",
    "const TAG: String = \"lib-tag\";\n",
    "\n",
    "fn hello() -> String {\n",
    "    return \"hi\";\n",
    "}\n",
    "\n",
    "locus Worker {\n",
    "    params {\n",
    "        n: Int = 0;\n",
    "    }\n",
    "\n",
    "    fn bump() {\n",
    "        self.n = self.n + 1;\n",
    "    }\n",
    "}\n",
);

/// The five positions a qualified path can stand in, with a head no
/// seed in the build declares. The trailing comments are line numbers:
/// the assertions name them, because a located error is the point.
const UNKNOWN_HEAD: &str = concat!(
    "import \"../lib\" as b;\n",                // 1
    "\n",                                       // 2
    "fn annotated(t: zz::Greeting) -> Int {\n",  // 3  annotation
    "    return 1;\n",                          // 4
    "}\n",                                      // 5
    "\n",                                       // 6
    "fn called() -> String {\n",                // 7
    "    return zz::hello();\n",                // 8  call
    "}\n",                                      // 9
    "\n",                                       // 10
    "fn literal() -> Int {\n",                  // 11
    "    let g = zz::Greeting { };\n",          // 12 struct literal
    "    return 1;\n",                          // 13
    "}\n",                                      // 14
    "\n",                                       // 15
    "fn variant() -> Int {\n",                  // 16
    "    let m = zz::Mood::Calm;\n",            // 17 enum variant
    "    return 2;\n",                          // 18
    "}\n",                                      // 19
    "\n",                                       // 20
    "main locus App {\n",                       // 21
    "    bindings {\n",                         // 22
    "        zz::Ping: unix(\"/tmp/gh803-head.sock\", role: listen);\n", // 23
    "    }\n",                                  // 24
    "\n",                                       // 25
    "    run() {\n",                            // 26
    "        println(\"ran\");\n",              // 27
    "    }\n",                                  // 28
    "}\n",                                      // 29
    "\n",                                       // 30
    "fn main() {\n",                            // 31
    "    App { };\n",                           // 32
    "}\n",                                      // 33
);

/// The same five positions with a head the seed DOES import and a name
/// the library does not declare.
const UNKNOWN_NAME: &str = concat!(
    "import \"../lib\" as b;\n",               // 1
    "\n",                                      // 2
    "fn annotated(t: b::Nope) -> Int {\n",     // 3  annotation
    "    return 1;\n",                         // 4
    "}\n",                                     // 5
    "\n",                                      // 6
    "fn called() -> String {\n",               // 7
    "    return b::nope();\n",                 // 8  call
    "}\n",                                     // 9
    "\n",                                      // 10
    "fn literal() -> Int {\n",                 // 11
    "    let g = b::Nope { };\n",              // 12 struct literal
    "    return 1;\n",                         // 13
    "}\n",                                     // 14
    "\n",                                      // 15
    "fn variant() -> Int {\n",                 // 16
    "    let m = b::Nope::Calm;\n",            // 17 enum variant
    "    return 2;\n",                         // 18
    "}\n",                                     // 19
    "\n",                                      // 20
    "main locus App {\n",                      // 21
    "    bindings {\n",                        // 22
    "        b::Nope: unix(\"/tmp/gh803-name.sock\", role: listen);\n", // 23
    "    }\n",                                 // 24
    "\n",                                      // 25
    "    run() {\n",                           // 26
    "        println(\"ran\");\n",             // 27
    "    }\n",                                 // 28
    "}\n",                                     // 29
    "\n",                                      // 30
    "fn main() {\n",                           // 31
    "    App { };\n",                          // 32
    "}\n",                                     // 33
);

/// One position: the smallest program the rule must refuse, and the
/// one whose single-FILE check must stay permissive (no `bindings`
/// block, whose topic rule is unconditional and would mask it).
const ONE_CALL: &str = concat!(
    "import \"../lib\" as b;\n",
    "\n",
    "fn called() -> String {\n",
    "    return zz::hello();\n",
    "}\n",
    "\n",
    "fn main() {\n",
    "    println(called());\n",
    "}\n",
);

/// Every position again, filled with a path that RESOLVES: the five,
/// plus a locus instantiation, a const, an enum variant in pattern
/// position and a bus subject. `__SOCK__` becomes a pid-unique path.
const RESOLVES: &str = concat!(
    "import \"../lib\" as b;\n",
    "\n",
    "fn annotated(t: b::Greeting) -> String {\n",
    "    return t.text;\n",
    "}\n",
    "\n",
    "fn named(m: b::Mood) -> String {\n",
    "    return match m {\n",
    "        b::Mood::Calm -> \"calm\",\n",
    "        b::Mood::Loud -> \"loud\",\n",
    "    };\n",
    "}\n",
    "\n",
    "main locus App {\n",
    "    params {\n",
    "        w: b::Worker = b::Worker { };\n",
    "    }\n",
    "\n",
    "    bus {\n",
    "        subscribe b::Ping as on_ping;\n",
    "    }\n",
    "\n",
    "    bindings {\n",
    "        b::Ping: unix(\"__SOCK__\", role: listen);\n",
    "    }\n",
    "\n",
    "    fn on_ping(t: b::Tick) {\n",
    "        println(\"ping \", t.n);\n",
    "    }\n",
    "\n",
    "    run() {\n",
    "        self.w.bump();\n",
    "        let g = b::Greeting { text: \"hey\" };\n",
    "        println(b::hello(), \" \", b::TAG, \" \", annotated(g));\n",
    "        println(named(b::Mood::Loud), \" \", self.w.n);\n",
    "    }\n",
    "}\n",
    "\n",
    "fn main() {\n",
    "    App { };\n",
    "}\n",
);

/// A head that names a DECLARATION is not an import at all: an enum
/// and a `type C2 = Color;` alias of one. No `import` line in this
/// seed, so nothing could resolve a path through the table.
const DECLARED_HEADS: &str = concat!(
    "type Color = enum { Red, Green };\n",
    "\n",
    "type C2 = Color;\n",
    "\n",
    "fn pick(c: Color) -> String {\n",
    "    return match c {\n",
    "        Color::Red -> \"red\",\n",
    "        Color::Green -> \"green\",\n",
    "    };\n",
    "}\n",
    "\n",
    "fn main() {\n",
    "    println(pick(Color::Red), \" \", pick(C2::Green));\n",
    "}\n",
);

/// Write a throwaway two-seed tree under a pid-unique temp dir and
/// return the app directory.
fn app_seed(tag: &str, app: &str) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!(
        "hale_unresolved_qualified_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&d);
    for (name, src) in [("lib/main.hl", LIB), ("app/main.hl", app)] {
        let p = d.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
        std::fs::write(&p, src).expect("write");
    }
    d.join("app")
}

/// One seed of one file, for the cases that import nothing.
fn solo_seed(tag: &str, src: &str) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!(
        "hale_unresolved_qualified_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    std::fs::write(d.join("main.hl"), src).expect("write");
    d
}

fn hale(cwd: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// Is `msg` reported on the line `file.hl:<n>:` — one diagnostic, both
/// halves, rather than two substrings that could come from two
/// different findings?
fn located_at(out: &str, at: &str, msg: &str) -> bool {
    out.lines().any(|l| l.contains(at) && l.contains(msg))
}

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // repo root
    p
}

/// The head case, in all five positions, through `check`, `run` and
/// `build` — the three commands that hold a whole program.
#[test]
fn a_head_no_seed_declares_is_refused_in_every_position() {
    let app = app_seed("head", UNKNOWN_HEAD);

    let (ok, out) = hale(&app, &["check", "."]);
    assert!(!ok, "check must refuse an unresolvable head:\n{out}");
    // One finding per position, each located at the path the author
    // wrote — not at a declaration, and not without a span.
    for (line, path) in [
        (3, "zz::Greeting"),
        (8, "zz::hello"),
        (12, "zz::Greeting"),
        (17, "zz::Mood::Calm"),
    ] {
        let msg = format!(
            "`{path}`: `zz` is not an import or a type of this seed"
        );
        assert!(
            located_at(&out, &format!("main.hl:{line}:"), &msg),
            "expected {msg:?} at main.hl:{line} in:\n{out}"
        );
    }
    assert_eq!(
        out.matches("is not an import or a type of this seed").count(),
        4,
        "one finding per path, and no more:\n{out}"
    );
    // The `bindings { }` topic needs no site of its own: an entry
    // naming a topic nothing declares has always been a located
    // error, qualified or not.
    assert!(
        located_at(
            &out,
            "main.hl:23:",
            "binding references unknown topic `zz::Ping`"
        ),
        "the bindings position is located too:\n{out}"
    );

    // `run` and `build` bundle exactly what they compile, so they
    // report the same finding instead of codegen's spanless one.
    for cmd in ["run", "build"] {
        let (ok, out) = hale(&app, &[cmd, "."]);
        assert!(!ok, "{cmd} must refuse it too:\n{out}");
        assert!(
            out.contains("`zz` is not an import or a type of this seed"),
            "{cmd} reports the same finding:\n{out}"
        );
        assert!(
            !out.contains("path call `zz::hello` in expression position"),
            "{cmd} must not fall through to codegen's spanless \
             refusal:\n{out}"
        );
    }

    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

/// The name case: the head IS this seed's import, and the library
/// declares no such name.
#[test]
fn a_name_the_imported_library_lacks_is_refused_in_every_position() {
    let app = app_seed("name", UNKNOWN_NAME);

    let (ok, out) = hale(&app, &["check", "."]);
    assert!(!ok, "check must refuse the unknown name:\n{out}");
    for (line, path) in [
        (3, "b::Nope"),
        (8, "b::nope"),
        (12, "b::Nope"),
        (17, "b::Nope"),
    ] {
        let msg = format!(
            "`{path}` is not declared by the library imported as `b`"
        );
        assert!(
            located_at(&out, &format!("main.hl:{line}:"), &msg),
            "expected {msg:?} at main.hl:{line} in:\n{out}"
        );
    }
    assert_eq!(
        out.matches("is not declared by the library imported as").count(),
        4,
        "one finding per path, and no more:\n{out}"
    );
    // What the library DOES provide, which is the useful half of the
    // message when no spelling is close — the shape codegen's twin
    // message has for the same table.
    assert!(
        out.contains(
            "`b` provides: Greeting, Mood, Ping, TAG, Tick, Worker, hello"
        ),
        "the message names the library's surface:\n{out}"
    );
    assert!(
        located_at(
            &out,
            "main.hl:23:",
            "binding references unknown topic `b::Nope`"
        ),
        "the bindings position is located too:\n{out}"
    );

    for cmd in ["run", "build"] {
        let (ok, out) = hale(&app, &[cmd, "."]);
        assert!(!ok, "{cmd} must refuse it too:\n{out}");
        assert!(
            out.contains("is not declared by the library imported as `b`"),
            "{cmd} reports the same finding:\n{out}"
        );
    }

    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

/// A did-you-mean when a spelling IS close: the substring rule first
/// (`Greet` for `Greeting`), which is the one a plain edit distance
/// misses.
#[test]
fn a_close_spelling_is_suggested() {
    let near = concat!(
        "import \"../lib\" as b;\n",
        "\n",
        "fn main() {\n",
        "    let g = b::Greet { };\n",
        "    println(\"made\");\n",
        "}\n",
    );
    let app = app_seed("near", near);
    let (ok, out) = hale(&app, &["check", "."]);
    assert!(!ok, "check must refuse it:\n{out}");
    assert!(
        out.contains("did you mean `b::Greeting`?"),
        "a close spelling is suggested:\n{out}"
    );
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

/// The permissive control. One FILE of a seed is not a whole program —
/// its `import` line may live in a sibling — so `hale check <file>`
/// keeps the tolerance, exactly as it does for a bare identifier
/// (GH #721) and a bare type name (GH #877). `build`, which bundles
/// what it compiles, refuses the same file.
#[test]
fn one_file_checked_alone_stays_permissive() {
    let app = app_seed("single", ONE_CALL);

    let (ok, out) = hale(&app, &["check", "main.hl"]);
    assert!(ok, "a single file keeps the permissive reading:\n{out}");
    assert!(
        !out.contains("is not an import or a type of this seed"),
        "and says nothing about the path:\n{out}"
    );

    let (ok, out) = hale(&app, &["build", "main.hl"]);
    assert!(!ok, "the build of that same file refuses it:\n{out}");
    assert!(
        out.contains("`zz` is not an import or a type of this seed"),
        "with the located finding:\n{out}"
    );

    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

/// `check --json` carries the located record, for the gates that read
/// it rather than the rendered text.
#[test]
fn json_carries_the_located_record() {
    let app = app_seed("json", ONE_CALL);
    let (ok, out) = hale(&app, &["check", ".", "--json"]);
    assert!(!ok, "`check --json` must reject:\n{out}");
    let line = out
        .lines()
        .find(|l| l.starts_with('{') && l.contains("is not an import"))
        .unwrap_or_else(|| panic!("no NDJSON record:\n{out}"));
    for needle in [
        "\"line\":4",
        "\"severity\":\"error\"",
        "\"kind\":\"type error\"",
        "`zz` is not an import or a type of this seed",
        "main.hl",
    ] {
        assert!(
            line.contains(needle),
            "expected {needle:?} in the record:\n{line}"
        );
    }
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

/// Every qualified path that resolves stays green — and still computes
/// the library's answers, so the rule did not change what a resolvable
/// path MEANS.
#[test]
fn every_resolvable_qualified_path_stays_green() {
    let sock = std::env::temp_dir()
        .join(format!("gh803-green-{}.sock", std::process::id()));
    let src = RESOLVES.replace("__SOCK__", &sock.display().to_string());
    let app = app_seed("green", &src);

    let (ok, out) = hale(&app, &["check", "."]);
    assert!(ok, "every path here resolves:\n{out}");
    let (ok, out) = hale(&app, &["run", "."]);
    assert!(ok, "run: {out}");
    for line in ["hi lib-tag hey", "loud 1"] {
        assert!(
            out.contains(line),
            "expected {line:?}; every path still means what it \
             meant:\n{out}"
        );
    }
    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

/// The two paths codegen answers WITHOUT the `std::` prefix are the
/// qualified twin of `BARE_BUILTIN_CALLEES`: a rule that refused them
/// would refuse programs `hale run` executes. They are a legacy
/// spelling, and they still compile.
#[test]
fn the_unprefixed_paths_codegen_answers_stay_green() {
    let d = solo_seed(
        "legacy",
        concat!(
            "fn main() {\n",
            "    let t = time::monotonic();\n",
            "    time::sleep(20ms);\n",
            "    println(\"elapsed>=0 \", time::monotonic() >= t);\n",
            "}\n",
        ),
    );
    let (ok, out) = hale(&d, &["check", "."]);
    assert!(ok, "check must not refuse what codegen lowers:\n{out}");
    let (ok, out) = hale(&d, &["run", "."]);
    assert!(ok, "run: {out}");
    assert!(out.contains("elapsed>=0 true"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Every OTHER unprefixed stdlib path is a dropped `std::`, and the
/// check-time message is the one codegen has always given for it —
/// now at the path's own span, before the build.
#[test]
fn a_dropped_std_prefix_says_so() {
    let d = solo_seed(
        "prefix",
        concat!(
            "fn main() {\n",
            "    println(\"args=\", env::args_count());\n",
            "}\n",
        ),
    );
    let (ok, out) = hale(&d, &["check", "."]);
    assert!(!ok, "check must refuse the unprefixed path:\n{out}");
    assert!(
        located_at(
            &out,
            "main.hl:2:",
            "`env::args_count` is unresolved — did you mean \
             `std::env::args_count`?"
        ),
        "the message names the prefix, at the path:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// A head that names a declaration is not an import: an enum, an alias
/// of one, and no `import` line in the seed at all.
#[test]
fn a_head_that_names_a_declaration_is_not_an_import() {
    let d = solo_seed("declared", DECLARED_HEADS);

    let (ok, out) = hale(&d, &["check", "."]);
    assert!(ok, "a declared head is not an import:\n{out}");
    let (ok, out) = hale(&d, &["run", "."]);
    assert!(ok, "run: {out}");
    assert!(out.contains("red green"), "{out}");

    let _ = std::fs::remove_dir_all(&d);
}

/// The false-positive oracle: every seed the repo itself carries. A
/// rule that refused a reference the language allows would show up
/// here first.
///
/// The assertion is the ABSENCE of this rule's findings rather than a
/// clean exit: a handful of fixture directories are known not to check
/// as a bundle (a directory of independent single-file programs, a
/// workspace-root-relative import that only resolves from the repo
/// root), and that is not this rule's business.
#[test]
fn the_repos_own_seeds_never_trip_the_rule() {
    let root = repo_root();
    let mut targets: Vec<PathBuf> = vec![
        root.join("dna/core"),
        root.join("dna/host"),
        root.join("dna/organism"),
        root.join("iris/consumer/fuse-hl"),
        root.join("tests/hale"),
    ];
    let examples = root.join("crates/hale-codegen/tests/fixtures/examples");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&examples)
        .expect("examples dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(dirs.len() > 50, "the example corpus went missing");
    targets.append(&mut dirs);

    for t in &targets {
        if !t.is_dir() {
            continue;
        }
        let (_, out) = hale(&root, &["check", &t.display().to_string()]);
        for needle in [
            "is not an import or a type of this seed",
            "is not declared by the library imported as",
        ] {
            assert!(
                !out.contains(needle),
                "GH #803's rule tripped on {}:\n{out}",
                t.display()
            );
        }
    }
}
