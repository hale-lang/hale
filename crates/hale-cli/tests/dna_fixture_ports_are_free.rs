//! A DNA fixture never invents a port: it takes `dna::free_port(index)`.
//!
//! `dna::free_port` binds a candidate on the loopback before handing it
//! back, so a port another listener holds is stepped past. A fixture
//! that writes its own number instead — a literal in a server's
//! `port:`, a literal handed to a listen or a bind, or a port derived
//! from the pid and a salt — fails with a bare exit before its first
//! assertion the day anything else holds that number, and passes on
//! every machine where nothing does. Two model-adapter fixtures did
//! exactly that on main; this scan closes the class rather than the
//! instances, in the spirit of `harness_paths_are_unique.rs`.
//!
//! Scanned: every `*.hl` under a directory named `tests` anywhere in
//! `dna/` (`dna/tests`, `dna/api/**/tests`, `dna/host/tests`, ...).
//! A port below 1024 is a placeholder and never a bind for an
//! unprivileged process (`port: 1` in the default of a param that
//! `fn main` overrides), so it is not a violation. The only allowlist
//! is for parked fixtures, each with the reason.

use std::path::{Path, PathBuf};

/// Parked fixtures: not in the suite's run set, so a literal there
/// binds nothing today. Each entry goes when its fixture is unparked or
/// deleted, and the scan fails on an entry with nothing left to allow.
const PARKED: &[(&str, &str)] = &[
    (
        "dna/tests/parked/b1_team_test.hl",
        "parked: not run; the App's default issuer still names 47815 (its heads already take dna::free_port)",
    ),
    (
        "dna/tests/parked/principal_oidc_test.hl",
        "parked: not run; the App's default issuer still names 47811 (its head already takes dna::free_port)",
    ),
    (
        "dna/tests/parked/two_heads_test.hl",
        "parked: not run; its FakeIssuer and issuer URLs still name 47811 and 47813",
    ),
];

/// Directories the scan leaves alone. `dna/tests/onboarding/<repo>/` are
/// repositories of their own, shaped for a clone that has no `dna/` above it
/// to import `dna::free_port` from, so each owns its ports (a bound probe
/// from a fixed base, never a number derived from the pid).
const EXEMPT_DIRS: &[&str] = &["dna/tests/onboarding/"];

const PLACEHOLDER_BELOW: u64 = 1024;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A run of digits at the start of `s`, as a number, when it is the
/// whole token (nothing identifier-like or `.` follows).
fn leading_number(s: &str) -> Option<u64> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    match s[digits.len()..].chars().next() {
        Some(c) if is_ident(c) || c == '.' => None,
        _ => digits.parse().ok(),
    }
}

/// `port: <literal>` with the literal at or above the placeholder line.
fn literal_port_field(line: &str) -> bool {
    line.match_indices("port").any(|(i, _)| {
        let before_ok = !line[..i].chars().next_back().map(is_ident).unwrap_or(false);
        let rest = line[i + 4..].trim_start();
        before_ok
            && rest.strip_prefix(':').is_some_and(|after| {
                leading_number(after.trim_start()).is_some_and(|n| n >= PLACEHOLDER_BELOW)
            })
    })
}

/// A call whose name says listen or bind, with a bare numeric argument.
fn literal_listen_or_bind(line: &str) -> bool {
    line.match_indices('(').any(|(open, _)| {
        let name: String = line[..open]
            .chars()
            .rev()
            .take_while(|c| is_ident(*c) || *c == ':')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let last = name.rsplit("::").next().unwrap_or("");
        if !(last.contains("listen") || last.contains("bind")) {
            return false;
        }
        let args = match line[open + 1..].find(')') {
            Some(close) => &line[open + 1..open + 1 + close],
            None => &line[open + 1..],
        };
        args.split(',')
            .any(|a| leading_number(a.trim()).is_some_and(|n| n >= PLACEHOLDER_BELOW) && a.trim().chars().all(|c| c.is_ascii_digit()))
    })
}

/// A port derived from the pid: modulo arithmetic over `process::pid()`.
fn pid_arithmetic(line: &str) -> bool {
    line.contains("process::pid()") && line.contains('%')
}

