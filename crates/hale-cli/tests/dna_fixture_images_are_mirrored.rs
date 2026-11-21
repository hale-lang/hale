//! A DNA fixture never pulls an image from Docker Hub: it names the
//! repository's mirror on ghcr.io.
//!
//! Docker Hub's anonymous pull limit is counted per address, and a CI
//! runner's address is shared with every other runner's, so a fixture that
//! started `busybox:1.36` failed whole DNA runs at random (GH #2141, as
//! pgvector, NATS and Prometheus had before it). `mirror-images.yml`
//! copies each image a fixture starts to `ghcr.io/hale-lang/ci-<name>`,
//! and a fixture names it through a `dna::TEST_*` constant in
//! `dna/core/workspace.hl`. As `dna_fixture_ports_are_free` refuses a
//! drawn port, this refuses the next Docker Hub name, wherever a fixture
//! spells the image it starts:
//!
//! - a compose file's `image:`;
//! - a Dockerfile's `FROM`;
//! - a `docker run` or `docker create` argv (`docker\nrun\n...`), where an
//!   argument with a tag (`name:tag`) or a registry path is the image (an
//!   untagged word there cannot be told from the command after it).
//!
//! A name is fine when it is the mirror (`ghcr.io/hale-lang/...`), a
//! fixture's own build (`hale-dna-test-*`, built and never pulled), another
//! registry's, or one of the Docker Hub names `tests.yml` pulls from the
//! mirror and tags before any fixture runs (its `for pair in` lists). An
//! `image:` or `FROM` spelled as an expression names a `dna::TEST_*`
//! constant. Refused: `docker.io/...`, and a bare name (`busybox:1.36`,
//! `alpine`), which is Docker Hub's.
//!
//! Scanned: every `*.hl` under a directory named `tests` anywhere in
//! `dna/`. Left alone, each for its reason:
//!
//! - `dna/tests/onboarding/`: repositories of their own, shaped as a
//!   user's clone; a fixture starts their bodies under
//!   `HALE_DNA_COMPOSE=off`;
//! - [`READS_ONLY`]: fixtures that hand compose or manifest text to a
//!   parser and start nothing (each is held to never running `docker`
//!   or a verb that starts a body).
//!
//! `dna/core/teardown.hl`'s `busybox:1.36` is production: it runs on the
//! user's machine, where Docker Hub is right, and is not scanned.

use std::path::{Path, PathBuf};

const EXEMPT_DIRS: &[&str] = &["dna/tests/onboarding/"];

/// Fixtures whose image names are data: read as written, never started.
const READS_ONLY: &[&str] = &[
    "dna/tests/compose_facets_test.hl",
    "dna/tests/graph_ingest_refusals_test.hl",
    "dna/tests/habitat_compose_block_test.hl",
    "dna/tests/k8s_workloads_test.hl",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The Docker Hub names `tests.yml` tags from the mirror: the right-hand
/// side of each `ci-<name>:<tag>=<docker hub name>` in a `for pair in` list.
fn seeded_from_mirror() -> Vec<String> {
    let text = std::fs::read_to_string(repo_root().join(".github/workflows/tests.yml")).expect("tests.yml");
    let mut out = Vec::new();
    for line in text.lines().filter(|l| l.trim_start().starts_with("for pair in ")) {
        for tok in line.split_whitespace() {
            if let Some((_, hub)) = tok.trim_end_matches(';').split_once('=') {
                if !out.iter().any(|s: &String| s == hub) {
                    out.push(hub.to_string());
                }
            }
        }
    }
    out
}

fn name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-'
}

/// Whether `s` reads as an image reference with a tag or a registry path:
/// `[registry[:port]/]path[:tag]`, lowercase path components holding a
/// letter, and at least one of a tag or a `/`.
fn looks_like_reference(s: &str) -> bool {
    if s.is_empty() || s.contains("::") || s.contains('=') {
        return false;
    }
    let (path, tag) = match s.rfind(':') {
        Some(i) if !s[i..].contains('/') => (&s[..i], Some(&s[i + 1..])),
        _ => (s, None),
    };
    if let Some(t) = tag {
        if t.is_empty() || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-') {
            return false;
        }
    }
    let mut parts: Vec<&str> = path.split('/').collect();
    // a registry with a port
    if parts.len() > 1 {
        if let Some((host, port)) = parts[0].split_once(':') {
            if !port.chars().all(|c| c.is_ascii_digit()) || host.is_empty() {
                return false;
            }
            parts[0] = host;
        }
    }
    let ok = parts.iter().all(|p| !p.is_empty() && p.chars().all(name_char)) && path.chars().any(|c| c.is_ascii_lowercase());
    ok && (tag.is_some() || parts.len() > 1)
}

