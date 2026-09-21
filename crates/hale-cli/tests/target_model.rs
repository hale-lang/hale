//! The target model, from the outside (GH #445, PR 1).
//!
//! The exit criterion for the first Windows PR is narrow and worth
//! stating as a test rather than a claim: existing targets behave exactly
//! as before, and `x86_64-pc-windows-msvc` can be parsed and described.
//!
//! The second half is the easy half to get wrong in the flattering
//! direction. A target model that accepts a Windows triple and then dies
//! somewhere inside the linker has not "supported" anything — it has
//! moved the failure further from its cause. So these tests pin the
//! refusal too: naming a target the compiler cannot build must produce a
//! precise, early, actionable error.
//!
//! The same holds for a native triple that is not the host (GH #969): a
//! build that quietly emitted a host binary under that name was the
//! worst version of this. Such a triple is a cross target now (GH #970),
//! and these tests pin how far it goes: an object for the target, named
//! as one, and nothing that claims to be an executable.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// Per-process, per-case directory. These cases build real executables,
/// and the suite runs in parallel: two tests sharing an output path is
/// the `ETXTBSY`/wrong-binary failure `harness_paths_are_unique.rs`
/// exists to prevent in the codegen suite. Same hazard, same discipline.
fn case_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hale_target_model_{}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    dir
}

fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn list_targets_names_every_platform_and_its_tier() {
    let (stdout, _, code) = run(&["--list-targets"]);
    assert_eq!(code, 0, "--list-targets should succeed");

    for triple in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
        "wasm32-unknown-unknown",
    ] {
        assert!(stdout.contains(triple), "missing {triple} in:\n{stdout}");
    }

    // The listing must distinguish what it can build from what it can
    // merely name, or it is advertising.
    assert!(stdout.contains("supported: builds and links"), "{stdout}");
    assert!(stdout.contains("object-only"), "{stdout}");
    assert!(stdout.contains("planned"), "{stdout}");
    assert!(
        stdout.contains("(host)"),
        "host target not marked:\n{stdout}"
    );
}

#[test]
fn windows_target_is_described_with_its_own_file_conventions() {
    let (stdout, _, _) = run(&["--list-targets"]);
    let win = stdout
        .split("\n\n")
        .find(|b| b.starts_with("x86_64-pc-windows-msvc"))
        .unwrap_or_else(|| panic!("no windows block in:\n{stdout}"));

    // The whole point of the target model: these differ from the host's,
    // and they are answered by the target rather than by `cfg!`.
    assert!(win.contains(".obj"), "{win}");
    assert!(win.contains(".exe"), "{win}");
    assert!(win.contains(".lib"), "{win}");
    assert!(win.contains(".dll"), "{win}");
    assert!(win.contains("Msvc"), "{win}");
}

#[test]
fn building_for_windows_fails_early_and_says_why() {
    let dir = case_dir("target_model_win");
    let src = dir.join("t.hl");
    std::fs::write(&src, "fn main() { println(\"hi\"); }\n").unwrap();

    let (_, stderr, code) = run(&[
        "build",
        src.to_str().unwrap(),
        "--target",
        "x86_64-pc-windows-msvc",
    ]);

    assert_ne!(code, 0, "a planned target must not report success");
    assert!(stderr.contains("not buildable yet"), "{stderr}");
    assert!(stderr.contains("x86_64-pc-windows-msvc"), "{stderr}");
    // Point at the work, not at a dead end.
    assert!(
        stderr.contains("445"),
        "error should reference the issue:\n{stderr}"
    );
    // And it must fail at argument parsing, not after emitting something.
    assert!(
        !dir.join("t.exe").exists() && !dir.join("t").exists(),
        "a rejected target still produced an artifact"
    );
}

/// The native triples the compiler builds on some host, minus this one.
/// Chosen at run time so the test means the same thing on every CI
/// runner: whatever the host is, the others are foreign to it.
fn foreign_native_triples() -> Vec<&'static str> {
    let host = host_triple();
    [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
    ]
    .into_iter()
    .filter(|t| *t != host)
    .collect()
}

