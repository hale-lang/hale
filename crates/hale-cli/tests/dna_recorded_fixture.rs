//! GH #583 M4 — the recorded fixture: a workspace of three services
//! under a plan with claims across them (`dna/acceptance/trio`), whose
//! organization is driven by scripted asks end to end — a change
//! deployed to two nodes, the organization grown by a supervisor under
//! pressure, a cross-service change denied by the fleet's law — every
//! model call answered from the checked-in tape
//! (`dna/acceptance/trio.fixture/tape`, its catalog beside it), keyless.
//! This is the acceptance for every later change to the organization.
//! GH #583 K4 — the learning scenario: the worker observes a recurring
//! condition and raises a concern, its own event on the nerves; the
//! heart lands it and the spine puts it in the record; the
//! organization proposes it; the Board ratifies the exact digest; the
//! next change to the trio is made with it in hand, and the editor's
//! evidence names the package.
//!
//! Re-record (a key in the environment, the tape rewritten):
//!   HALE_DNA_TAPE=record cargo test --release -p hale-cli --test dna_records dna_recorded_fixture::

#[path = "support/vault.rs"]
mod vault;
#[path = "support/trace.rs"]
mod trace;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

type Row = (u64, String, String, String);

fn git(args: &[&str], cwd: &Path) -> String {
    let _s = trace::Span::new("git", args[0].to_string());
    let out = Command::new("git").args(["-c", "user.name=riley", "-c", "user.email=r@l"]).args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn journal(app: &Path) -> Vec<Row> {
    let _s = trace::Span::new("git", "show refs/dna/journal:journal.jsonl");
    let out = Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            (v["seq"].as_u64().unwrap_or(0), s("kind"), s("entity"), s("body"))
        })
        .collect()
}

fn wait_row(app: &Path, what: &str, secs: u64, pred: impl Fn(&Row) -> bool) -> bool {
    trace::wait_until(what.to_string(), Duration::from_secs(secs), Duration::from_millis(300), || journal(app).iter().any(&pred))
}

/// The same, for a stage that is several rows rather than one, so a
/// wait for the last of them is not a wait for a different event.
fn wait_rows(app: &Path, what: &str, secs: u64, n: usize, pred: impl Fn(&Row) -> bool) -> bool {
    trace::wait_until(what.to_string(), Duration::from_secs(secs), Duration::from_millis(300), || journal(app).iter().filter(|r| pred(r)).count() >= n)
}

/// The process the organization is running as, as the host wrote it.
/// A stage that stalls says whether it is still the one that started,
/// so "the organism restarted" is answered where it is asked (GH #748).
fn org_pid(app: &Path) -> String {
    std::fs::read_to_string(app.join(".hale/dna/org.pid")).unwrap_or_default().trim().to_string()
}

/// Which occurrence each of a source's concerns was answered as, in the
/// order the record holds them.
fn concern_occurrences(app: &Path, source: &str) -> Vec<i64> {
    journal(app)
        .iter()
        .filter(|(_, k, e, _)| k == "concern.raised" && e == source)
        .map(|(_, _, _, b)| serde_json::from_str::<serde_json::Value>(b).ok().and_then(|v| v["occurrence"].as_i64()).unwrap_or(0))
        .collect()
}

fn dump(app: &Path) -> String {
    journal(app).iter().map(|(q, k, e, b)| format!("{q} {k} {e} {}", b.chars().take(160).collect::<String>())).collect::<Vec<_>>().join("\n")
}

// ---- the differential row harness (C3, the assembly's split) --------
//
// The record this fixture's organization writes, row for row, is checked
// in beside the tape (`dna/acceptance/trio.fixture/rows.jsonl`) and every
// replay must write the same. It is what a refactor of the organization
// proves it kept: the same rows, of the same kinds, for the same
// entities, saying the same things. The fixture's children run as one
// person (`USER=riley`) on every machine. Two things vary from run to run
// and are normalized away: values a run draws (commit and build hashes,
// digests over them, times, pids, the scratch root and the machine's
// name, the toolchain's version, ids minted from the clock, row numbers
// and the heads read at them, and the size of the model diff, which
// carries the core's own shape), and the order rows of
// different writers interleave in (the nodes, the heart and the
// organization append concurrently), so the comparison is of the
// multiset. Re-record, on the organization as it stands:
//   HALE_DNA_TRIO_ROWS=record cargo test --release -p hale-cli --test dna_records dna_recorded_fixture::

