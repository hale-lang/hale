//! The web UIs a seed declares in its manifest, which the habitat's home
//! screen lists beside each organism's face:
//!
//! ```text
//! [dna.ui.admin]
//! label   = "voice admin"      # what the home screen calls it
//! service = "api"              # the compose service that serves it
//! path    = "/"                # where under that service's address (default "/")
//! login   = "operator-token"   # how to sign in: a declared secret a person may read
//! ```
//!
//! A UI is linked at the address its organism's services allocation
//! publishes for `service` (the `links` of `Commands::allocations`), with
//! `path` after it; `login` names the secret a person reads to sign in
//! (`hale dna secret show <login>`), never its value. This is the one
//! reader of the declaration: `hale habitat graft` refuses a manifest whose
//! UIs are refused, and the habitat's head asks it through `hale dna
//! declared-uis <dir>` (`lines`).

use std::collections::BTreeMap;

use super::secrets::{self, Person};

/// One UI the seed declares.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Declared {
    pub label: String,
    /// the compose service that serves it
    pub service: String,
    /// where under the service's address, `/` first
    pub path: String,
    /// the declared secret a person reads to sign in ("" for none)
    pub login: String,
}

/// A UI's name: lowercase letters, digits and `-`, a letter first, at most
/// 32 characters, no `-` last.
fn name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.starts_with(|c: char| c.is_ascii_lowercase()) && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !name.ends_with('-')
}

/// A compose service's name, as compose spells one.
fn service_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && s.starts_with(|c: char| c.is_ascii_alphanumeric()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// The UIs a manifest declares (`[dna.ui.<name>]`), checked.
pub(crate) fn declared(manifest: &str) -> Result<BTreeMap<String, Declared>, String> {
    let doc: toml::Table = toml::from_str(manifest).map_err(|e| format!("hale.toml: {e}"))?;
    let mut out = BTreeMap::new();
    let Some(uis) = doc.get("dna").and_then(|d| d.get("ui")) else { return Ok(out) };
    let uis = uis.as_table().ok_or("[dna.ui] is a table of UIs, one `[dna.ui.<name>]` each")?;
    for (name, t) in uis {
        if !name_ok(name) {
            return Err(format!("[dna.ui.{name}]: a UI's name is lowercase letters, digits and `-`, a letter first, at most 32 characters, with no `-` last"));
        }
        let t = t.as_table().ok_or_else(|| format!("[dna.ui.{name}] is a table"))?;
        if let Some(k) = t.keys().find(|k| !["label", "service", "path", "login"].contains(&k.as_str())) {
            return Err(format!("[dna.ui.{name}] has no `{k}` (label, service, path, login)"));
        }
        let word = |k: &str| -> Result<Option<&str>, String> { t.get(k).map(|v| v.as_str().ok_or_else(|| format!("[dna.ui.{name}] {k} is a string"))).transpose() };
        let label = word("label")?.map(str::trim).filter(|l| !l.is_empty()).ok_or_else(|| format!("[dna.ui.{name}] names its label: what the habitat's home screen calls it"))?;
        if label.contains(['\t', '\n', '\r']) || label.chars().count() > 80 {
            return Err(format!("[dna.ui.{name}] label: one line of at most 80 characters"));
        }
        let service = word("service")?.ok_or_else(|| format!("[dna.ui.{name}] names its service: the compose service that serves it"))?;
        if !service_ok(service) {
            return Err(format!("[dna.ui.{name}] service `{service}`: a compose service's name (letters, digits, `-`, `_`, `.`)"));
        }
        let path = word("path")?.unwrap_or("/");
        if !path.starts_with('/') || path.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!("[dna.ui.{name}] path `{path}`: a path under the service's address, `/` first, with no space"));
        }
        let login = word("login")?.unwrap_or("");
        if !login.is_empty() {
            let secrets = secrets::declared(manifest).map_err(|why| format!("[dna.ui.{name}] login `{login}`: the seed's secrets are refused: {why}"))?;
            match secrets.get(login) {
                None => return Err(format!("[dna.ui.{name}] login `{login}`: the seed declares no secret `{login}` ([dna.secrets.{login}])")),
                Some(s) if s.person != Person::Read => {
                    return Err(format!("[dna.ui.{name}] login `{login}`: a person signs in with a secret they may read, and `{login}` is not person = \"read\""));
                }
                Some(s) if s.family => {
                    return Err(format!("[dna.ui.{name}] login `{login}`: a family holds several values, and a sign-in reads one"));
                }
                Some(_) => {}
            }
        }
        out.insert(name.clone(), Declared { label: label.to_string(), service: service.to_string(), path: path.to_string(), login: login.to_string() });
    }
    Ok(out)
}

/// The declaration as `hale dna declared-uis` prints it for the DNA's code,
/// a UI a line: `<name>\t<label>\t<service>\t<path>\t<login or nothing>`.
pub(crate) fn lines(declared: &BTreeMap<String, Declared>) -> String {
    declared.iter().map(|(name, d)| format!("{name}\t{}\t{}\t{}\t{}\n", d.label, d.service, d.path, d.login)).collect()
}

