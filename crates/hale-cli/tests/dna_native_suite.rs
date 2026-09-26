//! Runs the DNA Phase 0 domain proof (`dna/tests/`, GH #526) through
//! `hale test`, and `hale verify` over its core, so a compiler change
//! that breaks the domain shapes fails the build here rather than in
//! a friction log. `dna/core` is a library seed (imported by the
//! tests); every fixture is an ordinary Hale program that exits 0.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// The DNA fixtures, run in slices so CI's hash partition can spread
/// them: one test over the whole `dna/tests` directory took eleven
/// minutes on one partition while the others' test phases took three
/// (2026-09-19), and every workflow card adds a fixture. Each slice
/// takes every SLICES-th fixture of the sorted listing and runs them in
/// turn, as `hale test <dir>` did.
///
/// The slices that land on one job run beside each other, four at a
/// time (the `dna-fixtures` test group in `.config/nextest.toml`):
/// ports are already picked free per fixture, and the knowledge
/// database a slice hands its fixtures is now the slice's own
/// (`SliceMemoryDb`), so nothing but the runner is shared. Four
/// because the suite has CI jobs of its own (`dna` in tests.yml) on
/// 4-core runners, and every fixture builds and boots an organism, so
/// one per core is where a fixture's bounded waits stop paying for
/// contention (GH #795). Sixteen slices of ~5 fixtures rather than
/// eight of ~10, so the hash partition over the slice tests spreads
/// them evenly across two jobs. One wedged fixture still ends in a
/// named failure within the group's timeout.
const SLICES: usize = 16;

fn fixture_files() -> Vec<PathBuf> {
    let dir = repo_root().join("dna/tests");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("dna/tests")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.ends_with("_test.hl")))
        .collect();
    files.sort();
    files
}

/// The environment variable a slice stamps on every process its own
/// fixtures start (GH #872). `hale test` runs a fixture as a child of
/// this process, and the fixture spawns its organism, bodies and
/// services with `execvp`, which carries the environment through every
/// generation — so anything one of THIS slice's fixtures leaves running
/// still holds THIS slice's value, while the slice running beside it in
/// the same `cargo test` process (the slices run in parallel by
/// default) and another checkout's DNA run on the same box hold a
/// different value or none at all.
const SLICE_TAG: &str = "HALE_DNA_SUITE_TAG";

/// A value no other slice and no other run on this box can hold: this
/// process, this slice, this moment.
fn slice_tag(slice: usize) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-slice{}-{}", std::process::id(), slice, now)
}

/// The DNA scratch root a command line or a working directory names, if
/// it names one: the fixtures build their roots as
/// `/tmp/dna-<what>-<pid>` and run their organisms from inside them, so
/// the root is what attributes a leaked process to a fixture.
fn dna_scratch_root(text: &str) -> Option<String> {
    let at = text.find("/tmp/dna-")?;
    let rest = &text[at..];
    let after_tmp = "/tmp/".len();
    let end = rest[after_tmp..].find('/').map_or(rest.len(), |i| after_tmp + i);
    // A command line arrives here as one string, and a removed
    // directory reads back from `cwd` as `<path> (deleted)`: either
    // way the root ends at the first blank.
    Some(rest[..end].split_whitespace().next()?.to_string())
}

/// The processes THIS slice's fixtures started that are still running,
/// one line each, named by the scratch root that owns them so a real
/// leak is attributable to the fixture that leaked it.
///
/// A process is this slice's when it carries this slice's stamp; it is
/// a leaked organism when a DNA scratch root is in its command line or
/// is its working directory. Both conditions, so that the slice's own
/// `hale test` children on their way out are not blamed, and so that
/// the scope is exactly what it was before #872 minus the neighbours.
fn leftover_processes(tag: &str) -> Vec<String> {
    let Ok(procfs) = std::fs::read_dir("/proc") else {
        return ledger_leftover_processes(tag);
    };
    let stamp = format!("{SLICE_TAG}={tag}");
    let mut left = Vec::new();
    for entry in procfs.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) else {
            continue;
        };
        let dir = entry.path();
        // Unreadable is not this slice's: either another user's
        // process, or one that ended between the listing and here.
        let Ok(environ) = std::fs::read(dir.join("environ")) else { continue };
        if !environ.split(|b| *b == 0).any(|v| v == stamp.as_bytes()) {
            continue;
        }
        let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
        let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ").trim().to_string();
        let cwd = std::fs::read_link(dir.join("cwd")).map(|p| p.display().to_string()).unwrap_or_default();
        let Some(root) = dna_scratch_root(&cmdline).or_else(|| dna_scratch_root(&cwd)) else { continue };
        left.push(format!("  {root}: pid {pid}: {cmdline}"));
    }
    left.sort();
    left
}

