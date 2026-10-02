//! The effective target's precedence (F.40 phase 3, P3 2 of 3;
//! `notes/f40-capability-matrix.md` §1.3, T1(b)), from the outside.
//!
//! One target for analysis and emission alike, on every entry point:
//! an explicit `--target` is the effective target; with none, a written
//! `target wasm { }` / `target browser_js { }` declaration selects
//! wasm32; with neither, the host. An explicit `--target` of another
//! class than a written declaration's is refused at the declaration.
//! `--wrap-main` needs wasm32, and the declaration it injects never
//! selects it.
//!
//! Each cell of the table runs `hale check` and `hale build` with the
//! same `--target`, and, where the CLI names no target, the editor's
//! snapshot (`Config::editor`) over the same file: the three agree on
//! admission and on the located refusals, compared as `line:col
//! message` sets. (`--wrap-main` is a build flag `hale check` does not
//! take, so its row is held to the build alone.) The same three are
//! held to one answer over every wasm-relevant program in the tree.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_frontend::frontend::LoadMode;

fn case_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hale_target_precedence_{}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    dir
}

fn hale(args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).output().expect("run hale");
    (
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
        out.status.code().unwrap_or(-1),
    )
}

/// The located errors a CLI run printed for `file`, as `line:col
/// message`.
fn cli_refusals(text: &str, file: &Path) -> BTreeSet<String> {
    let prefix = format!("{}:", file.display());
    text.lines()
        .filter_map(|l| l.strip_prefix(&prefix))
        .filter_map(|rest| {
            let (line, rest) = rest.split_once(':')?;
            let (col, rest) = rest.split_once(": ")?;
            let (kind, message) = rest.split_once(": ")?;
            kind.ends_with("error").then(|| format!("{line}:{col} {message}"))
        })
        .collect()
}

/// The editor's errors located in `file`, as `line:col message`.
fn editor_refusals(file: &Path) -> BTreeSet<String> {
    editor_refusals_with(file, Config::editor())
}

/// The editor's, configured for `--target wasm32`.
fn editor_refusals_on_wasm32(file: &Path) -> BTreeSet<String> {
    let spec = hale_types::target::TargetSpec::parse("wasm32").unwrap();
    let target = hale_frontend::snapshot::Target { name: spec.triple.to_string(), spec, explicit: true };
    editor_refusals_with(file, Config { target, ..Config::editor() })
}