/// `s` with every value a run draws replaced by the name of its class.
/// `minted` is the set of ids this run minted from its clock (see
/// `minted_ids`): each is replaced whole, whatever its length or letters.
fn normalize_row_text(s: &str, root: &str, minted: &std::collections::HashSet<String>) -> String {
    let s = s.replace(root, "<root>").replace(&format!("@{}:", this_host()), "@<host>:");
    let s = string_after(&s, &["\"toolchain\":\"", "\"toolchain\": \""]);
    let s = number_after(&s, &["\"revision\": ", "\"head\": ", "\"pid\": ", "\"ratified_at\": ", "\"watermark\": ", "\"hat_watermark\": ", "at row "]);
    let mut out = String::with_capacity(s.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        let class = if minted.contains(token.as_str()) { Some(format!("{}<clock>", &token[..1])) } else { token_class(token) };
        out.push_str(&class.unwrap_or_else(|| token.clone()));
        token.clear();
    };
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// The class of a token a run draws, or None for one it does not.
fn token_class(t: &str) -> Option<String> {
    let hex = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let digits = t.bytes().all(|b| b.is_ascii_digit());
    if digits && t.starts_with('1') && t.len() == 19 {
        return Some("<ns>".into());
    }
    if digits && t.starts_with('1') && t.len() == 10 {
        return Some("<time>".into());
    }
    if hex(t) {
        return match t.len() {
            64 => Some("<digest>".into()),
            40 => Some("<sha>".into()),
            16 => Some("<hex16>".into()),
            12 => Some("<hex12>".into()),
            _ => None,
        };
    }
    None
}

/// `s` with the number after each of `prefixes` (quoted or not) replaced
/// by `<n>`: a row number, or the head read at one.
fn number_after(s: &str, prefixes: &[&str]) -> String {
    let mut s = s.to_string();
    for p in prefixes {
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        while let Some(at) = rest.find(p) {
            out.push_str(&rest[..at + p.len()]);
            rest = &rest[at + p.len()..];
            let quoted = rest.starts_with('"');
            let body = if quoted { &rest[1..] } else { rest };
            let n = body.bytes().take_while(|b| b.is_ascii_digit()).count();
            // the whole value, not the digits a hash happens to start with
            let whole = match body.as_bytes().get(n) {
                None => true,
                Some(&c) => if quoted { c == b'"' } else { !c.is_ascii_alphanumeric() },
            };
            if n > 0 && whole {
                out.push_str(if quoted { "\"<n>" } else { "<n>" });
                rest = &body[n..];
            }
        }
        out.push_str(rest);
        s = out;
    }
    s
}

/// This machine's name, as the organization reads it (`hostname`): part
/// of a body's holder, `<user>@<host>:<place>`.
fn this_host() -> String {
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        Command::new("hostname").output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|| "localhost".into())
    })
    .clone()
}

/// `s` with the quoted string after each of `prefixes` replaced by `<v>`:
/// the toolchain's version, which a release moves and the organization
/// does not choose.
fn string_after(s: &str, prefixes: &[&str]) -> String {
    let mut s = s.to_string();
    for p in prefixes {
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        while let Some(at) = rest.find(p) {
            out.push_str(&rest[..at + p.len()]);
            rest = &rest[at + p.len()..];
            let end = rest.find('"').unwrap_or(rest.len());
            out.push_str("<v>");
            rest = &rest[end..];
        }
        out.push_str(rest);
        s = out;
    }
    s
}

/// The ids this run minted from its clock: an intent's (`i` + hex of the
/// machine's monotonic milliseconds, the entity of its `intent.*` rows)
/// and a request's (a row's top-level `request`, `r` or `i` + hex). Their
/// length and letters follow the machine's uptime, so no shape names them
/// all: an id of five hex letters and no digit is as much one as any. They
/// are read from the rows themselves and replaced exactly.
fn minted_ids(rows: &[Row]) -> std::collections::HashSet<String> {
    let clock_id = |v: &str| {
        let mut chars = v.chars();
        matches!(chars.next(), Some('i') | Some('r')) && v.len() > 1 && chars.all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    };
    let mut out = std::collections::HashSet::new();
    for (_, k, e, b) in rows {
        if k.starts_with("intent.") && clock_id(e) {
            out.insert(e.clone());
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(b) {
            if let Some(r) = v.get("request").and_then(|r| r.as_str()) {
                if clock_id(r) {
                    out.insert(r.to_string());
                }
            }
        }
    }
    out
}

/// The record as the harness compares it: one line per row, normalized,
/// sorted.
fn normalized_rows(rows: &[Row], root: &str) -> Vec<String> {
    let minted = minted_ids(rows);
    // a body's holder names the root as `pwd -P` resolves it: where the
    // temporary directory is a symlink (macOS's /var), that is another path
    let canonical = std::fs::canonicalize(root).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| root.to_string());
    let root_text = |t: &str| normalize_row_text(&t.replace(&canonical, root), root, &minted);
    let mut out: Vec<String> = rows
        .iter()
        .map(|(_, k, e, b)| {
            // An evidence step's output size: the model diff's is the
            // organization's model, which carries the shape of the core it
            // imports (a refactor of the core moves it without changing a
            // thing the organization does), and the others' name paths under
            // the scratch root, whose pid is as wide as the machine makes it.
            // Their digests are normalized with every other; their sizes go
            // the same way.
            let b = if k.starts_with("evidence.") { number_after(b, &["\"bytes\": "]) } else { b.clone() };
            serde_json::json!([k, root_text(e), root_text(&b)]).to_string()
        })
        .collect();
    out.sort();
    out
}