/// The guard where no process's environment can be read — macOS,
/// which returns another process's arguments and never its
/// environment, `ps -E` and `sysctl(KERN_PROCARGS2)` alike (GH #970).
/// It used to fall back to blaming every process under any
/// `/tmp/dna-` root, which blamed the slices running beside it for
/// their live organisms and every fixture's reaper on its way out.
///
/// The slice's own roots stand in for its stamp: `dna::scratch_register`
/// records each in the slice's ledger ([`roots_ledger`]), and a
/// process is this slice's leak when its command line or its working
/// directory (`lsof -d cwd`, readable for the same user) names one of
/// them.
fn ledger_leftover_processes(tag: &str) -> Vec<String> {
    let roots = ledger_roots(tag);
    if roots.is_empty() {
        return Vec::new();
    }
    // A root may appear as itself or through the /private/tmp symlink.
    let names = |text: &str| -> Option<&String> {
        roots.iter().find(|r| {
            [r.to_string(), format!("/private{r}")].iter().any(|p| {
                text.match_indices(p.as_str()).any(|(i, _)| {
                    matches!(text[i + p.len()..].chars().next(), None | Some('/') | Some(' '))
                })
            })
        })
    };
    let mut cwd_of: std::collections::HashMap<String, String> = Default::default();
    if let Ok(out) = Command::new("lsof").args(["-a", "-d", "cwd", "-Fpn"]).output() {
        let mut pid = String::new();
        for l in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(p) = l.strip_prefix('p') {
                pid = p.to_string();
            } else if let Some(n) = l.strip_prefix('n') {
                cwd_of.insert(pid.clone(), n.to_string());
            }
        }
    }
    let ps = Command::new("ps").args(["-eo", "pid=,args="]).output().expect("ps");
    let me = std::process::id().to_string();
    let mut left = Vec::new();
    for l in String::from_utf8_lossy(&ps.stdout).lines() {
        let l = l.trim();
        let pid = l.split_whitespace().next().unwrap_or("").to_string();
        if pid == me {
            continue;
        }
        let root = names(l).or_else(|| cwd_of.get(&pid).and_then(|c| names(c)));
        if let Some(root) = root {
            left.push(format!("  {root}: pid {l}"));
        }
    }
    left.sort();
    left
}

/// Every scratch root a slice's fixtures made, kept when a fixture
/// passes (unlike [`scratch_registry`], whose entry goes with it):
/// `/tmp/dna-suite-<tag>.all`, one file per root holding its path.
fn roots_ledger(tag: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/dna-suite-{tag}.all"))
}

fn ledger_roots(tag: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(roots_ledger(tag)) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .map(|r| r.trim().to_string())
        .filter(|r| r.starts_with("/tmp/dna-") && !r.contains(".."))
        .collect()
}

/// Where a fixture of this slice records the scratch root it made (GH
/// #909): `/tmp/dna-suite-<tag>`, one file per root, holding that
/// root's path. `dna::scratch_root` writes the entry and
/// `dna::scratch_done` drops it, so what is still here when the slice
/// ends is exactly the roots of fixtures that never reached their end
/// — a failed assertion, a kill, a crash.
///
/// A literal `/tmp`, as the fixtures' own roots are and as
/// [`dna_scratch_root`] above reads them out of a command line.
fn scratch_registry(tag: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/dna-suite-{tag}"))
}

/// Whether a scratch root is kept for inspection instead of removed —
/// `dna::keep_scratch()` on the fixture's side, one value deciding
/// both, since a suite that swept up behind the fixture that honoured
/// the knob would be no knob at all.
fn keep_scratch() -> bool {
    std::env::var("HALE_DNA_KEEP_SCRATCH").map_or(false, |v| !v.is_empty() && v != "0")
}

/// Remove the scratch roots THIS slice's fixtures recorded, and the
/// registry with them; the roots removed, for the report.
///
/// Never a root this slice did not stamp: a neighbouring slice's
/// registry carries a different tag, another checkout's DNA run on
/// this box carries a different tag, and a directory a developer left
/// under `/tmp/dna-` is in no registry at all. And never a path that
/// is not shaped like a fixture's root — an entry is a path read off
/// disk driving a recursive remove, so `/tmp/dna-<name>` directly
/// under `/tmp`, with no `..` in it, is the whole of what is acted on.
///
/// `keep` is passed rather than read here so the regression below can
/// exercise both answers without mutating this process's environment
/// (GH #843).
fn sweep_scratch_roots(tag: &str, keep: bool) -> Vec<String> {
    if keep {
        return Vec::new();
    }
    let reg = scratch_registry(tag);
    let Ok(entries) = std::fs::read_dir(&reg) else { return Vec::new() };
    let mut swept = Vec::new();
    for e in entries.flatten() {
        let Ok(root) = std::fs::read_to_string(e.path()) else { continue };
        let root = root.trim().to_string();
        if !root.starts_with("/tmp/dna-") || root.contains("..") || root.matches('/').count() != 2 {
            continue;
        }
        if std::fs::remove_dir_all(&root).is_ok() {
            swept.push(root);
        }
    }
    let _ = std::fs::remove_dir_all(&reg);
    let _ = std::fs::remove_dir_all(roots_ledger(tag));
    swept.sort();
    swept
}

/// The slice's scratch roots, swept when the slice ends however it
/// ends (GH #909). A `Drop`, like [`SliceMemoryDb`], so a fixture's
/// failed assertion — which unwinds out of the slice — leaves the box
/// as the slice found it too. Developer boxes are not ephemeral: the
/// fixtures had left 953 directories totalling 2.2 GB on one.
///
/// It runs after the leftover-process guard on the way out, so a
/// leaked process is still attributed to the root that owns it; a
/// process whose root has gone reads its `cwd` back as `<path>
/// (deleted)`, which [`dna_scratch_root`] already takes.
struct SliceScratch {
    tag: String,
}