/// `hale dna declared-uis [dir]`: the UIs `<dir>/hale.toml` declares
/// (`lines`); nothing when it has no manifest or declares none, exit 1 and
/// why when the declaration is refused.
pub(crate) fn declared_cmd(args: &[String]) -> std::process::ExitCode {
    let dir = args.first().map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("."));
    let path = dir.join("hale.toml");
    let Ok(text) = std::fs::read_to_string(&path) else { return std::process::ExitCode::SUCCESS };
    match declared(&text) {
        Ok(d) => {
            print!("{}", lines(&d));
            std::process::ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("{}: {why}", path.display());
            std::process::ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = r#"
[dna]
needs = ["memory"]

[dna.secrets.operator-token]
kind = "drawn"
readers = ["api"]
person = "read"

[dna.secrets.upstream-key]
kind = "slot"
readers = ["api"]

[dna.ui.admin]
label = "voice admin"
service = "api"
login = "operator-token"

[dna.ui.metrics]
label = "  metrics  "
service = "web_ui"
path = "/dashboards/main?refresh=5s"
"#;

    #[test]
    fn a_declaration_is_read_with_its_defaults() {
        let d = declared(SEED).unwrap();
        assert_eq!(d["admin"], Declared { label: "voice admin".into(), service: "api".into(), path: "/".into(), login: "operator-token".into() });
        assert_eq!(d["metrics"].label, "metrics");
        assert_eq!(d["metrics"].login, "");
        assert_eq!(lines(&d), "admin\tvoice admin\tapi\t/\toperator-token\nmetrics\tmetrics\tweb_ui\t/dashboards/main?refresh=5s\t\n");
        assert!(declared("[dna]\nneeds = []\n").unwrap().is_empty());
        assert!(declared("").unwrap().is_empty());
    }

    #[test]
    fn each_refusal_says_what_is_wrong() {
        let ui = |body: &str| format!("[dna.secrets.operator-token]\nkind = \"drawn\"\nperson = \"read\"\n\n[dna.secrets.upstream-key]\nkind = \"slot\"\n\n[dna.secrets.models]\nkind = \"slot\"\nperson = \"read\"\nfamily = true\n\n{body}");
        let refuse = |body: &str| declared(&ui(body)).unwrap_err();
        assert!(declared(&ui("[dna.ui.a]\nlabel = \"A\"\nservice = \"api\"\nlogin = \"operator-token\"\n")).is_ok());
        assert!(refuse("[dna.ui.Admin]\nlabel = \"A\"\nservice = \"api\"\n").contains("lowercase letters"));
        assert!(refuse("[dna.ui.1a]\nlabel = \"A\"\nservice = \"api\"\n").contains("a letter first"));
        assert!(refuse("[dna.ui.a-]\nlabel = \"A\"\nservice = \"api\"\n").contains("no `-` last"));
        assert!(refuse(&format!("[dna.ui.{}]\nlabel = \"A\"\nservice = \"api\"\n", "a".repeat(33))).contains("at most 32"));
        assert!(refuse("[dna.ui.a]\nlabel = \"A\"\nservice = \"api\"\nport = 80\n").contains("has no `port`"));
        assert!(refuse("[dna.ui.a]\nservice = \"api\"\n").contains("names its label"));
        assert!(refuse("[dna.ui.a]\nlabel = \"  \"\nservice = \"api\"\n").contains("names its label"));
        assert!(refuse("[dna.ui.a]\nlabel = \"a\\tb\"\nservice = \"api\"\n").contains("one line"));
        assert!(refuse("[dna.ui.a]\nlabel = \"A\"\n").contains("names its service"));
        assert!(refuse("[dna.ui.a]\nlabel = \"A\"\nservice = \"my api\"\n").contains("service `my api`"));
        assert!(refuse("[dna.ui.a]\nlabel = 1\nservice = \"api\"\n").contains("label is a string"));
        assert!(refuse("[dna.ui.a]\nlabel = \"A\"\nservice = \"api\"\npath = \"admin\"\n").contains("`/` first"));
        assert!(refuse("[dna.ui.a]\nlabel = \"A\"\nservice = \"api\"\npath = \"/a b\"\n").contains("no space"));
        assert!(refuse("[dna.ui.a]\nlabel = \"A\"\nservice = \"api\"\nlogin = \"nope\"\n").contains("declares no secret `nope`"));
        assert!(refuse("[dna.ui.a]\nlabel = \"A\"\nservice = \"api\"\nlogin = \"upstream-key\"\n").contains("not person = \"read\""));
        assert!(refuse("[dna.ui.a]\nlabel = \"A\"\nservice = \"api\"\nlogin = \"models\"\n").contains("a family"));
        assert!(refuse("[dna]\nui = 1\n").contains("table of UIs"));
        assert!(refuse("[dna.ui]\na = 1\n").contains("[dna.ui.a] is a table"));
        // a login over refused secrets says so
        assert!(declared("[dna.secrets.k]\nkind = \"minted\"\n\n[dna.ui.a]\nlabel = \"A\"\nservice = \"api\"\nlogin = \"k\"\n").unwrap_err().contains("the seed's secrets are refused"));
    }
}