/// The host as the compiler itself reports it: the `(host)` block of
/// `--list-targets`, so the test asks the binary rather than
/// re-deriving the host with its own `cfg!`.
fn host_triple() -> String {
    let (stdout, _, _) = run(&["--list-targets"]);
    stdout
        .split("\n\n")
        .find(|b| b.contains("(host)"))
        .and_then(|b| b.split_whitespace().next())
        .unwrap_or_else(|| panic!("no (host) block in:\n{stdout}"))
        .to_string()
}

/// The tier line `--list-targets` prints for `triple`, from this host.
fn tier_of(triple: &str) -> String {
    let (stdout, _, _) = run(&["--list-targets"]);
    stdout
        .split("\n\n")
        .find(|b| b.starts_with(triple))
        .unwrap_or_else(|| panic!("no {triple} block in:\n{stdout}"))
        .lines()
        .last()
        .unwrap()
        .trim()
        .to_string()
}

/// The foreign native triples this host LINKS for (the Linux gnu ones,
/// through zig) and the ones it only emits an object for (Darwin, from
/// anywhere else), as the compiler itself sorts them.
fn cross_triples() -> Vec<&'static str> {
    foreign_native_triples()
        .into_iter()
        .filter(|t| tier_of(t).starts_with("cross from this host"))
        .collect()
}
fn object_only_triples() -> Vec<&'static str> {
    foreign_native_triples()
        .into_iter()
        .filter(|t| tier_of(t).starts_with("cross, object-only"))
        .collect()
}

/// What an object file says it is, read from its header: the format and
/// the machine. Enough to tell a host object from a target one without
/// any tool on PATH.
fn object_identity(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.len() >= 20 && &bytes[..4] == b"\x7fELF" {
        let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
        return Some(("elf", match machine {
            0x3e => "x86_64",
            0xb7 => "aarch64",
            _ => "other",
        }));
    }
    if bytes.len() >= 8 && bytes[..4] == [0xcf, 0xfa, 0xed, 0xfe] {
        let cpu = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        return Some(("macho", match cpu {
            0x0100_0007 => "x86_64",
            0x0100_000c => "aarch64",
            _ => "other",
        }));
    }
    None
}

/// The identity a triple's object must carry.
fn expected_identity(triple: &str) -> (&'static str, &'static str) {
    let format = if triple.contains("linux") { "elf" } else { "macho" };
    let arch = if triple.starts_with("x86_64") { "x86_64" } else { "aarch64" };
    (format, arch)
}

/// GH #969: a foreign native triple used to parse, build a HOST binary
/// under the requested name, and print `built:`. GH #970: it is its own
/// target now. One this host has no toolchain to link for (Darwin,
/// from anywhere else) is taken as far as wasm first went — a
/// relocatable object for the TARGET's format and architecture, named
/// as an object, with no executable beside it and a note saying so.
#[test]
fn a_foreign_native_triple_emits_an_object_for_that_target() {
    let triples = object_only_triples();
    assert!(!triples.is_empty(), "every host has a Darwin triple foreign to it");
    for triple in triples {
        let dir = case_dir("target_model_foreign");
        let src = dir.join("t.hl");
        std::fs::write(&src, "fn main() { println(\"hi\"); }\n").unwrap();

        let (_, stderr, code) =
            run(&["build", src.to_str().unwrap(), "--target", triple]);

        assert_eq!(code, 0, "{triple}: object build failed: {stderr}");
        let obj = dir.join("t.o");
        let bytes = std::fs::read(&obj)
            .unwrap_or_else(|e| panic!("{triple}: no object at {}: {e}", obj.display()));
        assert_eq!(
            object_identity(&bytes),
            Some(expected_identity(triple)),
            "{triple}: the object is not the target's"
        );
        assert!(
            !dir.join("t").exists(),
            "{triple}: a foreign build must not leave an executable"
        );
        assert!(stderr.contains("relocatable object"), "{stderr}");
        assert!(stderr.contains("970"), "should reference the issue:\n{stderr}");
    }
}