impl Drop for SliceScratch {
    fn drop(&mut self) {
        let swept = sweep_scratch_roots(&self.tag, keep_scratch());
        if !swept.is_empty() {
            eprintln!(
                "dna slice: swept {} scratch root(s) no fixture reclaimed (HALE_DNA_KEEP_SCRATCH=1 keeps them):\n  {}",
                swept.len(),
                swept.join("\n  ")
            );
        }
    }
}

/// The DSN `dsn` with its database path replaced by `name`, query
/// string and all: `postgres://dna:dna@host:5480/dna?sslmode=disable`
/// with `dna_1234_s2_…` becomes
/// `postgres://dna:dna@host:5480/dna_1234_s2_…?sslmode=disable`.
/// `None` when the DSN has no `scheme://host/database` shape to edit.
fn dsn_with_database(dsn: &str, name: &str) -> Option<String> {
    let authority = dsn.find("://")? + "://".len();
    let path = authority + dsn[authority..].find('/')?;
    let tail = dsn[path + 1..].find(['?', '#']).map_or(dsn.len(), |i| path + 1 + i);
    Some(format!("{}/{}{}", &dsn[..path], name, &dsn[tail..]))
}

/// A memory database of this slice's own, so that the slices of one
/// partition can run beside each other. CI's `test` job sets one
/// `HALE_DNA_MEMORY_DSN_OWNER` for the whole job and every memory fixture
/// write to it; sharing it is the reason the slices used to be
/// serialized. The slice creates a database named after itself, hands
/// its fixtures a DSN pointing at that one, and drops it on the way out
/// (including when a fixture fails: the drop runs while the assertion
/// unwinds).
///
/// Only for a `postgres` DSN. An unset DSN, or a box with no `psql`,
/// leaves the fixtures with what the environment gave them —
/// nothing about the suite requires the isolation, it only lets the
/// slices overlap.
struct SliceMemoryDb {
    /// The DSN the database was created from, and will be dropped from.
    admin: String,
    name: String,
    /// The DSN this slice's fixtures get: `admin` with its database
    /// replaced by `name`.
    dsn: String,
}

impl SliceMemoryDb {
    fn create(slice: usize) -> Option<SliceMemoryDb> {
        let admin = std::env::var("HALE_DNA_MEMORY_DSN_OWNER").ok()?;
        if !admin.starts_with("postgres") {
            return None;
        }
        // Lowercase, unqualified, well under Postgres's 63-byte
        // identifier limit, and unique per process, slice and moment
        // for the same reason the slice tag is: another checkout may be
        // running its own DNA suite against this same server.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() % 1_000_000_000)
            .unwrap_or(0);
        let name = format!("dna_{}_s{}_{}", std::process::id(), slice, now);
        // Rewritten before it is created: a DSN this cannot edit is one
        // the fixtures could not be pointed at anyway, and an
        // unreachable database is not worth creating.
        let dsn = dsn_with_database(&admin, &name)?;
        match Command::new("psql").arg(&admin).args(["-v", "ON_ERROR_STOP=1", "-c", &format!("CREATE DATABASE {name}")]).output() {
            Ok(out) if out.status.success() => Some(SliceMemoryDb { admin, name, dsn }),
            Ok(out) => {
                eprintln!(
                    "dna slice {slice}: could not create its own memory database ({name}); \
                     falling back to the shared HALE_DNA_MEMORY_DSN_OWNER:\n{}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
                None
            }
            Err(e) => {
                eprintln!(
                    "dna slice {slice}: no usable `psql` ({e}), so it cannot take a memory database of its own; \
                     falling back to the shared HALE_DNA_MEMORY_DSN_OWNER. The slices of one partition then write to \
                     one database while they run beside each other."
                );
                None
            }
        }
    }
}

impl Drop for SliceMemoryDb {
    fn drop(&mut self) {
        // Best effort: a database left behind costs a CI runner that is
        // thrown away in minutes nothing, and failing the slice over
        // the cleanup would bury the failure that got us here.
        let sql = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name);
        let out = Command::new("psql").arg(&self.admin).args(["-v", "ON_ERROR_STOP=1", "-c", &sql]).output();
        if !matches!(&out, Ok(o) if o.status.success()) {
            eprintln!("dna slice: could not drop its memory database {} (harmless; it is a scratch database)", self.name);
        }
    }
}

/// What one fixture cost, for the slice's timing table.
struct FixtureTiming {
    fixture: String,
    seconds: f64,
}

/// The slice's fixtures, slowest first, on stdout and — under GitHub
/// Actions — in the job summary. Which fixture a slice's minutes went
/// to is otherwise invisible: the slices were sized by hashing, and the
/// spread between them (59s to 196s on the 2026-09-20 run) is a
/// property of their contents that nothing reported.
fn report_timings(slice: usize, timings: &mut Vec<FixtureTiming>) {
    if timings.is_empty() {
        return;
    }
    timings.sort_by(|a, b| b.seconds.total_cmp(&a.seconds).then_with(|| a.fixture.cmp(&b.fixture)));
    let total: f64 = timings.iter().map(|t| t.seconds).sum();
    let width = timings.iter().map(|t| t.fixture.len()).max().unwrap_or(0);
    println!("DNA fixtures, slice {slice}: {} fixtures, {total:.1}s", timings.len());
    for t in timings.iter() {
        println!("  {:<width$}  {:>7.1}s", t.fixture, t.seconds, width = width);
    }
    let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") else { return };
    let mut md = format!("### DNA fixtures, slice {slice}\n\n| fixture | seconds |\n| --- | ---: |\n");
    for t in timings.iter() {
        md.push_str(&format!("| {} | {:.1} |\n", t.fixture, t.seconds));
    }
    md.push_str(&format!("| **total** | **{total:.1}** |\n\n"));
    // Best effort, like the drop above: a summary that cannot be
    // written is not a test failure.
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&summary) {
        use std::io::Write;
        let _ = f.write_all(md.as_bytes());
    }
}