fn editor_refusals_with(file: &Path, config: Config) -> BTreeSet<String> {
    let snap = match Snapshot::load(file, LoadMode::Editor, &Disk, config) {
        Ok(s) => s,
        Err(_) => panic!("the editor could not load {}", file.display()),
    };
    let src = std::fs::read_to_string(file).unwrap();
    // Spans are bundle-global: the file's own slice of that space.
    let canonical = file.canonicalize().unwrap();
    let (base, len) = snap
        .file_bases()
        .iter()
        .find(|(_, p, _)| p.canonicalize().is_ok_and(|p| p == canonical))
        .map(|(b, _, l)| (*b as usize, *l as usize))
        .unwrap_or((0, src.len()));
    let diags = snap.demand_check().map(|c| c.diags.clone()).unwrap_or_else(|b| b.because.clone());
    diags
        .iter()
        .filter(|d| d.is_error())
        .filter(|d| (base..=base + len).contains(&(d.span.start.0 as usize)))
        .map(|d| {
            let at = d.span.start.0 as usize - base;
            let before = &src[..at.min(src.len())];
            let line = before.matches('\n').count() + 1;
            let col = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
            format!("{line}:{col} {}", d.message)
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Effective {
    Host,
    Wasm32,
    Musl,
}

/// What a cell expects: the effective target, or a refusal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Expect {
    Admitted(Effective),
    /// Refused at the declaration (line 1).
    Conflict,
    /// `--wrap-main` without wasm32.
    WrapRefused,
}

const DECLARED: &str = "fn main() {\n    println(\"ok\");\n}\n";

fn source(row: &str) -> String {
    match row {
        "none" | "wrap" => DECLARED.to_string(),
        "wasm" => format!("target wasm {{ }}\n\n{DECLARED}"),
        "browser_js" => format!("target browser_js {{ }}\n\n{DECLARED}"),
        _ => unreachable!(),
    }
}

fn host_triple() -> String {
    hale_types::target::TargetSpec::host().triple.to_string()
}

fn conflict(name: &str, triple: &str) -> String {
    format!(
        "1:1 this program declares `target {name}`, and is being checked for `{triple}`: build it with \
         `--target wasm32`, or drop the declaration"
    )
}

/// One cell: check, build and (with no `--target`) the editor.
fn cell(row: &str, cli: Option<&str>, expect: Expect, host: &str) {
    let dir = case_dir(&format!("{row}_{}", cli.unwrap_or("none")));
    let file = dir.join("t.hl");
    std::fs::write(&file, source(row)).unwrap();
    let f = file.to_str().unwrap();
    let mut target_args: Vec<&str> = Vec::new();
    if let Some(t) = cli {
        target_args.extend(["--target", t]);
    }
    let what = format!("{row} × {}", cli.unwrap_or("none"));

    let mut build_args = vec!["build", f];
    build_args.extend(&target_args);
    if row == "wrap" {
        build_args.push("--wrap-main");
    }
    let (build, build_code) = hale(&build_args);
    let mut check_args = vec!["check", f];
    check_args.extend(&target_args);
    let (check, check_code) = hale(&check_args);

    let triple = match cli {
        Some("native") | None => host.to_string(),
        Some(t) => t.to_string(),
    };
    match expect {
        Expect::Admitted(eff) => {
            assert_eq!(check_code, 0, "{what}: check refused it:\n{check}");
            match eff {
                Effective::Host => {
                    assert_eq!(build_code, 0, "{what}: build refused it:\n{build}");
                    assert!(file.with_extension("").exists(), "{what}: no host executable:\n{build}");
                    assert!(!file.with_extension("wasm").exists(), "{what}: a wasm module for a host target");
                }
                Effective::Wasm32 => {
                    assert_eq!(build_code, 0, "{what}: build refused it:\n{build}");
                    assert!(file.with_extension("wasm").exists(), "{what}: no wasm module:\n{build}");
                    assert!(file.with_extension("mjs").exists(), "{what}: no loader:\n{build}");
                    assert!(!file.with_extension("").exists(), "{what}: a host executable for wasm32");
                }
                Effective::Musl => {
                    // The check admits it; the build may still end at the
                    // cross toolchain on a host without zig — a failure past
                    // the check that names its own cause.
                    assert!(cli_refusals(&build, &file).is_empty(), "{what}: build refused it:\n{build}");
                    if build_code != 0 {
                        assert!(build.contains("zig") || build.contains("sysroot"), "{what}:\n{build}");
                    }
                }
            }
        }
        Expect::Conflict => {
            let want: BTreeSet<String> = [conflict(row, &triple)].into();
            assert_ne!(check_code, 0, "{what}: check admitted it");
            assert_ne!(build_code, 0, "{what}: build admitted it");
            assert_eq!(cli_refusals(&check, &file), want, "{what}: check:\n{check}");
            assert_eq!(cli_refusals(&build, &file), want, "{what}: build:\n{build}");
            assert!(!file.with_extension("").exists() && !file.with_extension("wasm").exists());
        }
        Expect::WrapRefused => {
            assert_eq!(build_code, 2, "{what}: build admitted it:\n{build}");
            assert!(build.contains("--wrap-main requires --target wasm32"), "{what}:\n{build}");
        }
    }
    if row != "wrap" {
        // check and build agree on the located refusals.
        assert_eq!(cli_refusals(&check, &file), cli_refusals(&build, &file), "{what}: check vs build");
        if cli.is_none() {
            assert_eq!(editor_refusals(&file), cli_refusals(&check, &file), "{what}: editor vs check");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

const MUSL: &str = "x86_64-unknown-linux-musl";

/// One row of the table of design §1.3: the source is the row, the
/// columns are the CLI's `--target` — none, `native`, the host's
/// triple, `wasm32`, `wasm32-unknown-unknown`, a musl triple.
fn row(row: &str, expects: [Expect; 6]) {
    if !wasm_toolchain() {
        eprintln!("SKIP target_precedence row `{row}`: no wasm32 clang or wasm-ld");
        return;
    }
    let host = host_triple();
    let cols: [Option<&str>; 6] =
        [None, Some("native"), Some(host.as_str()), Some("wasm32"), Some("wasm32-unknown-unknown"), Some(MUSL)];
    for (cli, expect) in cols.iter().zip(expects) {
        cell(row, *cli, expect, &host);
    }
}

#[test]
fn no_declaration_is_the_configured_target_or_the_host() {
    use Effective::*;
    use Expect::*;
    row("none", [Admitted(Host), Admitted(Host), Admitted(Host), Admitted(Wasm32), Admitted(Wasm32), Admitted(Musl)]);
}

#[test]
fn a_target_wasm_declaration_selects_wasm32_and_refuses_another_class() {
    use Effective::*;
    use Expect::*;
    row("wasm", [Admitted(Wasm32), Conflict, Conflict, Admitted(Wasm32), Admitted(Wasm32), Conflict]);
}

#[test]
fn target_browser_js_is_as_target_wasm() {
    use Effective::*;
    use Expect::*;
    row("browser_js", [Admitted(Wasm32), Conflict, Conflict, Admitted(Wasm32), Admitted(Wasm32), Conflict]);
}

/// `--wrap-main` needs wasm32 in any spelling (`wasm32-unknown-unknown`
/// was refused by a string guard before), and the declaration it
/// injects never selects it.
#[test]
fn wrap_main_needs_a_wasm32_effective_target() {
    use Effective::*;
    use Expect::*;
    row("wrap", [WrapRefused, WrapRefused, WrapRefused, Admitted(Wasm32), Admitted(Wasm32), WrapRefused]);
}

/// `iris/examples/wasm-flower` declares `target wasm` and exports only:
/// it builds for wasm32 with no `--target` (it used to build natively
/// and fail late, "program has no `fn main()`"), the same with
/// `--target wasm32`, and is refused at its declaration for the host.
/// The editor agrees with the check.
#[test]
fn wasm_flower_builds_for_its_declared_target() {
    if !wasm_toolchain() {
        eprintln!("SKIP wasm_flower_builds_for_its_declared_target: no wasm32 clang or wasm-ld");
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let src = std::fs::read_to_string(root.join("iris/examples/wasm-flower/flower.hl")).unwrap();
    for cli in [None, Some("wasm32")] {
        let dir = case_dir(&format!("flower_{}", cli.unwrap_or("none")));
        let file = dir.join("flower.hl");
        std::fs::write(&file, &src).unwrap();
        let f = file.to_str().unwrap();
        let mut args = vec!["build", f];
        args.extend(cli.map(|t| ["--target", t]).iter().flatten());
        let (text, code) = hale(&args);
        assert_eq!(code, 0, "{cli:?}: {text}");
        assert!(dir.join("flower.wasm").exists() && dir.join("flower.mjs").exists(), "{cli:?}: {text}");
        let (check, code) = hale(&["check", f]);
        assert_eq!(code, 0, "{check}");
        assert!(editor_refusals(&file).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
    let dir = case_dir("flower_native");
    let file = dir.join("flower.hl");
    std::fs::write(&file, &src).unwrap();
    let (text, code) = hale(&["build", file.to_str().unwrap(), "--target", "native"]);
    assert_eq!(code, 1, "{text}");
    let line = src.lines().position(|l| l.starts_with("target wasm")).unwrap() + 1;
    let want = conflict("wasm", &host_triple()).replacen("1:1", &format!("{line}:1"), 1);
    assert_eq!(cli_refusals(&text, &file), [want].into());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Paired case 12 (design §5): a call through an import alias into a
/// seed whose fn reaches a refused namespace is refused at the call
/// that crosses into the seed, with the chain down to the primitive;
/// the seed's own body is beyond the horizon and carries nothing. Check,
/// build and (declared) the editor agree; the host admits it.
#[test]
fn an_import_alias_is_refused_at_the_crossing_call() {
    if !wasm_toolchain() {
        eprintln!("SKIP an_import_alias_is_refused_at_the_crossing_call: no wasm32 clang or wasm-ld");
        return;
    }
    let body = "fn main() {\n    let t = c::stamp();\n    println(t);\n}\n";
    for (decl, cli, selector) in
        [("", Some("wasm32"), "`--target wasm32`"), ("target wasm { }\n", None, "`target wasm`")]
    {
        let dir = case_dir("alias");
        std::fs::create_dir_all(dir.join("clocklib")).unwrap();
        std::fs::write(dir.join("clocklib/clock.hl"), "fn stamp() -> Int {\n    return std::process::pid();\n}\n").unwrap();
        let file = dir.join("main.hl");
        std::fs::write(&file, format!("import \"clocklib\" as c;\n{decl}\n{body}")).unwrap();
        let f = file.to_str().unwrap();
        let line = 4 + decl.lines().count();
        let want: BTreeSet<String> = [format!(
            "{line}:13 `std::process` is unavailable under {selector}: {PROCESS} — witness: `c::stamp` → \
             `std::process::pid`"
        )]
        .into();
        let mut tail = vec![f];
        tail.extend(cli.map(|t| ["--target", t]).iter().flatten());
        let (check, code) = hale(&[&["check"], tail.as_slice()].concat());
        assert_eq!(code, 1, "{check}");
        assert_eq!(cli_refusals(&check, &file), want, "check:\n{check}");
        assert!(!check.contains("clock.hl:"), "the seed's own body is beyond the horizon:\n{check}");
        let (build, _) = hale(&[&["build"], tail.as_slice()].concat());
        assert_eq!(cli_refusals(&build, &file), want, "build:\n{build}");
        if cli.is_none() {
            assert_eq!(editor_refusals(&file), want, "editor");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    let dir = case_dir("alias_host");
    std::fs::create_dir_all(dir.join("clocklib")).unwrap();
    std::fs::write(dir.join("clocklib/clock.hl"), "fn stamp() -> Int {\n    return std::process::pid();\n}\n").unwrap();
    std::fs::write(dir.join("main.hl"), format!("import \"clocklib\" as c;\n\n{body}")).unwrap();
    let (check, code) = hale(&["check", dir.join("main.hl").to_str().unwrap()]);
    assert_eq!(code, 0, "the host admits it:\n{check}");
    let _ = std::fs::remove_dir_all(&dir);
}

const PROCESS: &str = "OS process control (`std::process`) isn't available in the browser";

/// The selections a wasm32 refusal is held to: no declaration and
/// `--target wasm32`, or a written `target wasm { }` and no flag.
const SELECTIONS: [(&str, Option<&str>, &str); 2] =
    [("", Some("wasm32"), "`--target wasm32`"), ("target wasm { }\n", None, "`target wasm`")];

/// `hale check`, `hale build` and the editor's `demand_check`, each
/// configured as `cli` names, refuse `file` with exactly `want`; the
/// seed's own files print nothing.
fn refused_at_every_entry_point(file: &Path, cli: Option<&str>, want: &BTreeSet<String>, seed_file: Option<&str>) {
    let mut tail = vec![file.to_str().unwrap()];
    tail.extend(cli.map(|t| ["--target", t]).iter().flatten());
    let (check, code) = hale(&[&["check"], tail.as_slice()].concat());
    assert_eq!(code, 1, "{check}");
    assert_eq!(&cli_refusals(&check, file), want, "check:\n{check}");
    if let Some(seed) = seed_file {
        assert!(!check.contains(&format!("{seed}:")), "the seed's own body is beyond the horizon:\n{check}");
    }
    let (build, code) = hale(&[&["build"], tail.as_slice()].concat());
    assert_eq!(code, 1, "{build}");
    assert_eq!(&cli_refusals(&build, file), want, "build:\n{build}");
    let editor = match cli {
        Some(_) => editor_refusals_on_wasm32(file),
        None => editor_refusals(file),
    };
    assert_eq!(&editor, want, "editor");
}

/// The review of #1318: a call in an index operand reached neither a use
/// row nor a hole, so the program passed under wasm32. Refused at the
/// call through every entry point, declared and flagged; the host
/// admits it.
#[test]
fn an_index_operand_is_refused_at_every_entry_point() {
    if !wasm_toolchain() {
        eprintln!("SKIP an_index_operand_is_refused_at_every_entry_point: no wasm32 clang or wasm-ld");
        return;
    }
    let body = "fn main() {\n    let xs = [0];\n    let _ = xs[std::process::pid()];\n}\n";
    for (decl, cli, selector) in SELECTIONS {
        let dir = case_dir("index");
        let file = dir.join("main.hl");
        std::fs::write(&file, format!("{decl}{body}")).unwrap();
        let line = 3 + decl.lines().count();
        let want = [format!("{line}:16 `std::process::pid` is unavailable under {selector}: {PROCESS}")].into();
        refused_at_every_entry_point(&file, cli, &want, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
    let dir = case_dir("index_host");
    std::fs::write(dir.join("main.hl"), body).unwrap();
    let (check, code) = hale(&["check", dir.join("main.hl").to_str().unwrap()]);
    assert_eq!(code, 0, "the host admits it:\n{check}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An imported wrapper whose index operand calls an unavailable
/// operation: the seed's body is beyond the horizon, so the call that
/// crosses into it is refused, with the chain through the subscript.
#[test]
fn an_imported_wrappers_index_operand_is_refused_at_the_crossing_call() {
    if !wasm_toolchain() {
        eprintln!("SKIP an_imported_wrappers_index_operand_is_refused_at_the_crossing_call: no wasm32 clang or wasm-ld");
        return;
    }
    let lib = "fn pick() -> Int {\n    let xs = [0];\n    return xs[std::process::pid()];\n}\n";
    let body = "fn main() {\n    println(p::pick());\n}\n";
    for (decl, cli, selector) in SELECTIONS {
        let dir = case_dir("index_alias");
        std::fs::create_dir_all(dir.join("picklib")).unwrap();
        std::fs::write(dir.join("picklib/pick.hl"), lib).unwrap();
        let file = dir.join("main.hl");
        std::fs::write(&file, format!("import \"picklib\" as p;\n{decl}{body}")).unwrap();
        let line = 3 + decl.lines().count();
        let want = [format!(
            "{line}:13 `std::process` is unavailable under {selector}: {PROCESS} — witness: `p::pick` → \
             `std::process::pid`"
        )]
        .into();
        refused_at_every_entry_point(&file, cli, &want, Some("pick.hl"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// One case of a seed `kidlib/kid.hl` and a `main.hl` written after the
/// selection's declaration: refused with `want(line offset, selector)`
/// at every entry point, the seed's own file printing nothing. `None`
/// for `lib`: the program declares everything itself.
fn seeded_case(tag: &str, lib: Option<&str>, main: &str, want: impl Fn(usize, &str) -> Vec<String>) {
    for (decl, cli, selector) in SELECTIONS {
        let dir = case_dir(tag);
        if let Some(lib) = lib {
            std::fs::create_dir_all(dir.join("kidlib")).unwrap();
            std::fs::write(dir.join("kidlib/kid.hl"), lib).unwrap();
        }
        let file = dir.join("main.hl");
        let import = if lib.is_some() { "import \"kidlib\" as lib;\n" } else { "" };
        std::fs::write(&file, format!("{import}{decl}{main}")).unwrap();
        let offset = import.lines().count() + decl.lines().count();
        let want = want(offset, selector).into_iter().collect();
        refused_at_every_entry_point(&file, cli, &want, lib.map(|_| "kid.hl"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Admitted by check, build and the editor under every selection and
/// on the host.
fn admitted_everywhere(tag: &str, lib: &str, main: &str) {
    for (decl, cli, _) in SELECTIONS.into_iter().chain([("", None, "the host")]) {
        let dir = case_dir(tag);
        std::fs::create_dir_all(dir.join("kidlib")).unwrap();
        std::fs::write(dir.join("kidlib/kid.hl"), lib).unwrap();
        let file = dir.join("main.hl");
        std::fs::write(&file, format!("import \"kidlib\" as lib;\n{decl}{main}")).unwrap();
        let mut tail = vec![file.to_str().unwrap()];
        tail.extend(cli.map(|t| ["--target", t]).iter().flatten());
        let (check, code) = hale(&[&["check"], tail.as_slice()].concat());
        assert_eq!(code, 0, "{decl}{cli:?}: {check}");
        let (build, _) = hale(&[&["build"], tail.as_slice()].concat());
        assert!(cli_refusals(&build, &file).is_empty(), "{decl}{cli:?}: {build}");
        let editor = if cli.is_some() { editor_refusals_on_wasm32(&file) } else { editor_refusals(&file) };
        assert!(editor.is_empty(), "{decl}{cli:?}: {editor:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

const KID: &str = "locus Kid {\n    params { n: Int = std::process::pid(); }\n    run() { println(self.n); }\n}\n";

/// The review of #1318: the horizon relocates a refusal to the
/// construction; it never erases a requirement. A params default has no
/// summary body, so the same locus was refused declared in the program
/// and admitted imported. Declared, it is refused at its default; imported,
/// at the `lib::Kid { }` construction, naming the capability and the
/// witness through the default — and at the call into a seed fn that
/// constructs it. The host admits all three.
#[test]
fn a_default_is_refused_declared_and_imported() {
    if !wasm_toolchain() {
        eprintln!("SKIP a_default_is_refused_declared_and_imported: no wasm32 clang or wasm-ld");
        return;
    }
    seeded_case("default_own", None, &format!("{KID}\nfn main() {{ Kid {{ }}; }}\n"), |at, selector| {
        vec![format!("{}:23 `std::process::pid` is unavailable under {selector}: {PROCESS}", at + 2)]
    });
    seeded_case("default_imported", Some(KID), "fn main() { lib::Kid { }; }\n", |at, selector| {
        vec![format!(
            "{}:13 `std::process` is unavailable under {selector}: {PROCESS} — witness: `lib::Kid` → \
             `params {{ n }}` → `std::process::pid`",
            at + 1
        )]
    });
    let maker = format!("{KID}\nfn make() {{\n    Kid {{ }};\n}}\n");
    seeded_case("default_imported_fn", Some(&maker), "fn main() { lib::make(); }\n", |at, selector| {
        vec![format!(
            "{}:13 `std::process` is unavailable under {selector}: {PROCESS} — witness: `lib::make` → `lib::Kid` \
             → `params {{ n }}` → `std::process::pid`",
            at + 1
        )]
    });
    for (lib, main) in [(None, format!("{KID}\nfn main() {{ Kid {{ }}; }}\n")), (Some(KID), "fn main() { lib::Kid { }; }\n".into())] {
        let dir = case_dir("default_host");
        if let Some(lib) = lib {
            std::fs::create_dir_all(dir.join("kidlib")).unwrap();
            std::fs::write(dir.join("kidlib/kid.hl"), lib).unwrap();
        }
        let import = if lib.is_some() { "import \"kidlib\" as lib;\n" } else { "" };
        std::fs::write(dir.join("main.hl"), format!("{import}{main}")).unwrap();
        let (check, code) = hale(&["check", dir.join("main.hl").to_str().unwrap()]);
        assert_eq!(code, 0, "the host admits it:\n{check}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

const ONCE: &str = "locus Once {\n    params { runs: Int = 0; }\n    closure fuse { captures: runs; epoch inline; }\n    \
                    run() {\n        self.runs = self.runs + 1;\n        if self.runs < 2 { violate fuse; }\n    }\n}\n\n";

/// The same boundary for an `on_failure` handler, which the summary
/// keys no body for either: declared, refused at the call in the
/// handler; imported, at the construction, through `on_failure()` —
/// down a method the handler calls on a params field, resolved through
/// the field's declared type.
#[test]
fn an_on_failure_body_is_refused_declared_and_imported() {
    if !wasm_toolchain() {
        eprintln!("SKIP an_on_failure_body_is_refused_declared_and_imported: no wasm32 clang or wasm-ld");
        return;
    }
    let keeper = format!(
        "{ONCE}locus Keeper {{\n    params {{ early: Once = Once {{ }}; }}\n    \
         on_failure(c: Once, err: ClosureViolation) {{\n        std::process::exit(1);\n    }}\n}}\n"
    );
    seeded_case("failure_own", None, &format!("{keeper}\nfn main() {{ Keeper {{ }}; }}\n"), |at, selector| {
        vec![format!("{}:9 `std::process::exit` is unavailable under {selector}: {PROCESS}", at + 13)]
    });
    seeded_case("failure_imported", Some(&keeper), "fn main() { lib::Keeper { }; }\n", |at, selector| {
        vec![format!(
            "{}:13 `std::process` is unavailable under {selector}: {PROCESS} — witness: `lib::Keeper` → \
             `on_failure()` → `std::process::exit`",
            at + 1
        )]
    });
    let minder = format!(
        "{ONCE}locus Helper {{\n    params {{ v: Int = 0; }}\n    fn ping() -> Int {{ return std::process::pid(); }}\n}}\n\n\
         locus Minder {{\n    params {{ early: Once = Once {{ }}; h: Helper = Helper {{ }}; seen: Int = 0; }}\n    \
         on_failure(c: Once, err: ClosureViolation) {{\n        self.seen = self.h.ping();\n    }}\n}}\n"
    );
    seeded_case("failure_method", Some(&minder), "fn main() { lib::Minder { }; }\n", |at, selector| {
        vec![format!(
            "{}:13 `std::process` is unavailable under {selector}: {PROCESS} — witness: `lib::Minder` → \
             `on_failure()` → `lib::Helper::ping` → `std::process::pid`",
            at + 1
        )]
    });
}

/// The control: an imported default that asks the target for nothing is
/// admitted wherever the program is.
#[test]
fn a_portable_imported_default_is_admitted() {
    if !wasm_toolchain() {
        eprintln!("SKIP a_portable_imported_default_is_admitted: no wasm32 clang or wasm-ld");
        return;
    }
    admitted_everywhere(
        "default_portable",
        "locus Calm {\n    params { n: Int = len(\"abc\"); }\n    run() { println(self.n); }\n}\n",
        "fn main() { lib::Calm { }; }\n",
    );
}

const PID: &str = "fn pid() -> Int { return std::process::pid(); }\n\n";

/// The review of #1318, round 2: a call through a local function value
/// in an imported initializer met nothing, so the construction carried
/// neither the requirement nor a hole. The local resolves to the fn it
/// is bound to, and the construction is refused with the witness through
/// it, as the direct call is; the host admits both.
#[test]
fn a_call_through_a_local_fn_value_is_refused_at_the_construction() {
    if !wasm_toolchain() {
        eprintln!("SKIP a_call_through_a_local_fn_value_is_refused_at_the_construction: no wasm32 clang or wasm-ld");
        return;
    }
    for init in ["{ let f = pid; f() }", "pid()"] {
        let lib = format!("{PID}locus Kid {{\n    params {{ n: Int = {init}; }}\n    run() {{ println(self.n); }}\n}}\n");
        seeded_case("fn_value", Some(&lib), "fn main() { lib::Kid { }; }\n", |at, selector| {
            vec![format!(
                "{}:13 `std::process` is unavailable under {selector}: {PROCESS} — witness: `lib::Kid` → \
                 `params {{ n }}` → `lib::pid` → `std::process::pid`",
                at + 1
            )]
        });
        let dir = case_dir("fn_value_host");
        std::fs::create_dir_all(dir.join("kidlib")).unwrap();
        std::fs::write(dir.join("kidlib/kid.hl"), &lib).unwrap();
        std::fs::write(dir.join("main.hl"), "import \"kidlib\" as lib;\nfn main() { lib::Kid { }; }\n").unwrap();
        let (check, code) = hale(&["check", dir.join("main.hl").to_str().unwrap()]);
        assert_eq!(code, 0, "{init}: the host admits it:\n{check}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The same walk serves an imported `on_failure` handler: a call through
/// a local bound to a seed fn is refused at the construction, through
/// `on_failure()`.
#[test]
fn a_call_through_a_local_fn_value_in_on_failure_is_refused() {
    if !wasm_toolchain() {
        eprintln!("SKIP a_call_through_a_local_fn_value_in_on_failure_is_refused: no wasm32 clang or wasm-ld");
        return;
    }
    let keeper = format!(
        "{ONCE}{PID}locus Keeper {{\n    params {{ early: Once = Once {{ }}; seen: Int = 0; }}\n    \
         on_failure(c: Once, err: ClosureViolation) {{\n        let f = pid;\n        self.seen = f();\n    }}\n}}\n"
    );
    seeded_case("fn_value_failure", Some(&keeper), "fn main() { lib::Keeper { }; }\n", |at, selector| {
        vec![format!(
            "{}:13 `std::process` is unavailable under {selector}: {PROCESS} — witness: `lib::Keeper` → \
             `on_failure()` → `lib::pid` → `std::process::pid`",
            at + 1
        )]
    });
}

/// A local the walk cannot resolve — here bound to a function-typed
/// params field — is a hole at the construction, never nothing: wasm32
/// cannot admit what the call might need.
#[test]
fn a_call_through_an_unresolved_fn_value_is_a_hole() {
    if !wasm_toolchain() {
        eprintln!("SKIP a_call_through_an_unresolved_fn_value_is_a_hole: no wasm32 clang or wasm-ld");
        return;
    }
    let lib = format!(
        "{PID}locus Kid {{\n    params {{ g: fn() -> Int = pid; n: Int = {{ let f = self.g; f() }}; }}\n    \
         run() {{ println(self.n); }}\n}}\n"
    );
    seeded_case("fn_value_hole", Some(&lib), "fn main() { lib::Kid { }; }\n", |at, _| {
        vec![format!(
            "{}:13 cannot establish what `lib::Kid` requires on wasm32: the callee is a function value the \
             summary cannot resolve — witness: `lib::Kid` → `params {{ n }}` → `f()`",
            at + 1
        )]
    });
}

/// The control: a local bound to a seed fn that asks the target for
/// nothing is admitted wherever the program is.
#[test]
fn a_portable_local_fn_value_is_admitted() {
    if !wasm_toolchain() {
        eprintln!("SKIP a_portable_local_fn_value_is_admitted: no wasm32 clang or wasm-ld");
        return;
    }
    admitted_everywhere(
        "fn_value_portable",
        "fn width(s: String) -> Int { return len(s); }\n\n\
         locus Calm {\n    params { n: Int = { let f = width; f(\"abc\") }; }\n    run() { println(self.n); }\n}\n",
        "fn main() { lib::Calm { }; }\n",
    );
}

/// `hale run` executes what it builds, and a declared program builds a
/// wasm32 module: refused, as `--target wasm32` is.
#[test]
fn run_refuses_a_declared_wasm_program() {
    let dir = case_dir("run_declared");
    let file = dir.join("t.hl");
    std::fs::write(&file, source("wasm")).unwrap();
    let (text, code) = hale(&["run", file.to_str().unwrap()]);
    assert_eq!(code, 2, "{text}");
    assert!(!text.contains("\nok"), "{text}");
    assert!(
        text.contains(
            "hale run: this program declares `target wasm`, so it builds a wasm32 module this host \
             cannot execute"
        ),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The files whose programs are the wasm-relevant set of design §2.9:
/// the Rust test strings that build, check or gate for wasm32, the
/// flower, and the playground.
const WASM_ORIGINS: &[&str] = &[
    "crates/hale-types/tests/wasm_target_gating.rs",
    "crates/hale-codegen/tests/wasm_target.rs",
    "crates/hale-cli/tests/wasm_package_csrc.rs",
    "crates/hale-cli/tests/target_model.rs",
    "crates/hale-cli/tests/build_output_path.rs",
    "crates/hale-cli/tests/wasm_link_is_quiet.rs",
    "crates/hale-cli/tests/check_arg_parsing.rs",
    "crates/hale-syntax/tests/wrap_main.rs",
    "iris/examples/wasm-flower/",
    "play/",
];

/// The agreement test (design §5, P3 2 of 3 item 4): every wasm-relevant
/// program in the tree, under equivalent configuration, gets the same
/// located refusals from `hale check`, `hale build` and the editor —
/// with no `--target` (the editor's configuration, so all three), and
/// with `--target wasm32` (check against build).
#[test]
fn the_wasm_programs_agree_across_check_build_and_the_editor() {
    if !wasm_toolchain() {
        eprintln!("SKIP the_wasm_programs_agree_across_check_build_and_the_editor: no wasm32 clang or wasm-ld");
        return;
    }
    let root = hale_corpus::repo_root();
    let mut programs = hale_corpus::all();
    for rel in ["iris/examples/wasm-flower", "play"] {
        walk_hl(&root.join(rel), &root, &mut programs);
    }
    let mut seen = BTreeSet::new();
    programs.retain(|p| WASM_ORIGINS.iter().any(|o| p.origin.starts_with(o)) && seen.insert(p.source.clone()));
    assert!(programs.len() >= 20, "the wasm-relevant set shrank to {}", programs.len());

    // Each program has its own directory, so they run side by side.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let disagreements = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(p) = programs.get(i) else { break };
                disagreements.lock().unwrap().extend(agreement(i, p));
            });
        }
    });
    let disagreements = disagreements.into_inner().unwrap();
    assert!(
        disagreements.is_empty(),
        "{} of {} programs disagree:\n{}",
        disagreements.len(),
        programs.len(),
        disagreements.join("\n")
    );
}

/// One program's disagreements between check, build and the editor.
fn agreement(i: usize, p: &hale_corpus::Program) -> Vec<String> {
    let mut disagreements = Vec::new();
    let dir = case_dir(&format!("agree_{i}"));
    let file = dir.join("t.hl");
    std::fs::write(&file, &p.source).unwrap();
    let f = file.to_str().unwrap();
    for cli in [None, Some("wasm32")] {
        let mut tail: Vec<&str> = vec![f];
        if let Some(t) = cli {
            tail.extend(["--target", t]);
        }
        let (check, _) = hale(&[&["check"], tail.as_slice()].concat());
        let (build, _) = hale(&[&["build"], tail.as_slice()].concat());
        let check = cli_refusals(&check, &file);
        let build = cli_refusals(&build, &file);
        let what = format!("{} × {}", p.origin, cli.unwrap_or("none"));
        if check != build {
            disagreements.push(format!("{what}: check {check:?} vs build {build:?}"));
        }
        if cli.is_none() {
            let editor = editor_refusals(&file);
            if editor != check {
                disagreements.push(format!("{what}: editor {editor:?} vs check {check:?}"));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    disagreements
}

fn walk_hl(dir: &Path, root: &Path, out: &mut Vec<hale_corpus::Program>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk_hl(&p, root, out);
        } else if p.extension().is_some_and(|e| e == "hl") {
            if let Ok(source) = std::fs::read_to_string(&p) {
                let origin = p.strip_prefix(root).unwrap_or(&p).display().to_string();
                out.push(hale_corpus::Program { origin, source });
            }
        }
    }
}

/// Whether this box can build wasm32: a clang with the wasm32 backend
/// and wasm-ld (bare or `-18`), as the wasm suite asks.
fn wasm_toolchain() -> bool {
    let tool = |names: &[&str]| {
        names.iter().any(|n| Command::new(n).arg("--version").output().is_ok_and(|o| o.status.success()))
    };
    if !tool(&["wasm-ld", "wasm-ld-18"]) {
        return false;
    }
    let dir = case_dir("probe");
    let c = dir.join("p.c");
    std::fs::write(&c, "int x;\n").unwrap();
    let ok = ["clang", "clang-18"].iter().any(|cc| {
        Command::new(cc)
            .args(["--target=wasm32", "-c"])
            .arg(&c)
            .arg("-o")
            .arg(dir.join("p.o"))
            .output()
            .is_ok_and(|o| o.status.success())
    });
    let _ = std::fs::remove_dir_all(&dir);
    ok
}