/// Assert this replay wrote the rows recorded beside the tape (or, under
/// `HALE_DNA_TRIO_ROWS=record`, record them).
fn same_rows_as_recorded(rows: &[Row], root: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dna/acceptance/trio.fixture/rows.jsonl");
    let now = normalized_rows(rows, root);
    if std::env::var("HALE_DNA_TRIO_ROWS").as_deref() == Ok("record") {
        std::fs::write(&path, now.join("\n") + "\n").unwrap();
        return;
    }
    let recorded: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the rows recorded beside the tape, {}: {e} (HALE_DNA_TRIO_ROWS=record records them)", path.display()))
        .lines()
        .map(String::from)
        .collect();
    let mut left: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for r in &recorded {
        *left.entry(r.as_str()).or_default() += 1;
    }
    for r in &now {
        *left.entry(r.as_str()).or_default() -= 1;
    }
    let mut missing: Vec<String> = left.iter().filter(|(_, n)| **n > 0).map(|(r, n)| format!("  recorded, not written ({n}x): {r}")).collect();
    let mut extra: Vec<String> = left.iter().filter(|(_, n)| **n < 0).map(|(r, n)| format!("  written, not recorded ({}x): {r}", -n)).collect();
    missing.sort();
    extra.sort();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "the organization's rows differ from the ones recorded beside the tape ({} recorded, {} written):\n{}\n{}",
        recorded.len(),
        now.len(),
        missing.join("\n"),
        extra.join("\n")
    );
}

#[test]
fn the_row_normalization_keeps_what_a_row_says_and_drops_what_a_run_draws() {
    let root = "/tmp/hale_dna_trio_42";
    let none = std::collections::HashSet::new();
    let minted: std::collections::HashSet<String> = ["r43c9e4e", "iabcde", "i4f2"].iter().map(|s| s.to_string()).collect();
    let a = normalize_row_text(
        "{\"base\": \"3e3bf378db66fc11b67910a093348d6e338bbc53\", \"at\": 1790620494, \"id\": \"backlog-1790620491573709736-2\", \"head\": \"152\", \"revision\": 175, \"request\": \"r43c9e4e\", \"path\": \"/tmp/hale_dna_trio_42/trio\", \"what\": \"mail backlog\"}",
        root,
        &minted,
    );
    assert_eq!(a, "{\"base\": \"<sha>\", \"at\": <time>, \"id\": \"backlog-<ns>-2\", \"head\": \"<n>\", \"revision\": <n>, \"request\": \"r<clock>\", \"path\": \"<root>/trio\", \"what\": \"mail backlog\"}");
    // an id the run minted is its class whatever its shape: five hex letters
    // and no digit (a machine up about twelve minutes), or four characters
    // (one up about a minute), and wherever it appears
    assert_eq!(normalize_row_text("plan/iabcde claimed; iabcde/plan called; i4f2: ask-edit@1", root, &minted), "plan/i<clock> claimed; i<clock>/plan called; i<clock>: ask-edit@1");
    // and a word that merely looks like one is not an id the run minted
    assert_eq!(normalize_row_text("ice, iface, reface, i2", root, &minted), "ice, iface, reface, i2");
    // the machine: its name in a body's holder, the toolchain's version
    let holder = format!("{{\"holder\": \"riley@{}:/tmp/hale_dna_trio_42/trio\", \"toolchain\":\"0.21.0\"}}", this_host());
    assert_eq!(normalize_row_text(&holder, root, &none), "{\"holder\": \"riley@<host>:<root>/trio\", \"toolchain\":\"<v>\"}");
    // a hash under a number's key is a hash, whatever it starts with
    assert_eq!(normalize_row_text("{\"revision\": \"8a45cfb3ca423ffd138a1c2c031d921b17794437\", \"head\": \"55d4703af8442f0aefd734426a6a506bb1234567\"}", root, &none), "{\"revision\": \"<sha>\", \"head\": \"<sha>\"}");
    // what a row says stays: a mutation id, a count, a word, a small number
    assert_eq!(normalize_row_text("m2 applied 3 of 4 to gateway-1, review:m3, t9, i2", root, &none), "m2 applied 3 of 4 to gateway-1, review:m3, t9, i2");
}