/// A directory holding a `timeout` for the fixtures, on a machine that has
/// none (a stock Mac: `timeout` is GNU coreutils), `None` where one is on
/// PATH. The fixtures cap every host they start in the background with
/// `timeout <secs> …`, sixteen sites in all; without it on PATH each one
/// failed to start its host at all (GH #970).
///
/// The stand-in has GNU's semantics, not merely its name, because the
/// fixtures lean on them: `timeout` puts itself and the command in a
/// process group of their own, and when it is signalled — a fixture
/// stopping the host it started — or its time runs out, it signals that
/// whole GROUP, so what the command started stops with it; 124 on expiry,
/// 128+n when the command died of signal n. A first stand-in that merely
/// exec'd the command under an alarm let a host's children outlive it,
/// and two fixtures read their record still moving after "the organism is
/// down". Perl, which every Mac ships. One per process, made once.
fn timeout_shim_dir() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let has = Command::new("sh")
            .args(["-c", "command -v timeout >/dev/null 2>&1"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if has {
            return None;
        }
        let dir = std::env::temp_dir().join(format!("hale-dna-suite-bin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let shim = dir.join("timeout");
        std::fs::write(&shim, TIMEOUT_SHIM).ok()?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).ok()?;
        Some(dir)
    })
    .clone()
}

/// GNU `timeout <secs> <cmd…>`, in perl: see [`timeout_shim_dir`].
const TIMEOUT_SHIM: &str = r#"#!/usr/bin/env perl
# GNU timeout, for a machine without coreutils (dna_native_suite.rs).
use strict;
my $secs = shift @ARGV;
setpgrp(0, 0);                        # a group of our own, as GNU does
my $pid = fork();
die "timeout: fork: $!
" unless defined $pid;
if ($pid == 0) { exec { $ARGV[0] } @ARGV; exit 127; }
my $expired = 0;
# signal the whole group once, never ourselves again
sub group { my ($sig) = @_; $SIG{$_} = 'IGNORE' for qw(TERM INT HUP QUIT); kill $sig, -$$; }
for my $s (qw(TERM INT HUP QUIT)) { $SIG{$s} = sub { group($s) }; }
$SIG{ALRM} = sub { $expired = 1; group('TERM') };
alarm $secs;
while (waitpid($pid, 0) < 0) { last unless $!{EINTR}; }
my $st = $?;
exit 124 if $expired;
exit(($st & 127) ? 128 + ($st & 127) : $st >> 8);
"#;

/// Run one fixture, returning what it cost or why it failed. `hale test
/// --json` is what carries the cost: the text form reports only the
/// pass/fail summary, and the wall time of the child here would fold
/// the compile of every import into the first fixture that needed it.
fn run_one_fixture(f: &PathBuf, tag: &str, dsn: Option<&str>) -> Result<FixtureTiming, String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hale"));
    cmd.arg("test").arg("--json").arg(f).env("HALE_BIN", env!("CARGO_BIN_EXE_hale"));
    cmd.env("HALE_DNA_SOURCE", repo_root());
    cmd.env(SLICE_TAG, tag);
    cmd.current_dir(std::env::temp_dir());
    if let Some(dsn) = dsn {
        cmd.env("HALE_DNA_MEMORY_DSN_OWNER", dsn);
    }
    // GH #795: a fixture's bounded waits are written for a quiet
    // machine, and here they are not on one — the slices run beside the
    // rest of the workspace's tests, and beside one another, with an
    // organism, its bodies and their builds inside each fixture.
    // `dna::wait_scale` widens every wait at once, and the suite asks
    // for double by default; a slower runner can ask for more from the
    // environment. Two, not more: the widest wait in a fixture is 300s,
    // and the slice's own `terminate-after` allowance is 15 minutes for
    // every fixture in it, so one stuck wait must not be able to eat
    // the whole slice. Running a fixture by hand keeps the
    // quiet-machine bounds, so a real hang is still reported in seconds
    // rather than minutes.
    if std::env::var_os("HALE_DNA_WAIT_SCALE").is_none() {
        cmd.env("HALE_DNA_WAIT_SCALE", "2");
    }
    if let Some(dir) = timeout_shim_dir() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut parts = vec![dir.clone()];
        parts.extend(std::env::split_paths(&path));
        cmd.env("PATH", std::env::join_paths(parts).expect("PATH"));
    }
    let out = cmd.output().expect("invoke hale test on a DNA fixture");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The whole of both streams, as before: what the organism said on
    // its way down is most of the diagnosis.
    let failed = |why: &str| format!("the DNA fixture {} {why}\n\nstdout:\n{}\nstderr:\n{}", f.display(), stdout, stderr);
    if !out.status.success() {
        return Err(failed("failed."));
    }
    // `hale test --json` prints the outcome array as its last line.
    let line = stdout.lines().rev().find(|l| l.trim_start().starts_with('[')).ok_or_else(|| failed("printed no JSON outcome array."))?;
    let outcomes: Vec<serde_json::Value> =
        serde_json::from_str(line.trim()).map_err(|e| failed(&format!("printed an unreadable JSON outcome array ({e}).")))?;
    // A fixture that ran nothing passes vacuously: `hale test` exits 0
    // over an empty set, so the count is part of the contract, as the
    // `, 0 failed` summary this replaces made it.
    if outcomes.is_empty() {
        return Err(failed("ran no test."));
    }
    // The JSON escapes a failure's own output; unescaped here so the
    // assertion that failed reads as it would from `hale test`.
    let not_passed: Vec<String> = outcomes
        .iter()
        .filter(|o| o["status"] != "pass")
        .map(|o| format!("{}:\n{}", o["file"].as_str().unwrap_or("?"), o["message"].as_str().unwrap_or("(no message)")))
        .collect();
    if !not_passed.is_empty() {
        return Err(failed(&format!("did not pass:\n{}", not_passed.join("\n"))));
    }
    let ms: u64 = outcomes.iter().map(|o| o["elapsed_ms"].as_u64().unwrap_or(0)).sum();
    Ok(FixtureTiming {
        fixture: f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.display().to_string()),
        seconds: ms as f64 / 1000.0,
    })
}