/// A Linux gnu triple foreign to this host is LINKED here, through
/// `zig cc` and a target sysroot (GH #970). Where both are present the
/// result is an executable for the target — the header says ELF, the
/// target's machine, and an executable type, and there is no `.o`
/// left as if the build had stopped short. Where either is missing the
/// build fails naming exactly that and how to get it, never with a
/// linker's undefined symbols and never with a host binary.
#[test]
fn a_cross_triple_links_or_names_what_is_missing() {
    let triples = cross_triples();
    assert!(!triples.is_empty(), "every host has a Linux triple foreign to it");
    for triple in triples {
        let dir = case_dir("target_model_cross");
        let src = dir.join("t.hl");
        std::fs::write(&src, "fn main() { println(\"hi\"); }\n").unwrap();

        let (stdout, stderr, code) =
            run(&["build", src.to_str().unwrap(), "--target", triple]);

        assert!(
            !dir.join("t.o").exists(),
            "{triple}: a cross build must not stop at an object"
        );
        if code == 0 {
            let bin = dir.join("t");
            let bytes = std::fs::read(&bin)
                .unwrap_or_else(|e| panic!("{triple}: no executable at {}: {e}", bin.display()));
            assert_eq!(
                object_identity(&bytes),
                Some(expected_identity(triple)),
                "{triple}: the executable is not the target's"
            );
            // e_type: ET_EXEC (2) or ET_DYN (3, a PIE) — not a
            // relocatable (1).
            let e_type = u16::from_le_bytes([bytes[16], bytes[17]]);
            assert!(matches!(e_type, 2 | 3), "{triple}: e_type {e_type}");
            assert!(!stderr.contains("relocatable object"), "{stderr}");
        } else {
            assert!(!stdout.contains("built:"), "{triple}: {stdout}");
            assert!(
                stderr.contains("zig") || stderr.contains("target-sysroot"),
                "{triple}: a cross build without its toolchain must say \
                 which piece is missing:\n{stderr}"
            );
            assert!(
                !stderr.contains("undefined symbol"),
                "{triple}: the failure reached the linker:\n{stderr}"
            );
        }
    }
}

/// `run` and `test` execute what they build, and nothing a foreign
/// target builds runs here: refused, like `--target wasm32`.
#[test]
fn run_refuses_a_foreign_native_triple() {
    let dir = case_dir("target_model_foreign_run");
    let src = dir.join("t.hl");
    std::fs::write(&src, "fn main() { println(\"hi\"); }\n").unwrap();
    let triple = foreign_native_triples()[0];

    let (stdout, stderr, code) =
        run(&["run", "--target", triple, src.to_str().unwrap()]);
    assert_ne!(code, 0, "run of a foreign target must not succeed");
    assert!(!stdout.contains("hi"), "{stdout}");
    assert!(stderr.contains("not this host's platform"), "{stderr}");
    assert!(stderr.contains(triple), "{stderr}");
}

/// `where async_io` is a property of the TARGET (GH #970). The check used
/// to ask `cfg!(target_os = "macos")` — the host — so a Mac refused an
/// `async_io` pool bound for Linux, and a Linux host accepted one bound
/// for macOS, whose runtime has no such backend.
#[test]
fn async_io_is_judged_against_the_target() {
    const PROG: &str = r#"
fn ignore_conn(s: std::io::tcp::Stream) { }

main locus App {
    params {
        listener: std::io::tcp::Listener = std::io::tcp::Listener {
            host:          "127.0.0.1",
            port:          0,
            max_accepts:   -1,
            on_connection: ignore_conn,
        };
    }
    placement {
        listener: cooperative(pool = io) where async_io;
    }
}

fn main() { App { }; }
"#;
    let foreign = foreign_native_triples();
    let linux = foreign.iter().find(|t| t.contains("linux-gnu")).unwrap();
    let musl = foreign.iter().find(|t| t.contains("linux-musl")).unwrap();
    let darwin = foreign.iter().find(|t| t.contains("darwin")).unwrap();

    let dir = case_dir("target_model_async_io");
    let src = dir.join("t.hl");
    std::fs::write(&src, PROG).unwrap();

    let (_, stderr, code) = run(&["build", src.to_str().unwrap(), "--target", linux]);
    // The check must pass it. The build may still fail later, at the
    // cross link, on a host without zig or a sysroot — that failure is
    // past the check and names its own cause.
    assert!(
        !stderr.contains("aren't supported on macOS"),
        "{linux} has async_io, the check refused it: {stderr}"
    );
    if code != 0 {
        assert!(
            stderr.contains("zig") || stderr.contains("target-sysroot"),
            "{linux}: failed for a reason other than the cross toolchain:\n{stderr}"
        );
    }

    let (_, stderr, code) = run(&["build", src.to_str().unwrap(), "--target", darwin]);
    assert_ne!(code, 0, "{darwin} has no async_io, the check accepted it");
    assert!(stderr.contains("aren't supported on macOS"), "{stderr}");

    // musl declares ucontext and implements none of it: no backend.
    let (_, stderr, code) = run(&["build", src.to_str().unwrap(), "--target", musl]);
    assert_ne!(code, 0, "{musl} has no async_io, the check accepted it");
    assert!(stderr.contains("aren't supported on musl Linux"), "{stderr}");
}