#[test]
fn the_ids_a_run_minted_are_read_from_its_rows() {
    let row = |k: &str, e: &str, b: &str| (0u64, k.to_string(), e.to_string(), b.to_string());
    let rows = vec![
        row("intent.requested", "iabcde", "{\"outcome\": \"x\"}"),
        row("intent.offered", "i4f2", "document the Gateway (from riley)"),
        row("pressure.requested", "worker", "{\"request\": \"r43c9e4e\"}"),
        row("concern.raised", "org/trio/worker", "{\"request\": \"trio/concern.raised/backlog-1-2\"}"),
        row("task.born", "t5", "iabcde: ask-edit@1 (workflow)"),
        row("mutation.requested", "m1", "{\"task_id\": \"t5\"}"),
    ];
    let mut minted: Vec<String> = minted_ids(&rows).into_iter().collect();
    minted.sort();
    assert_eq!(minted, vec!["i4f2", "iabcde", "r43c9e4e"]);
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), &dst).unwrap();
        }
    }
}

struct Fixture {
    d: PathBuf,
    app: PathBuf,
    bare: PathBuf,
    edges: Vec<PathBuf>,
    procs: Vec<std::process::Child>,
    tape: PathBuf,
    mode: String,
    spine: String,     // memory under the record's spine role (K4; GH #985)
    nats_spine: String, // the nerves' spine URL (GH #986)
    nats_org: String,   // the organization's token on the nerves (GH #986)
    nats_app: String,   // an application's server, which a node hands its instances (GH #986)
    nats_app_user: String, // the application's own user (GH #989)
    nats_app_vault: String, // the vault name of the application's credential (GH #989; never the password)
}