fn run_fixture_slice(slice: usize) {
    let files = fixture_files();
    assert!(!files.is_empty(), "no DNA fixtures under dna/tests");
    let mine: Vec<&PathBuf> = files.iter().enumerate().filter(|(i, _)| i % SLICES == slice).map(|(_, f)| f).collect();
    let tag = slice_tag(slice);
    // Held to the end of the slice, and dropped on the way out of a
    // failure too (GH #909).
    let _scratch = SliceScratch { tag: tag.clone() };
    let db = SliceMemoryDb::create(slice);
    // Without one of its own — no postgres DSN, no `psql` — the
    // fixtures get exactly what this process was given.
    let dsn = match &db {
        Some(db) => Some(db.dsn.clone()),
        None => std::env::var("HALE_DNA_MEMORY_DSN_OWNER").ok(),
    };

    let mut timings: Vec<FixtureTiming> = Vec::with_capacity(mine.len());
    let mut failure = None;
    for f in &mine {
        // Progress on stderr as it happens: a slice that nextest kills
        // at its deadline shows what it had captured, so the fixture
        // that ate the minutes is named rather than the slice alone.
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        eprintln!("slice {slice}: {name} starting");
        let started = std::time::Instant::now();
        let outcome = run_one_fixture(f, &tag, dsn.as_deref());
        eprintln!(
            "slice {slice}: {name} {} after {:.1}s",
            if outcome.is_ok() { "passed" } else { "failed" },
            started.elapsed().as_secs_f64()
        );
        match outcome {
            // A fixture failure stops the slice, as it always has: the
            // fixtures after it would run against whatever the failing
            // one left behind.
            Err(why) => {
                failure = Some(why);
                break;
            }
            Ok(t) => timings.push(t),
        }
    }
    // Before the panic, so a slice that failed still says where its
    // minutes went.
    report_timings(slice, &mut timings);
    if let Some(why) = failure {
        panic!("{why}");
    }
    // Every process a fixture started must be gone once the fixture is:
    // a host, a body or a service left running is a leak the fixture
    // did not reclaim. The wait is for a child still tearing down; the
    // filter is scoped to the processes THIS slice started (GH #872),
    // because the slices of one `cargo test` run in parallel by default
    // and other checkouts run their own DNA fixtures on the same box —
    // unscoped, the slice that finished first was blamed for a
    // neighbour's live organism.
    std::thread::sleep(std::time::Duration::from_secs(5));
    let left = leftover_processes(&tag);
    assert!(
        left.is_empty(),
        "processes this slice's DNA fixtures started are still running ({SLICE_TAG}={tag}), by the scratch root that owns them:\n{}",
        left.join("\n")
    );
}