/// Why `image` may not be named by a fixture, or `None` when it may.
fn refusal(image: &str, seeded: &[String]) -> Option<&'static str> {
    let first = image.split('/').next().unwrap_or("");
    let registry = image.contains('/') && (first.contains('.') || first.contains(':') || first == "localhost");
    if image.starts_with("ghcr.io/hale-lang/") || image.starts_with("hale-dna-test-") || seeded.iter().any(|s| s == image) {
        None
    } else if matches!(first, "docker.io" | "index.docker.io" | "registry-1.docker.io") {
        Some("a Docker Hub image")
    } else if registry {
        None
    } else {
        Some("a bare name, which is Docker Hub's")
    }
}

/// The value of a YAML/Dockerfile field starting at `rest`: an optional
/// quote, then up to the next delimiter in source (an escape, a quote,
/// whitespace, a flow delimiter).
fn field_value(rest: &str) -> &str {
    let rest = rest.trim_start_matches(' ');
    let rest = rest.strip_prefix("\\\"").or_else(|| rest.strip_prefix('\'')).unwrap_or(rest);
    let end = rest.find(|c: char| matches!(c, '\\' | '"' | '\'' | ' ' | ',' | '}' | ']' | '#')).unwrap_or(rest.len());
    &rest[..end]
}

/// When the value is spelled as an expression (`image: " + x + "`), the
/// operand it is concatenated with; `rest` runs to the end of the file, as
/// `hale fmt` may break the line before the operand.
fn expression_operand(rest: &str) -> Option<&str> {
    let after = rest.trim_start_matches(' ').strip_prefix('"')?.trim_start();
    let after = after.strip_prefix('+')?.trim_start();
    let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':')).unwrap_or(after.len());
    Some(&after[..end])
}

/// Whether the `image:` at byte `i` of `line` is a YAML key: at the line's
/// start or after whitespace, a quote, or a `\n` escape.
fn starts_a_key(line: &str, i: usize) -> bool {
    let before = &line[..i];
    before.is_empty() || before.ends_with(' ') || before.ends_with('\t') || before.ends_with('"') || before.ends_with("\\n")
}

/// Whether the `FROM ` at byte `i` of `line` starts a Dockerfile line: at
/// the start of a literal, after a `\n` escape, or alone at the start of a
/// line of a literal that spans lines. SQL's `FROM` follows a word.
fn starts_a_dockerfile_line(line: &str, i: usize) -> bool {
    let before = &line[..i];
    before.trim().is_empty() || before.ends_with('"') || before.ends_with("\\n")
}

/// Whether a `FROM` value ends where a Dockerfile's does: at the end of the
/// line or the literal, or before ` AS `. `FROM tbl WHERE` is SQL.
fn ends_a_from(after: &str) -> bool {
    after.is_empty() || after.starts_with("\\n") || after.starts_with("\\\"") || after.starts_with('"') || after.starts_with(" AS ") || after.starts_with(" as ")
}

/// The string literals of the `docker run`/`create` call whose argv starts
/// at byte `at` of `text`: from there to the close of the call that holds it.
fn argv_literals(text: &str, at: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur: Option<String> = Some(String::new());
    let mut chars = text[at..].chars();
    while let Some(c) = chars.next() {
        if let Some(s) = cur.as_mut() {
            match c {
                '\\' => {
                    s.push(c);
                    if let Some(n) = chars.next() {
                        s.push(n);
                    }
                }
                '"' => out.push(cur.take().unwrap()),
                _ => s.push(c),
            }
            continue;
        }
        match c {
            '"' => cur = Some(String::new()),
            '(' => depth += 1,
            ')' if depth == 0 => break,
            ')' => depth -= 1,
            ';' if depth == 0 => break,
            _ => {}
        }
    }
    out
}