fn violations(text: &str) -> Vec<(usize, &'static str, String)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let rule = if literal_port_field(line) {
            "a literal port in a `port:` field"
        } else if literal_listen_or_bind(line) {
            "a literal port handed to a listen or a bind"
        } else if pid_arithmetic(line) {
            "a port derived from the pid (pid-plus-salt arithmetic)"
        } else {
            continue;
        };
        out.push((n + 1, rule, line.trim().to_string()));
    }
    out
}

fn collect(dir: &Path, under_tests: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            let in_tests = under_tests || p.file_name().is_some_and(|n| n == "tests");
            collect(&p, in_tests, out);
        } else if under_tests && p.extension().is_some_and(|e| e == "hl") {
            out.push(p);
        }
    }
}

fn scanned_files() -> Vec<(String, String)> {
    let root = repo_root();
    let mut files = Vec::new();
    collect(&root.join("dna"), false, &mut files);
    files
        .into_iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let rel = p.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
            if EXEMPT_DIRS.iter().any(|d| rel.starts_with(d)) {
                return None;
            }
            Some((rel, text))
        })
        .collect()
}

#[test]
fn no_fixture_invents_a_port() {
    let parked: Vec<&str> = PARKED.iter().map(|(p, _)| *p).collect();
    let mut offenders = Vec::new();
    for (rel, text) in scanned_files() {
        if parked.contains(&rel.as_str()) {
            continue;
        }
        for (line, rule, src) in violations(&text) {
            offenders.push(format!("{rel}:{line}: {rule}: {src}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "a DNA fixture takes its port from `dna::free_port(index)` and hands it to the \
         client side; it never writes a number of its own ({} found):\n{}\n",
        offenders.len(),
        offenders.join("\n")
    );
}

#[test]
fn every_parked_entry_still_has_something_to_allow() {
    let files = scanned_files();
    for (path, why) in PARKED {
        let Some((_, text)) = files.iter().find(|(rel, _)| rel == path) else {
            panic!("{path} ({why}) is allowlisted but no longer exists: delete the entry");
        };
        assert!(
            !violations(text).is_empty(),
            "{path} ({why}) no longer binds a literal port: delete the entry"
        );
    }
}

#[test]
fn the_scan_is_not_vacuous() {
    let files = scanned_files();
    assert!(files.len() >= 50, "only {} fixture files scanned; the directory walk is broken", files.len());
    for must in ["dna/tests/oidc_head_test.hl", "dna/api/tests/fixture/main.hl", "dna/host/tests"] {
        assert!(files.iter().any(|(rel, _)| rel.starts_with(must)), "{must} is not scanned");
    }
}

#[test]
fn the_matchers_see_what_they_name() {
    let hit = |l: &str| !violations(l).is_empty();
    // the shapes the two adapter fixtures had
    assert!(hit("srv: std::http::Server = std::http::Server { port: 47392, handler: F { } };"));
    assert!(hit("        port: 47391,"));
    // listen and bind with a literal
    assert!(hit("let fd = std::io::tcp::__listen_socket(\"127.0.0.1\", 45001);"));
    assert!(hit("std::io::tcp::bind(\"127.0.0.1\", 45001);"));
    // the pid-plus-salt arithmetic the api fixture had
    assert!(hit("let port = 30000 + (std::process::pid() + salt + i) % 25000;"));
    // what is fine: a variable, a placeholder, a param default, a compare, a comment
    assert!(!hit("std::http::Server { host: \"127.0.0.1\", port: port, handler: h }"));
    assert!(!hit("server: std::http::Server = std::http::Server { port: 1, handler: h };"));
    assert!(!hit("port: Int = 8080;"));
    assert!(!hit("assert(d.port == 5480, \"a full DSN\");"));
    assert!(!hit("let fd = std::io::tcp::__listen_socket(\"127.0.0.1\", port);"));
    assert!(!hit("let scratch = \"/tmp/dna-\" + to_string(std::process::pid());"));
    assert!(!hit("// port: 47392 was the old fixed port"));
    assert!(!hit("let report = \"support: 47392\";"));
}