macro_rules! fixture_slices {
    ($($name:ident => $i:expr),* $(,)?) => { $( #[test] fn $name() { run_fixture_slice($i); } )* };
}
fixture_slices! {
    dna_fixtures_slice_0 => 0, dna_fixtures_slice_1 => 1, dna_fixtures_slice_2 => 2, dna_fixtures_slice_3 => 3,
    dna_fixtures_slice_4 => 4, dna_fixtures_slice_5 => 5, dna_fixtures_slice_6 => 6, dna_fixtures_slice_7 => 7,
    dna_fixtures_slice_8 => 8, dna_fixtures_slice_9 => 9, dna_fixtures_slice_10 => 10, dna_fixtures_slice_11 => 11,
    dna_fixtures_slice_12 => 12, dna_fixtures_slice_13 => 13, dna_fixtures_slice_14 => 14, dna_fixtures_slice_15 => 15,
}

/// The slice's own memory database is reached by editing exactly the
/// database of the job's DSN: the user, password, host, port and query
/// string are what let a fixture connect at all, and CI's DSN carries
/// `?sslmode=disable`.
#[test]
fn a_slice_database_edits_only_the_database_of_the_dsn() {
    assert_eq!(
        dsn_with_database("postgres://dna:dna@127.0.0.1:5480/dna?sslmode=disable", "dna_7_s2_9").as_deref(),
        Some("postgres://dna:dna@127.0.0.1:5480/dna_7_s2_9?sslmode=disable")
    );
    assert_eq!(dsn_with_database("postgresql://dna@host/dna", "s").as_deref(), Some("postgresql://dna@host/s"));
    // No database to replace: the caller keeps the DSN it was given.
    assert_eq!(dsn_with_database("postgres://dna@host", "s"), None);
    assert_eq!(dsn_with_database("memory", "s"), None);
}

#[test]
fn the_slices_cover_every_fixture_once() {
    let files = fixture_files();
    let mut covered = 0;
    for slice in 0..SLICES {
        covered += files.iter().enumerate().filter(|(i, _)| i % SLICES == slice).count();
    }
    assert_eq!(covered, files.len(), "every fixture belongs to exactly one slice");
}

/// GH #872: the leftover-process guard blames only what its own slice
/// started. Two slice bodies' guards run here at the same moment in one
/// process: slice A has a process still running under its own scratch
/// root, slice B has left nothing, and a third process under a DNA root
/// of its own stands in for another checkout's run on the same box.
/// Only A is blamed, its message names A's root, and B passes.
///
/// Before the scoping every guard saw every `/tmp/dna-` process on the
/// box, so B failed for A's leak and both failed for the stranger's.
/// Cheap by construction: no fixture runs, the leak is a `sleep`.
///
/// On macOS the same scoping holds through the slice's roots ledger
/// rather than the environment stamp (GH #970), so it runs there too.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn the_leftover_guard_blames_only_its_own_slice() {
    let pid = std::process::id();
    let tag_a = slice_tag(0);
    let tag_b = slice_tag(1);
    let tag_stranger = format!("another-checkouts-slice-{pid}");
    let root_a = format!("/tmp/dna-guard872-a-{pid}");
    let root_stranger = format!("/tmp/dna-guard872-stranger-{pid}");
    let mut running = Vec::new();
    for (root, tag) in [(&root_a, tag_a.as_str()), (&root_stranger, tag_stranger.as_str())] {
        std::fs::create_dir_all(root).expect("a scratch root for the guard's regression");
        // What `dna::scratch_register` records for a fixture's root; the
        // macOS guard attributes by it, having no stamp to read.
        let ledger = roots_ledger(tag);
        std::fs::create_dir_all(&ledger).expect("the slice's roots ledger");
        std::fs::write(ledger.join(root.trim_start_matches("/tmp/")), root).expect("record the root");
        running.push(
            Command::new("sleep")
                .arg("120")
                .current_dir(root)
                .env(SLICE_TAG, tag)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("leave a process running under a DNA scratch root"),
        );
    }
    let leaked_pid = running[0].id();
    // The stamp reaches a process at exec, not at fork: wait for BOTH
    // to be visible before either guard looks, so what this pins is the
    // scoping and not a race — a stranger who had not exec'd yet would
    // pass the guard for having no environment rather than for holding
    // someone else's stamp.
    for tag in [tag_a.as_str(), tag_stranger.as_str()] {
        for _ in 0..100 {
            if !leftover_processes(tag).is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    let (a, b) = (tag_a.clone(), tag_b.clone());
    let guard_a = std::thread::spawn(move || leftover_processes(&a));
    let guard_b = std::thread::spawn(move || leftover_processes(&b));
    let left_a = guard_a.join().expect("slice A's guard");
    let left_b = guard_b.join().expect("slice B's guard");

    // Reclaim before asserting: this test must not itself leave a
    // process under a DNA root for the next slice to trip over.
    for mut child in running {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_stranger);
    let _ = std::fs::remove_dir_all(roots_ledger(&tag_a));
    let _ = std::fs::remove_dir_all(roots_ledger(&tag_stranger));

    assert_eq!(left_a.len(), 1, "slice A is blamed for its own leak and for nothing else, got:\n{}", left_a.join("\n"));
    assert!(left_a[0].contains(&root_a), "slice A's message names the scratch root that owns the leak, got:\n{}", left_a[0]);
    assert!(
        left_a[0].contains(&format!("pid {leaked_pid}")),
        "slice A's message names the process that leaked, got:\n{}",
        left_a[0]
    );
    assert!(
        left_b.is_empty(),
        "slice B left nothing running: neither slice A's leak nor a stranger's process may fail it, got:\n{}",
        left_b.join("\n")
    );
}

/// GH #909: a slice sweeps the scratch roots its own fixtures recorded
/// and nothing else. Four cases stand side by side here — a root this
/// slice stamped, a root a stranger stamped (another checkout's DNA run
/// on this box), a root under the keep knob, and a registry entry
/// naming a path that is not a fixture's root at all, which a sweep
/// driven by a file on disk must refuse to act on.
///
/// Cheap by construction: no fixture runs, the roots are empty
/// directories, and the decoy is the test's own so a broken guard can
/// only destroy what this test made.
#[test]
fn the_slice_sweeps_only_the_scratch_roots_its_own_fixtures_stamped() {
    let pid = std::process::id();
    let (mine, stranger, kept) = (format!("sweep909-mine-{pid}"), format!("sweep909-stranger-{pid}"), format!("sweep909-kept-{pid}"));
    let root_of = |tag: &str| format!("/tmp/dna-{tag}");
    // A path that is not a fixture's scratch root, reached from an
    // entry in THIS slice's own registry: the guard, not the tag, is
    // what has to refuse it.
    let decoy = format!("/tmp/e1-909-decoy-{pid}");

    let stamp = |tag: &str, entry: &str, holds: &str| {
        let reg = scratch_registry(tag);
        std::fs::create_dir_all(&reg).expect("a slice's scratch registry");
        std::fs::write(reg.join(entry), holds).expect("a root's registry entry");
    };
    for tag in [&mine, &stranger, &kept] {
        std::fs::create_dir_all(root_of(tag)).expect("a scratch root");
        std::fs::write(format!("{}/what-the-fixture-built", root_of(tag)), "x").expect("something in it");
        stamp(tag, &format!("dna-{tag}"), &root_of(tag));
    }
    std::fs::create_dir_all(&decoy).expect("the decoy");
    stamp(&mine, "not-a-scratch-root", &decoy);
    stamp(&mine, "traversal", &format!("{}/../e1-909-decoy-{pid}", root_of(&mine)));

    let swept = sweep_scratch_roots(&mine, false);

    assert_eq!(swept, vec![root_of(&mine)], "the slice sweeps its own root, and only its own");
    assert!(!PathBuf::from(root_of(&mine)).exists(), "this slice's root is gone");
    assert!(PathBuf::from(root_of(&stranger)).exists(), "a root another run stamped is untouched: it is in no registry of ours");
    assert!(PathBuf::from(&decoy).exists(), "a registry entry naming something that is not a fixture's root is not acted on");
    assert!(!scratch_registry(&mine).exists(), "and the registry goes with the roots it held");

    // The keep knob, on the same registry shape: nothing removed, and
    // the registry left for the next sweep that is allowed to.
    assert!(sweep_scratch_roots(&kept, true).is_empty(), "HALE_DNA_KEEP_SCRATCH sweeps nothing");
    assert!(PathBuf::from(root_of(&kept)).exists(), "the kept root is still there to look at");
    assert!(scratch_registry(&kept).exists(), "with its registry entry");

    // The stranger's own slice does sweep it — which is also this
    // test's cleanup.
    assert_eq!(sweep_scratch_roots(&stranger, false), vec![root_of(&stranger)], "the slice that stamped it is the one that sweeps it");
    assert_eq!(sweep_scratch_roots(&kept, false), vec![root_of(&kept)]);
    let _ = std::fs::remove_dir_all(&decoy);
}

#[test]
fn dna_core_verifies_clean() {
    let dir = repo_root().join("dna/core");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("verify")
        .arg(&dir)
        .output()
        .expect("invoke hale verify dna/core");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "hale verify dna/core must report zero findings.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
}

/// A suite that quietly emptied would pass the check above by
/// running nothing. #526 names eight programs; seven are Hale-native,
/// #528 adds the hosted-model adapter's, #529 the gateways' and
/// the source-editing Attempt's, #583 the budget's and the Anthropic
/// adapter's.
#[test]
fn dna_fixture_set_is_complete() {
    let dir = repo_root().join("dna/tests");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("dna/tests exists")
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.ends_with("_test.hl").then_some(n)
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "acceptance_binding_test.hl",
            "anthropic_messages_test.hl",
            "apply_test.hl",
            "assembly_test.hl",
            "attempt_commands_test.hl",
            "b1_team_test.hl",
            "binding_store_test.hl",
            "body_claim_expired_test.hl",
            "body_lease_blocked_test.hl",
            "body_lease_start_test.hl",
            "body_provision_script_test.hl",
            "body_scan_test.hl",
            "books_slice_test.hl",
            "budget_test.hl",
            "claims_test.hl",
            "command_relay_trust_test.hl",
            "concern_identity_test.hl",
            "concern_restart_test.hl",
            "deployment_test.hl",
            "edge_projection_test.hl",
            "edge_unlink_test.hl",
            "editing_test.hl",
            "effect_outcomes_test.hl",
            "embedded_provenance_test.hl",
            "extensions_test.hl",
            "genome_pull_test.hl",
            "grant_layering_test.hl",
            "grant_resources_test.hl",
            "graph_ingest_refusals_test.hl",
            "graph_ingest_test.hl",
            "graph_model_test.hl",
            "handed_task_test.hl",
            "handoff_test.hl",
            "harness_test.hl",
            "journal_contention_test.hl",
            "journal_test.hl",
            "knowledge_binding_recovery_test.hl",
            "knowledge_context_test.hl",
            "knowledge_document_test.hl",
            "knowledge_edge_review_test.hl",
            "knowledge_events_test.hl",
            "knowledge_node_recovery_test.hl",
            "knowledge_node_relay_test.hl",
            "knowledge_store_test.hl",
            "knowledge_test.hl",
            "lease_epoch_test.hl",
            "ledger_test.hl",
            "mutation_review_test.hl",
            "native_json_test.hl",
            "nerves_compose_test.hl",
            "nerves_fake_test.hl",
            "nerves_loss_test.hl",
            "node_document_test.hl",
            "openai_chat_test.hl",
            "optimize_test.hl",
            "org_test.hl",
            "organization_ownership_source_test.hl",
            "organization_source_request_test.hl",
            "organization_source_review_test.hl",
            "owners_test.hl",
            "ownership_test.hl",
            "plan_routing_test.hl",
            "practice_command_recovery_test.hl",
            "practice_lifecycle_test.hl",
            "practice_test.hl",
            "principal_oidc_test.hl",
            "principal_test.hl",
            "projection_fence_test.hl",
            "projectors_test.hl",
            "prompt_receipt_test.hl",
            "receipt_retention_test.hl",
            "receipt_vault_test.hl",
            "record_identity_test.hl",
            "record_test.hl",
            "recorded_model_test.hl",
            "recovery_association_test.hl",
            "relay_repeated_request_test.hl",
            "retired_admission_test.hl",
            "review_authority_test.hl",
            "review_command_test.hl",
            "routing_test.hl",
            "row_admission_test.hl",
            "schedule_cli_test.hl",
            "schedule_test.hl",
            "status_chain_race_test.hl",
            "supersession_test.hl",
            "support_test.hl",
            "sync_head_read_test.hl",
            "task_decide_test.hl",
            "task_evidence_test.hl",
            "two_heads_test.hl",
            "two_owners_test.hl",
            "usage_test.hl",
            "verification_test.hl",
            "workflow_admission_contention_test.hl",
            "workflow_admission_test.hl",
            "workflow_attempt_test.hl",
            "workflow_case_completion_test.hl",
            "workflow_children_test.hl",
            "workflow_conformance_test.hl",
            "workflow_definition_test.hl",
            "workflow_edit_outcomes_test.hl",
            "workflow_events_test.hl",
            "workflow_human_cases_test.hl",
            "workflow_lifetime_test.hl",
            "workflow_projection_test.hl",
            "workflow_public_admission_test.hl",
            "workflow_reconcile_test.hl",
            "workflow_recovery_test.hl",
            "workflow_release_type_wide_test.hl",
            "workflow_restore_test.hl",
            "workflow_retry_test.hl",
            "workflow_run_test.hl",
            "workflow_step_test.hl",
            "workspace_test.hl",
        ]
    );
}

/// GH #646 stage 0 (#649): the record has an API of its own, and only its
/// implementation spells the tool. In the core and the host, `git` is
/// invoked from exactly the record's implementation and the genome's own
/// files (the Structure): a new call site anywhere else is a boundary
/// crossed, and this fails the build until it goes behind `dna::Record`
/// (or `genome.hl` in the host).
#[test]
fn only_the_record_and_the_genome_spell_git() {
    let root = repo_root();
    let allowed = ["dna/core/record.hl", "dna/core/workspace.hl", "dna/core/verification.hl", "dna/core/org.hl", "dna/host/genome.hl"];
    let mut offenders = Vec::new();
    for dir in ["dna/core", "dna/host"] {
        for e in std::fs::read_dir(root.join(dir)).expect("dna dir").flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "hl").unwrap_or(true) {
                continue;
            }
            let rel = format!("{dir}/{}", p.file_name().unwrap().to_string_lossy());
            let text = std::fs::read_to_string(&p).unwrap();
            let spells = text.lines().enumerate().filter(|(_, l)| {
                let l = l.trim_start();
                !l.starts_with("//") && (l.contains("run_tool(\"git") || l.contains("\"git\\n") || l.contains("\\ngit\\n") || l.contains("process::run(\"git"))
            });
            for (i, l) in spells {
                if !allowed.contains(&rel.as_str()) {
                    offenders.push(format!("{rel}:{}: {}", i + 1, l.trim()));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "git is spelled outside the record's implementation and the genome's files:\n{}", offenders.join("\n"));
}

/// GH #647: a body's infrastructure and transport are implementations
/// behind the core's interfaces. In the host, `ssh`, `systemctl`,
/// `journalctl` and `docker compose` are invoked from exactly one file.
#[test]
fn only_the_reference_infrastructure_spells_its_tools() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for e in std::fs::read_dir(root.join("dna/host")).expect("dna/host").flatten() {
        let p = e.path();
        if p.extension().map(|x| x != "hl").unwrap_or(true) || p.file_name().unwrap() == "infra.hl" {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        for (i, l) in text.lines().enumerate() {
            let t = l.trim_start();
            if t.starts_with("//") {
                continue;
            }
            if t.contains("ssh -o") || t.contains("systemctl --user") || t.contains("journalctl") || t.contains("docker\\ncompose") {
                offenders.push(format!("dna/host/{}:{}: {}", p.file_name().unwrap().to_string_lossy(), i + 1, t));
            }
        }
    }
    assert!(offenders.is_empty(), "a tool of the body's infrastructure is spelled outside dna/host/infra.hl:\n{}", offenders.join("\n"));
}

/// GH #648: the code-review host is an implementation behind `Forge`;
/// `gh` is invoked from exactly one file in the host.
#[test]
fn only_the_github_forge_spells_gh() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for dir in ["dna/core", "dna/host"] {
        for e in std::fs::read_dir(root.join(dir)).expect("dna dir").flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "hl").unwrap_or(true) || p.file_name().unwrap() == "forge_github.hl" {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            for (i, l) in text.lines().enumerate() {
                let t = l.trim_start();
                if !t.starts_with("//") && (t.contains("run_tool(\"gh") || t.contains("\"gh\\n")) {
                    offenders.push(format!("{dir}/{}:{}: {}", p.file_name().unwrap().to_string_lossy(), i + 1, t));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "gh is spelled outside dna/host/forge_github.hl:\n{}", offenders.join("\n"));
}
