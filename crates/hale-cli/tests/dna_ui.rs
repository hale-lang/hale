//! GH #566 F6 — `hale dna ui`: the DNA surface from the record alone.
//! No organization runs here; the page and its API answer from the
//! record through the CLI's offline verbs, and a verdict from the
//! form becomes a `review.verdict` row in the record, as the CLI's
//! would.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn git(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git").args(["-c", "user.name=riley", "-c", "user.email=r@l"]).args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn http(port: u16, method: &str, path: &str, body: &str) -> Option<String> {
    let mut s = TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_secs(2)).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
    let req = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).ok()?;
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    Some(out)
}

const DRIVER: &str = r#"import "vendor/dna" as dna;

fn main() {
    let cur = std::io::fs::read_file("main.hl") or "";
    let core = dna::Dna {
        journal: dna::GitJournal { repo: "." },
        gateway: dna::MutationGateway {
            leases: dna::GitLeases { repo: "." },
            workspaces: dna::IsolatedWorktrees { repo: ".", root: ".hale/dna/worktrees" },
            repo: dna::LocalGit { repo: "." }
        },
        verification: dna::HaleVerification { receipts: dna::GitReceipts { repo: "." }, scratch: ".hale/dna/scratch", repo: ".", seed: "." },
        editor: dna::SourceEditor {
            name: "editor",
            models: dna::ModelRouter {
                quick: dna::FakeModel { name: "quick", answer: cur + "// documented for the surface\n" },
                deep: dna::FakeModel { name: "deep", answer: "docs_coverage +" }
            }
        },
        review_policy: dna::OrgPolicy { },
        membrane: dna::Board { who: "board" },
        boundary: dna::AutonomyBoundary { child: "uiapp", grant: dna::Grant { child: "uiapp", classes: "docs refactor", max_magnitude: 4, review: "pre" } }
    };
    println(core.mutate("t0", "docs", "document the entrypoint in main.hl", "main.hl", 1));
}
"#;

#[test]
fn the_surface_serves_the_record_and_a_verdict_from_the_form_lands_in_it() {
    let d = std::env::temp_dir().join(format!("hale_dna_ui_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
    let hale = |args: &[&str], cwd: &Path| -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(cwd).env("HALE_BIN", env!("CARGO_BIN_EXE_hale")).env("XDG_CACHE_HOME", &cache).output().expect("hale");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let (ok, out) = hale(&["dna", "new", "uiapp"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("uiapp");
    let bare = d.join("origin.git");
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], &d);
    git(&["config", "user.name", "riley"], &app);
    git(&["config", "user.email", "r@l"], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the app"], &app);
    git(&["remote", "add", "origin", &bare.to_string_lossy()], &app);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &app);
    std::fs::create_dir_all(app.join("mutate")).unwrap();
    std::fs::write(app.join("mutate/main.hl"), DRIVER).unwrap();
    let (ok, out) = hale(&["run", "mutate"], &app);
    assert!(ok && out.contains("m1: review"), "driver:\n{out}");
    // the surface, with no organization anywhere
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut ui = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "ui", ".", "--port", &port.to_string()])
        .current_dir(&app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("XDG_CACHE_HOME", &cache)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna ui");
    let dl = Instant::now() + Duration::from_secs(120);
    let mut page = None;
    while Instant::now() < dl {
        if let Some(p) = http(port, "GET", "/", "") {
            if p.starts_with("HTTP/1.1 200") {
                page = Some(p);
                break;
            }
        }
        if let Ok(Some(st)) = ui.try_wait() {
            panic!("hale dna ui exited early: {st}");
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let stop = |ui: &mut std::process::Child| {
        let _ = ui.kill();
        let _ = ui.wait();
    };
    let Some(page) = page else {
        stop(&mut ui);
        panic!("the surface never answered on {port}");
    };
    assert!(page.contains("<title>hale dna</title>") && page.contains("/api/status"), "the page");
    let status = http(port, "GET", "/api/status", "").unwrap_or_default();
    assert!(status.contains("\"reviews\"") && status.contains("\"m1\"") && status.contains("reading the Journal"), "status from the record:\n{status}");
    let reviews = http(port, "GET", "/api/reviews", "").unwrap_or_default();
    assert!(reviews.contains("pending review(s)") && reviews.contains("m1 needs leader"), "{reviews}");
    let review = http(port, "GET", "/api/review/m1", "").unwrap_or_default();
    assert!(review.contains("source diff (git") && review.contains("evidence (") && review.contains("documented for the surface"), "the three views:\n{review}");
    let board = http(port, "GET", "/api/board", "").unwrap_or_default();
    assert!(board.starts_with("HTTP/1.1 200") && board.contains("purpose") && board.contains("leader: 1 review(s) inside the grant"), "{board}");
    let fleet = http(port, "GET", "/api/fleet", "").unwrap_or_default();
    assert!(fleet.contains("no `[dna] fleet"), "no fleet here, said plainly:\n{fleet}");
    let history = http(port, "GET", "/api/history/m1", "").unwrap_or_default();
    assert!(history.contains("mutation.candidate"), "{history}");
    // a path cannot smuggle a flag or a file to the CLI
    let smuggled = http(port, "GET", "/api/review/--iris", "").unwrap_or_default();
    assert!(smuggled.contains("no `review.requested` for `-iris`") || smuggled.contains("no `review.requested`"), "{smuggled}");
    // the verdict form: into the record, in the reviewer's name
    let answer = http(port, "POST", "/api/verdict", r#"{"id":"m1","verdict":"reject","as":"riley","comment":"not like this"}"#).unwrap_or_default();
    assert!(answer.contains("verdict reject on m1 by riley sent into the record"), "{answer}");
    let rows = Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().unwrap();
    let rows = String::from_utf8_lossy(&rows.stdout).to_string();
    let verdict = rows.lines().find(|l| l.contains("\"kind\":\"review.verdict\"")).unwrap_or_else(|| panic!("a verdict row:\n{rows}"));
    assert!(verdict.contains("\"author\":\"riley\"") && verdict.contains("reviewer") && verdict.contains("not like this") && verdict.contains("reject"), "{verdict}");
    let ask = http(port, "POST", "/api/ask", r#"{"outcome":"greet twice","to":""}"#).unwrap_or_default();
    assert!(ask.contains("requested in the record"), "{ask}");
    let bad = http(port, "POST", "/api/ask", r#"{"outcome":""}"#).unwrap_or_default();
    assert!(bad.starts_with("HTTP/1.1 400"), "{bad}");
    stop(&mut ui);
    let _ = std::fs::remove_dir_all(&d);
}
