use std::path::PathBuf;
use std::env;
use std::process::ExitCode;
use std::path::Path;
use std::fs;
/// `hale init [dir]` — bootstrap a project. Writes the canonical
/// minimal scaffold: a `hale.toml` skeleton (the manifest is
/// `[deps]`-only by design — a project is identified by its
/// directory name and its source, per spec/packages.md), a
/// hello-world `main.hl`, a first `tests/*_test.hl` so `hale test` works
/// from minute one, and a `.gitignore` covering the build artifact
/// and `vendor/`. Strictly non-destructive: an existing file is
/// never touched, only reported — so `init` is also safe to run in
/// a partially-scaffolded directory to fill in what's missing.
pub(crate) fn run_init(root: &Path) -> ExitCode {
    if let Err(e) = fs::create_dir_all(root) {
        eprintln!("hale init: cannot create {}: {}", root.display(), e);
        return ExitCode::from(1);
    }
    let name = root
        .canonicalize()
        .ok()
        .and_then(|p| {
            p.file_name().map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "app".to_string());

    let manifest = "\
# hale.toml — the project manifest. The only section is [deps]:\n\
# a project is identified by its directory name and its source\n\
# (no [package] metadata table exists). `hale fetch` clones each\n\
# dep into vendor/<name>/ and pins the resolved SHA in hale.lock.\n\
#\n\
# [deps]\n\
# helpers = { git = \"https://github.com/me/helpers\", tag = \"v0.1.0\" }\n\
\n\
[deps]\n";

    let main_hl = r#"/// Entry seed. Every `.hl` file in this directory shares one
/// scope — decompose by concern, not by visibility.
fn greeting() -> String {
    return "Hello from Hale.";
}

fn main() {
    println(greeting());
}
"#;

    // Tests live in a SUBDIRECTORY: a seed is the `.hl` files
    // directly in one directory, and a test file carries its own
    // `fn main` — beside main.hl it would collide. `tests/` is its
    // own seed importing the parent, the pond/stdlib convention.
    let test_hl = r#"// `hale test` discovers *_test.hl recursively. Each test file is
// its own program: it imports the seed under test and asserts —
// typechecked, next to the code.

import ".." as app;

fn main() {
    std::test::assert_eq_str(app::greeting(), "Hello from Hale.", "greeting");
}
"#;

    let gitignore = format!(
        "# the build artifact (`hale build .` names it after the directory)\n\
         /{}\n\
         # toolchain-managed dependency clones (`hale fetch`)\n\
         /vendor/\n",
        name
    );

    let files: &[(&str, &str)] = &[
        ("hale.toml", manifest),
        ("main.hl", main_hl),
        ("tests/main_test.hl", test_hl),
        (".gitignore", &gitignore),
    ];
    let mut wrote = 0usize;
    for (fname, content) in files {
        let path = root.join(fname);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if path.exists() {
            println!("kept    {} (already exists)", path.display());
            continue;
        }
        if let Err(e) = fs::write(&path, content) {
            eprintln!("hale init: cannot write {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
        println!("created {}", path.display());
        wrote += 1;
    }
    if wrote == 0 {
        println!("nothing to do — every scaffold file already exists");
    } else {
        println!();
        println!("next steps:");
        println!("    hale run {}      # compile + run", root.display());
        println!("    hale test {}     # run tests/", root.display());
        println!("    hale check {}    # typecheck + analyze", root.display());
    }
    ExitCode::SUCCESS
}

/// The dispatch arm `main` held inline for this verb, moved out verbatim (C5 step 8).
pub(crate) fn run_init_cmd(args: &[String]) -> ExitCode {
    let root = if args.len() >= 3 {
        PathBuf::from(&args[2])
    } else {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };
    return run_init(&root);
}