fn violations(text: &str, seeded: &[String]) -> Vec<(usize, &'static str, String)> {
    let mut out = Vec::new();
    let mut offset = 0;
    for (n, line) in text.lines().enumerate() {
        let start = offset;
        offset += line.len() + 1;
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut found: Vec<(&'static str, String)> = Vec::new();
        let images = line.match_indices("image:").filter(|(i, _)| starts_a_key(line, *i)).map(|(i, k)| (i + k.len(), "compose `image:`"));
        let froms = line.match_indices("FROM ").filter(|(i, _)| starts_a_dockerfile_line(line, *i)).map(|(i, k)| (i + k.len(), "Dockerfile `FROM`"));
        for (at, shape) in images.chain(froms) {
            let rest = &line[at..];
            let value = field_value(rest);
            if value.is_empty() {
                match expression_operand(&text[start + at..]) {
                    Some(op) if op.starts_with("dna::TEST_") => {}
                    Some(op) => found.push((shape, format!("named by `{op}`, not a `dna::TEST_*` constant"))),
                    None => {}
                }
                continue;
            }
            if shape.starts_with("Dockerfile") {
                let after = rest.trim_start_matches(' ').trim_start_matches("\\\"").trim_start_matches('\'');
                if value.starts_with("--") || !ends_a_from(&after[value.len()..]) {
                    continue;
                }
            }
            if let Some(why) = refusal(value, seeded) {
                found.push((shape, format!("{value}: {why}")));
            }
        }
        for verb in ["docker\\nrun\\n", "docker\\ncreate\\n"] {
            for (i, _) in line.match_indices(verb) {
                for lit in argv_literals(text, start + i + verb.len()) {
                    for arg in lit.split("\\n").filter(|a| looks_like_reference(a)) {
                        if let Some(why) = refusal(arg, seeded) {
                            found.push(("`docker run` argv", format!("{arg}: {why}")));
                        }
                    }
                }
            }
        }
        for (shape, what) in found {
            out.push((n + 1, shape, what));
        }
    }
    out
}

fn collect(dir: &Path, under_tests: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "node_modules") {
                continue;
            }
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
fn no_fixture_pulls_from_docker_hub() {
    let seeded = seeded_from_mirror();
    let mut offenders = Vec::new();
    for (rel, text) in scanned_files() {
        if READS_ONLY.contains(&rel.as_str()) {
            continue;
        }
        for (line, shape, what) in violations(&text, &seeded) {
            offenders.push(format!("{rel}:{line}: {shape}: {what}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "a DNA fixture names the image it starts by the repository's mirror: a `dna::TEST_*` \
         constant in dna/core/workspace.hl, copied by .github/workflows/mirror-images.yml \
         ({} found):\n{}\n",
        offenders.len(),
        offenders.join("\n")
    );
}

/// What runs `docker`, or a verb that starts a body or brings up compose.
const STARTERS: &[&str] = &["docker\\n", "docker ", "dna\\ndev", "dna\\nrun\\n", "habitat\\nup", "habitat\\napply"];

/// The fixtures left alone because they only read what they name start
/// nothing: none of them runs `docker` or a verb that starts a body.
#[test]
fn a_reads_only_fixture_starts_nothing() {
    let files = scanned_files();
    for rel in READS_ONLY {
        let (_, text) = files.iter().find(|(r, _)| r == rel).unwrap_or_else(|| panic!("{rel} is listed in READS_ONLY but not scanned; drop it from the list"));
        let starts = text.lines().filter(|l| !l.trim_start().starts_with("//")).find(|l| STARTERS.iter().any(|s| l.contains(s)));
        assert!(
            starts.is_none(),
            "{rel} is listed in READS_ONLY but starts something ({}); drop it from the list so it is scanned like any fixture",
            starts.unwrap_or("").trim()
        );
    }
}

/// Each `dna::TEST_*` image is the mirror's, and `mirror-images.yml`
/// copies it there.
#[test]
fn every_test_image_is_mirrored() {
    let root = repo_root();
    let workspace = std::fs::read_to_string(root.join("dna/core/workspace.hl")).expect("workspace.hl");
    let workflow = std::fs::read_to_string(root.join(".github/workflows/mirror-images.yml")).expect("mirror-images.yml");
    let mut seen = 0;
    for line in workspace.lines().filter(|l| l.starts_with("const TEST_")) {
        let image = line.split('"').nth(1).unwrap_or_else(|| panic!("not a string constant: {line}"));
        let dest = image.strip_prefix("ghcr.io/hale-lang/").unwrap_or_else(|| panic!("{line}: not the mirror on ghcr.io/hale-lang"));
        assert!(
            workflow.lines().any(|l| l.trim_start().starts_with("copy ") && l.split_whitespace().nth(2) == Some(dest)),
            "{image} is not copied by mirror-images.yml (`copy docker.io/... {dest}`)"
        );
        seen += 1;
    }
    assert!(seen >= 2, "only {seen} `const TEST_` images in dna/core/workspace.hl; the scan is broken");
}

#[test]
fn the_scan_is_not_vacuous() {
    let files = scanned_files();
    assert!(files.len() >= 50, "only {} fixture files scanned; the directory walk is broken", files.len());
    assert!(seeded_from_mirror().iter().any(|s| s == "nats:2"), "tests.yml's mirror pairs were not read");
    let named = files.iter().filter(|(_, t)| t.contains("dna::TEST_BUSYBOX")).count();
    assert!(named >= 6, "only {named} fixtures name dna::TEST_BUSYBOX");
}

#[test]
fn the_matchers_see_what_they_name() {
    let seeded = vec!["nats:2".to_string(), "pgvector/pgvector:pg16".to_string()];
    let hit = |src: &str| !violations(src, &seeded).is_empty();
    // the shapes busybox had before GH #2141
    assert!(hit(r#"        "services:\n  probe:\n    image: busybox:1.36\n    command: [\"sleep\", \"3600\"]\n","#));
    assert!(hit(r#"    std::io::fs::write_file(src + "/docker/web.Dockerfile", "FROM busybox:1.36\nCOPY site /site\n")"#));
    assert!(hit("        \"FROM alpine:3.20\\nRUN apk add --no-cache openssh-server\\n\",\n"));
    assert!(hit(
        "        dna::run_tool(\n            \"docker\\nrun\\n-d\\n--name\\n\" + blocker + \"\\n--network\\n\" + net\n                + \"_default\\nbusybox:1.36\\nsleep\\n600\",\n        )\n"
    ));
    assert!(hit("let r = dna::run_tool(\"docker\\nrun\\n-d\\n-p\\n127.0.0.1::8080\\nbusybox:1.36\\nsh\");"));
    // Docker Hub spelled out, untagged, quoted, or behind an expression
    assert!(hit(r#""services:\n  x:\n    image: docker.io/library/busybox:1.36\n""#));
    assert!(hit(r#""services:\n  x:\n    image: busybox\n""#));
    assert!(hit(r#""services:\n  x:\n    image: \"redis:7\"\n""#));
    assert!(hit(r#""services:\n  x:\n    image: " + some_image() + "\n""#));
    assert!(hit("        \"services:\\n  x:\\n    image: \"\n            + some_image()\n            + \"\\n\","));
    assert!(hit("  image: postgres:17\n"));
    // what is fine: the mirror, a fixture's own build, a seeded name,
    // another registry, a constant, SQL, the JSON of `compose config`
    assert!(!hit(r#""services:\n  probe:\n    image: " + dna::TEST_BUSYBOX + "\n""#));
    assert!(!hit(r#""FROM " + dna::TEST_ALPINE + "\nRUN apk add x\n""#));
    assert!(!hit("        \"services:\\n  x:\\n    image: \"\n            + dna::TEST_BUSYBOX\n            + \"\\n\","));
    assert!(!hit(r#""services:\n  x:\n    image: ghcr.io/hale-lang/ci-busybox:1.36\n""#));
    assert!(!hit(r#""services:\n  sshd:\n    build: .\n    image: hale-dna-test-sshd:1\n""#));
    assert!(!hit(r#""services:\n  probe:\n    image: pgvector/pgvector:pg16\n""#));
    assert!(!hit("let r = dna::run_tool(\"docker\\nrun\\n-d\\n--label\\nhale.dna.body=\" + MARK + \"\\nnats:2\");"));
    assert!(!hit(r#""services:\n  x:\n    image: ghcr.io/acme/synth:1.4.0\n""#));
    assert!(!hit(r#"let rows = c.query_one("SELECT n FROM kept");"#));
    assert!(!hit(r#""SELECT count(*) FROM " + dna::schema_for(record)"#));
    assert!(!hit(r#"+ "\"nats\": {\"image\": \"nats:2.11-alpine\"},""#));
    assert!(!hit(r#"let ok = dna::run_tool("docker\nrun\n-d\n" + dna::TEST_BUSYBOX + "\nsleep\n600");"#));
    assert!(!hit("// image: busybox:1.36 was the old name"));
}