/// Naming the host by its triple is the same build as `native`.
#[test]
fn the_host_triple_builds_like_native() {
    let dir = case_dir("target_model_host_triple");
    let src = dir.join("t.hl");
    std::fs::write(&src, "fn main() { println(\"ok\"); }\n").unwrap();
    let host = host_triple();

    let (_, stderr, code) = run(&["build", src.to_str().unwrap(), "--target", &host]);
    assert_eq!(code, 0, "host-triple build failed: {stderr}");
    let out = Command::new(src.with_extension(""))
        .output()
        .expect("run built binary");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
}

/// The listing must say what a foreign native triple is from here — a
/// cross target linked through zig, or one only emitted as an object —
/// and never call it plainly supported.
#[test]
fn list_targets_marks_foreign_native_triples() {
    for triple in foreign_native_triples() {
        let tier = tier_of(triple);
        assert!(tier.starts_with("cross"), "{triple}: {tier}");
        assert!(!tier.contains("supported: builds and links"), "{triple}: {tier}");
        assert!(tier.contains("970"), "{triple}: {tier}");
    }
    // Both kinds exist from every host: the Linux gnu triples link
    // (zig carries their libc), the Darwin ones do not (no SDK here).
    for t in cross_triples() {
        assert!(t.contains("linux"), "{t} should not be a cross target");
    }
    for t in object_only_triples() {
        assert!(t.contains("darwin"), "{t} should link through zig");
    }
}

#[test]
fn an_unknown_triple_lists_the_ones_that_exist() {
    let dir = case_dir("target_model_unknown");
    let src = dir.join("t.hl");
    std::fs::write(&src, "fn main() { }\n").unwrap();

    // A near-miss: the right OS, the wrong ABI.
    let (_, stderr, code) = run(&[
        "build",
        src.to_str().unwrap(),
        "--target",
        "x86_64-pc-windows-gnu",
    ]);
    assert_ne!(code, 0);
    assert!(stderr.contains("unknown target"), "{stderr}");
    assert!(
        stderr.contains("x86_64-pc-windows-msvc"),
        "should suggest the real one:\n{stderr}"
    );
    assert!(
        stderr.contains("native"),
        "should mention the aliases:\n{stderr}"
    );
}

#[test]
fn the_existing_aliases_are_unchanged() {
    let dir = case_dir("target_model_native");
    let src = dir.join("t.hl");
    std::fs::write(&src, "fn main() { println(\"ok\"); }\n").unwrap();

    // `native` builds and runs, exactly as before the target model existed.
    let (_, stderr, code) = run(&["build", src.to_str().unwrap(), "--target", "native"]);
    assert_eq!(code, 0, "native build failed: {stderr}");
    let bin = src.with_extension("");
    assert!(bin.exists(), "no executable at {}", bin.display());
    let out = Command::new(&bin).output().expect("run built binary");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");

    // `wasm32` still names its output `.wasm`, which is now the target's
    // convention rather than a `with_extension` at one call site.
    let (_, stderr, code) = run(&["build", src.to_str().unwrap(), "--target", "wasm32"]);
    assert_eq!(code, 0, "wasm build failed: {stderr}");
    assert!(src.with_extension("wasm").exists(), "no .wasm output");
}