impl Fixture {
    fn cmd(&self, args: &[&str], cwd: &Path) -> Command {
        let mut c = vault::hale();
        c.args(args)
            .current_dir(cwd)
            .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
            .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
            // who asks, and whose body holds the organization, is part of
            // what the record says: the same person on every machine
            .env("USER", "riley")
            .env("LOGNAME", "riley")
            .env("HALE_DNA_TAPE", &self.mode)
            .env("HALE_DNA_TAPE_DIR", &self.tape)
            .env("HALE_DNA_MEMORY_DSN_SPINE", &self.spine)
            .env("HALE_DNA_NATS_URL_SPINE", &self.nats_spine)
            .env("HALE_DNA_NATS_ORG", &self.nats_org)
            .env("HALE_DNA_NATS_URL_APP", &self.nats_app)
            .env("HALE_DNA_NATS_USER_APP", &self.nats_app_user)
            .env("HALE_DNA_NATS_VAULT_APP", &self.nats_app_vault);
        c
    }
    fn hale(&self, args: &[&str], cwd: &Path) -> (bool, String) {
        let _s = trace::Span::new("hale", args.join(" "));
        let out = self.cmd(args, cwd).output().expect("hale");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }
    fn spawn(&mut self, args: &[&str], cwd: &Path) {
        let _s = trace::Span::new("spawn", args.join(" "));
        let log = std::fs::File::create(self.d.join(format!("{}.stderr", args.iter().take(2).map(|a| a.replace('/', "_")).collect::<Vec<_>>().join("-")))).unwrap();
        let c = self.cmd(args, cwd).stdout(Stdio::null()).stderr(log).spawn().expect("spawn");
        self.procs.push(c);
    }
    fn logs(&self) -> String {
        let mut out = String::new();
        if let Ok(rd) = std::fs::read_dir(&self.d) {
            for f in rd.flatten() {
                if f.path().extension().map(|x| x == "stderr").unwrap_or(false) {
                    out.push_str(&format!("--- {}\n{}\n", f.file_name().to_string_lossy(), std::fs::read_to_string(f.path()).unwrap_or_default()));
                }
            }
        }
        if let Ok(log) = std::fs::read_to_string(self.app.join(".hale/dna/org.log")) {
            out.push_str(&format!("--- org.log\n{}\n", log.chars().rev().take(6000).collect::<String>().chars().rev().collect::<String>()));
        }
        out
    }
    fn stop(&mut self) {
        for p in self.procs.iter_mut() {
            let _ = p.kill();
            let _ = p.wait();
        }
        trace::sleep("after kill, before reaping the pid files", Duration::from_millis(300));
        for f in ["org.pid", "app.pid"] {
            if let Ok(pid) = std::fs::read_to_string(self.app.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
        for (i, e) in self.edges.iter().enumerate() {
            let nd = e.join(".hale/node").join(format!("edge-{}", i + 1));
            if let Ok(rd) = std::fs::read_dir(&nd) {
                for f in rd.flatten() {
                    if f.path().extension().map(|x| x == "pid").unwrap_or(false) {
                        if let Ok(pid) = std::fs::read_to_string(f.path()) {
                            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
                        }
                    }
                }
            }
        }
    }
    fn fail(&mut self, why: &str) -> ! {
        let dump = dump(&self.app);
        let logs = self.logs();
        self.stop();
        panic!("{why}\n--- record\n{dump}\n{logs}");
    }
}

/// The trio with its DNA and the fixture's catalog, an origin, two node
/// clones, the organization and both nodes up, the base deployed.
fn bring_up() -> Fixture {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mode = std::env::var("HALE_DNA_TAPE").unwrap_or_else(|_| "replay".to_string());
    let tape = repo.join("dna/acceptance/trio.fixture/tape");
    if mode == "record" {
        let _ = std::fs::remove_dir_all(&tape);
    }
    std::fs::create_dir_all(&tape).unwrap();
    let d = std::env::temp_dir().join(format!("hale_dna_trio_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    // the workspace name is part of what the organization is generated
    // from (its project's name), so it is fixed: the tape depends on it
    let app = d.join("trio");
    copy_dir(&repo.join("dna/acceptance/trio"), &app);
    let mut f = Fixture { d: d.clone(), app: app.clone(), bare: d.join("origin.git"), edges: vec![d.join("edge-1"), d.join("edge-2")], procs: vec![], tape, mode, spine: String::new(), nats_spine: String::new(), nats_org: String::new(), nats_app: String::new(), nats_app_user: String::new(), nats_app_vault: String::new() };
    git(&["init", "-q", "-b", "main"], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the trio and its fleet"], &app);
    let (ok, out) = f.hale(&["dna", "init", ".", "--no-library"], &app);
    assert!(ok, "init: {out}");
    std::fs::write(app.join("dna/org/models.hl"), std::fs::read_to_string(repo.join("dna/acceptance/trio.fixture/catalog.hl")).unwrap()).unwrap();
    let (ok, out) = f.hale(&["check", "--matrix", "."], &app);
    assert!(ok, "the trio and its organization check: {out}");
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the organization"], &app);
    git(&["init", "-q", "--bare", "-b", "main", &f.bare.to_string_lossy()], &d);
    git(&["remote", "add", "origin", &f.bare.to_string_lossy()], &app);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &app);
    for e in &f.edges {
        git(&["clone", "-q", &f.bare.to_string_lossy(), &e.to_string_lossy()], &d);
    }
    // memory for the record (K4): the owner migrates, the organization
    // runs under the spine's role, then the nodes
    let (ok, out) = f.hale(&["dna", "memory", "migrate"], &app);
    if !ok {
        f.fail(&format!("memory migrate: {out}"));
    }
    f.spine = out.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_SPINE=")).unwrap_or("").to_string();
    // the nerves (GH #986): a verdict, an intent or a report reaches the
    // organization only over them — the owner migrates the stream, the
    // organization and the host run under the spine's role and the
    // organization's token
    let (ok, out) = f.hale(&["dna", "nerves", "migrate"], &app);
    if !ok {
        f.fail(&format!("nerves migrate: {out}"));
    }
    f.nats_spine = out.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_URL_SPINE=")).unwrap_or("").to_string();
    f.nats_org = out.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_ORG=")).unwrap_or("").to_string();
    f.nats_app = out.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_URL_APP=")).unwrap_or("").to_string();
    f.nats_app_user = out.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_USER_APP=")).unwrap_or("").to_string();
    f.nats_app_vault = out.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_VAULT_APP=")).unwrap_or("").to_string();
    f.spawn(&["dna", "run", ".", "--no-iris", "--observe", "4"], &app);
    let edges = f.edges.clone();
    for (i, e) in edges.iter().enumerate() {
        f.spawn(&["node", &format!("edge-{}", i + 1), "--repo", &e.to_string_lossy(), "--tick", "300"], &d);
    }
    let host_log = f.d.join("dna-run.stderr");
    let nerves_up = || std::fs::read_to_string(&host_log).unwrap_or_default().contains("the organization reads its facts from the nerves");
    trace::wait_until("dna run: the organization reads its facts from the nerves", Duration::from_secs(180), Duration::from_millis(200), nerves_up);
    if !nerves_up() {
        f.fail("the organization never read its facts from the nerves");
    }
    let (ok, out) = f.hale(&["dna", "deploy", "HEAD"], &app);
    if !ok || !out.contains("touching gateway-0 gateway-1 api-0 worker-0") {
        f.fail(&format!("deploy: {out}"));
    }
    let base = git(&["rev-parse", "HEAD"], &app);
    for id in ["gateway-0", "gateway-1", "api-0", "worker-0"] {
        if !wait_row(&app, &format!("instance.up {id} at the base"), 180, |(_, k, e, b)| k == "instance.up" && e == id && b.contains(&base)) {
            f.fail(&format!("{id} did not come up at the base"));
        }
    }
    f
}

#[test]
fn three_services_two_nodes_and_a_grown_organization_replay_from_the_tape() {
    let _t = trace::test("dna_recorded_fixture");
    if std::env::var("HALE_DNA_MEMORY_DSN_OWNER").map(|d| d.is_empty()).unwrap_or(true) {
        eprintln!("dna_recorded_fixture: no HALE_DNA_MEMORY_DSN_OWNER; the organization's knowledge is memory's, so nothing was exercised");
        return;
    }
    if std::env::var("HALE_DNA_NATS_URL_OWNER").map(|d| d.is_empty()).unwrap_or(true) {
        eprintln!("dna_recorded_fixture: no HALE_DNA_NATS_URL_OWNER; a verdict cannot reach the organization, so nothing was exercised");
        return;
    }
    let mut f = bring_up();
    let app = f.app.clone();

    // ---- 0. the learning scenario (K4): the worker on edge-2 observes
    //         its mail backlog and raises a concern, three times, as
    //         its own event on the nerves; the heart lands each and the
    //         spine puts it into the record; the host relays; the
    //         organization proposes it as knowledge for org/trio; the
    //         Board ratifies the exact digest; the service tails it
    //
    //         Each stage waits for ITS OWN condition (GH #795's rule):
    //         the spine putting three concerns into the record, the
    //         organization answering each, and the proposal the third
    //         earns. One wait for the last of them reported every
    //         earlier stall as "no proposal", 240 seconds later.
    let org_at_boot = org_pid(&app);
    if !wait_rows(&app, "concern.requested org/trio/worker x3", 240, 3, |(_, k, e, _)| k == "concern.requested" && e == "org/trio/worker") {
        let now = org_pid(&app);
        f.fail(&format!("the worker's three concerns did not reach the record (the organization: pid {org_at_boot} at boot, {now} now)"));
    }
    if !wait_rows(&app, "concern.raised org/trio/worker x3", 120, 3, |(_, k, e, _)| k == "concern.raised" && e == "org/trio/worker") {
        let now = org_pid(&app);
        f.fail(&format!("the organization did not answer every one of the worker's concerns (pid {org_at_boot} at boot, {now} now)"));
    }
    // Which occurrence each was answered as is the count in the
    // organization's own record: 1, 2, 3. A count that starts again is
    // an organization that read its record short (GH #748) — the
    // proposal below would then never come, and this is where that is
    // reported, by name, instead of two minutes later as a timeout.
    let occurrences = concern_occurrences(&app, "org/trio/worker");
    if occurrences.len() < 3 || occurrences[0..3] != [1, 2, 3] {
        let now = org_pid(&app);
        f.fail(&format!("the organization counted the worker's concerns {occurrences:?}, not 1, 2, 3 (pid {org_at_boot} at boot, {now} now)"));
    }
    if !wait_row(&app, "concern.proposed org/trio/worker", 120, |(_, k, e, _)| k == "concern.proposed" && e == "org/trio/worker") {
        let now = org_pid(&app);
        f.fail(&format!("the worker's concern did not become a proposal (pid {org_at_boot} at boot, {now} now)"));
    }
    let rows = journal(&app);
    let requested = rows.iter().filter(|(_, k, e, _)| k == "concern.requested" && e == "org/trio/worker").count();
    let raised = rows.iter().filter(|(_, k, e, _)| k == "concern.raised" && e == "org/trio/worker").count();
    assert!(requested >= 3 && raised >= 3, "three concerns travelled from the worker's own events into the record and onto the nerves: requested {requested}, raised {raised}");
    // GH #986: no socket of the node's carried it; each is the reading of
    // the worker's own event, named as its request
    assert!(rows.iter().any(|(_, k, e, b)| k == "concern.requested" && e == "org/trio/worker" && b.contains("\"request\": \"trio/concern.raised/backlog-") && b.contains("\"app\": \"trio\"")), "the concern is the worker's own event, landed by the heart");
    assert!(rows.iter().any(|(_, k, e, _)| k == "reading.recorded" && e.starts_with("trio/concern.raised/backlog-")), "and its reading is in the record");
    let kprop = rows.iter().find(|(_, k, _, b)| k == "knowledge.proposed" && b.contains("\"author\": \"org/trio/worker\"")).expect("the proposal");
    let kdigest = kprop.2.clone();
    assert!(kprop.3.contains("\"class\": \"concern\"") && kprop.3.contains("\"target\": \"org/trio\""), "a concern by the tower rule, bound to the application: {}", kprop.3);
    let (ok, reviews) = f.hale(&["dna", "review"], &app);
    let kreview = format!("k:{}", &kdigest[7..19]);
    assert!(ok && reviews.contains(&kreview) && reviews.contains("mail backlog behind fulfilment") && reviews.contains("needs board"), "the Board's queue has the concern:\n{reviews}");
    let (ok, out) = f.hale(&["dna", "review", &kreview, "approve", "--as", "riley", "--authority", "board", "--comment", "true, and worth knowing"], &app);
    if !ok {
        f.fail(&format!("ratify: {out}"));
    }
    if !wait_row(&app, "knowledge.ratified", 60, |(_, k, e, _)| k == "knowledge.ratified" && *e == kdigest) {
        f.fail("the concern was not ratified");
    }

    // ---- 1. a change to the gateway, deployed to both nodes — made with
    //         the ratified concern in hand: the organization consults
    //         the service for org/trio, folds it into the editor's
    //         objective, and the editor's evidence names the package
    let (ok, out) = f.hale(&["dna", "task", "create", "document", "the", "Gateway", "locus", "in", "main.hl", "with", "a", "doc", "comment", "saying", "what", "it", "takes", "and", "where", "it", "hands", "it"], &app);
    if !ok {
        f.fail(&format!("ask: {out}"));
    }
    if !wait_row(&app, "review.requested review:m1", 240, |(_, k, e, _)| k == "review.requested" && e == "review:m1") {
        f.fail("m1 did not reach its Review");
    }
    let (ok, out) = f.hale(&["dna", "review", "m1", "approve", "--as", "riley", "--comment", "documented"], &app);
    if !ok {
        f.fail(&format!("approve m1: {out}"));
    }
    if !wait_row(&app, "mutation.retained m1", 240, |(_, k, e, _)| k == "mutation.retained" && e == "m1") {
        f.fail("m1 was not retained");
    }
    let rows = journal(&app);
    // knowledge changed later work: the consult, the package on the evidence
    let consulted = rows.iter().find(|(_, k, e, _)| k == "knowledge.consulted" && e == "m1").unwrap_or_else(|| panic!("m1 consulted the service:\n{}", dump(&app)));
    assert!(consulted.3.contains("\"target\": \"org/trio\"") && consulted.3.contains("\"included_n\": 1") && consulted.3.contains(&format!("\"included\": \"{kdigest}\"")), "the package for the trio carries the ratified concern: {}", consulted.3);
    let ev = rows.iter().find(|(_, k, e, b)| k == "model.called" && e == "m1/a0" && b.contains("\"knowledge_bindings\": \"package:")).unwrap_or_else(|| panic!("the editor's evidence names the package:\n{}", dump(&app)));
    assert!(ev.3.contains(&kdigest), "and the concern's digest: {}", ev.3);
    let cand1 = rows.iter().find(|(_, k, e, _)| k == "mutation.candidate" && e == "m1").map(|r| r.3.clone()).unwrap_or_default();
    let deploy = rows.iter().find(|(_, k, e, b)| k == "fleet.deploy" && e == "m1" && b.contains("\"reason\": \"apply\"")).cloned();
    let Some(deploy) = deploy else { f.fail("no fleet.deploy for m1") };
    let db: serde_json::Value = serde_json::from_str(&deploy.3).unwrap();
    assert_eq!(db["touched"], serde_json::json!(["gateway-0", "gateway-1"]), "a change to the gateway touches its two instances, one per node: {}", deploy.3);
    // a node expresses the whole revision it checks out, so every
    // instance comes up at the candidate; the two the change touched
    // are the ones the window is judged over, one per node
    let ups: Vec<&Row> = rows.iter().filter(|(q, k, e, b)| *q > deploy.0 && k == "instance.up" && e.starts_with("gateway-") && b.contains(&cand1)).collect();
    assert_eq!(ups.len(), 2, "both gateway instances came up at the candidate:\n{}", dump(&app));
    let node_of = |b: &str| serde_json::from_str::<serde_json::Value>(b).unwrap()["node"].as_str().unwrap().to_string();
    assert!(ups.iter().any(|r| node_of(&r.3) == "edge-1") && ups.iter().any(|r| node_of(&r.3) == "edge-2"), "one per node");
    let observed = rows.iter().find(|(_, k, e, _)| k == "expression.observed" && e == "m1").expect("observed");
    assert!(observed.3.contains("healthy") && observed.3.contains("2 instance(s) up"), "the window was judged over the two touched instances: {}", observed.3);
    let live = git(&["show", "HEAD:main.hl"], &app);
    assert!(live != std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dna/acceptance/trio/main.hl")).unwrap(), "the gateway changed");

    // ---- 2. pressure from the worker grows the organization: a supervisor
    let org_pid_before = std::fs::read_to_string(app.join(".hale/dna/org.pid")).unwrap_or_default();
    for _ in 0..3 {
        let (ok, out) = f.hale(&["dna", "pressure", "raise", "worker", "mail", "backlog", "behind", "fulfilment"], &app);
        if !ok {
            f.fail(&format!("pressure: {out}"));
        }
        trace::sleep("between pressure signals", Duration::from_millis(400));
    }
    if !wait_row(&app, "review.requested review:m2", 300, |(_, k, e, _)| k == "review.requested" && e == "review:m2") {
        f.fail("the organization's growth did not reach its Review");
    }
    let (ok, board) = f.hale(&["dna", "board"], &app);
    assert!(ok && board.contains("m2"), "the Board's queue lists the growth: {board}");
    let (ok, out) = f.hale(&["dna", "review", "m2", "approve", "--as", "riley", "--comment", "grow it"], &app);
    if !ok {
        f.fail(&format!("approve m2: {out}"));
    }
    if !wait_row(&app, "mutation.retained m2", 300, |(_, k, e, _)| k == "mutation.retained" && e == "m2") {
        f.fail("m2 was not retained");
    }
    let rows = journal(&app);
    let m2 = rows.iter().find(|(_, k, e, _)| k == "review.requested" && e == "review:m2").unwrap();
    assert!(m2.3.contains("\"change_class\": \"organization\"") && m2.3.contains("\"seed\": \"dna/org\""), "{}", m2.3);
    let org_main = git(&["show", "HEAD:dna/org/main.hl"], &app);
    assert!(org_main.matches("dna::Leader {").count() >= 2, "a new position in the organization's main:\n{org_main}");
    assert!(org_main.contains("self.core.request_tick(") && !org_main.contains("self.core.tick("), "growing the organization preserves its queued cadence:\n{org_main}");
    let org_pid_after = std::fs::read_to_string(app.join(".hale/dna/org.pid")).unwrap_or_default();
    assert!(!org_pid_before.is_empty() && org_pid_before != org_pid_after, "the organization was restarted with its new position");
    assert!(!rows.iter().any(|(_, k, e, _)| k == "mutation.failed" && e == "m1"), "a retained Mutation whose worktree is gone is not 'in flight' to the restarted organization:\n{}", dump(&app));

    // ---- 3. a change to one service that breaks the fleet's law is
    //         denied, though the service itself still checks: the gateway
    //         renames the subject the api is routed on
    let (ok, out) = f.hale(&["dna", "task", "create", "in", "main.hl", "change", "the", "Orders", "topic's", "subject", "from", "\"trio.orders\"", "to", "\"trio.orders.v2\"", "and", "nothing", "else"], &app);
    if !ok {
        f.fail(&format!("ask: {out}"));
    }
    if !wait_row(&app, "mutation.deny m3", 240, |(_, k, e, b)| k == "mutation.deny" && e == "m3" && b.contains("breaks the fleet")) {
        f.fail("m3 was not denied by the fleet's law");
    }
    let rows = journal(&app);
    let deny = rows.iter().find(|(_, k, e, _)| k == "mutation.deny" && e == "m3").unwrap();
    assert!(deny.3.contains("check=0") && deny.3.contains("fleet=1"), "the service checks, the fleet does not: {}", deny.3);
    assert!(!rows.iter().any(|(_, k, e, _)| k == "review.requested" && e == "review:m3"), "a denied candidate is never a Review");

    // ---- the tape answered everything. A model call's evidence is
    // journaled by the assembly's bus handler, so it lands AFTER the
    // mutation row the test waited on; wait for the last attempt's
    // rows and re-read, rather than judging an older snapshot.
    let calls_landed = wait_row(&app, "model.called m3/a0", 60, |(_, k, e, _)| k == "model.called" && e == "m3/a0");
    // the run's last facts, before the snapshot the harness compares: both
    // of the attempt's calls (each lands from its own bus delivery) and the
    // denied change's workflow settled
    let _ = wait_rows(&app, "model.called m3/a0 x2", 60, 2, |(_, k, e, _)| k == "model.called" && e == "m3/a0");
    let _ = wait_row(&app, "workflow.settled t9", 60, |(_, k, e, _)| k == "workflow.settled" && e == "t9");
    let (ok, status) = f.hale(&["dna", "status"], &app);
    f.stop();
    assert!(ok, "{status}");
    assert!(calls_landed, "the last attempt's evidence reached the record:\n{}", dump(&app));
    let rows = journal(&app);
    if f.mode == "replay" {
        same_rows_as_recorded(&rows, &f.d.to_string_lossy());
    }
    let calls: Vec<&Row> = rows.iter().filter(|(_, k, _, _)| k == "model.called").collect();
    assert!(calls.len() >= 6, "model calls happened:\n{}", dump(&app));
    if f.mode == "replay" {
        for c in &calls {
            assert!(c.3.contains("\"adapter\": \"recorded\""), "every call was replayed: {}", c.3);
            assert!(!c.3.contains("tape miss"), "no miss: {}", c.3);
        }
    }
    let _ = std::fs::remove_dir_all(&f.d);
}
