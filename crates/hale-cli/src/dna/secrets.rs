//! The secrets a seed declares in its manifest, which the habitat keeps in
//! the organism's vault and hands to the programs and host commands that
//! read them:
//!
//! ```text
//! [dna.secrets.operator-token]
//! kind    = "drawn"                  # drawn: the habitat draws it; slot: a person fills it
//! readers = ["api", "host.node"]     # its programs (by directory) and host commands (host.<command>)
//! person  = "none"                   # read | write | none (a slot's default write, a drawn one's none)
//!
//! [dna.secrets.model-token]
//! kind    = "slot"
//! family  = true                     # several, each filled under a label: `secret set model-token --member work`
//! readers = ["worker"]
//! ```
//!
//! Each is the vault entry `secret-<org>-<name>` (`<org>` the record's
//! schema, `dna_<record id>`, as the nerves' entries are named); a family's
//! member is `secret-<org>-<name>--<label>`, and how many members it has is
//! the deployment's, never the seed's. This is
//! the one reader of the declaration: the DNA's own code asks it through
//! `hale dna declared-secrets <dir>` (`lines`), and `hale habitat organism
//! place` reads it from the source it fetched.

use std::collections::BTreeMap;

/// Who made the value: the habitat (`drawn`) or a person (`slot`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Drawn,
    Slot,
}

/// What a person may do with it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Person {
    /// `hale dna secret show` reveals it (and a slot's person sets it)
    Read,
    /// a person sets it and never reads it back
    Write,
    /// no person touches it
    None,
}

/// One secret the seed declares.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Declared {
    pub kind: Kind,
    /// its readers as declared: a program's directory, or `host.<command>`
    pub readers: Vec<String>,
    pub person: Person,
    /// a slot of several values, each filled under its own label
    pub family: bool,
}

impl Declared {
    /// The programs that read it, by directory name.
    pub(crate) fn programs(&self) -> Vec<&str> {
        self.readers.iter().filter(|r| !r.starts_with("host.")).map(String::as_str).collect()
    }
    /// The host commands that read it, by the command's name.
    pub(crate) fn hosts(&self) -> Vec<&str> {
        self.readers.iter().filter_map(|r| r.strip_prefix("host.")).collect()
    }
}

/// A secret's name: lowercase letters, digits and `-`, a letter first, at
/// most 32 characters, a `-` never last nor doubled (a family member's
/// entry is its name, `--` and its label).
fn name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.starts_with(|c: char| c.is_ascii_lowercase()) && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !name.ends_with('-') && !name.contains("--")
}

/// A family member's label: `^[a-z0-9][a-z0-9-]{0,62}$`, 1 to 63
/// characters, a lowercase letter or a digit first.
pub(crate) fn label_ok(label: &str) -> bool {
    !label.is_empty() && label.len() <= 63 && label.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit()) && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A program reader: a directory of the seed's own, named as it is.
fn program_ok(dir: &str) -> bool {
    !dir.is_empty() && dir != "." && dir != ".." && dir.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// The secrets a manifest declares (`[dna.secrets.<name>]`), checked.
pub(crate) fn declared(manifest: &str) -> Result<BTreeMap<String, Declared>, String> {
    let doc: toml::Table = toml::from_str(manifest).map_err(|e| format!("hale.toml: {e}"))?;
    let mut out = BTreeMap::new();
    let dna = doc.get("dna");
    let Some(secrets) = dna.and_then(|d| d.get("secrets")) else { return Ok(out) };
    let secrets = secrets.as_table().ok_or("[dna.secrets] is a table of secrets, one `[dna.secrets.<name>]` each")?;
    // the host commands it declares, by name (`[dna.host.<command>]`)
    let hosts: Vec<String> = dna.and_then(|d| d.get("host")).and_then(|h| h.as_table()).map(|t| t.keys().cloned().collect()).unwrap_or_default();
    for (name, t) in secrets {
        if !name_ok(name) {
            return Err(format!("[dna.secrets.{name}]: a secret's name is lowercase letters, digits and `-`, a letter first, at most 32 characters, with no `-` last or doubled"));
        }
        let t = t.as_table().ok_or_else(|| format!("[dna.secrets.{name}] is a table"))?;
        if let Some(k) = t.keys().find(|k| !["kind", "readers", "person", "family"].contains(&k.as_str())) {
            return Err(format!("[dna.secrets.{name}] has no `{k}` (kind, readers, person, family)"));
        }
        let word = |k: &str| -> Result<Option<&str>, String> { t.get(k).map(|v| v.as_str().ok_or_else(|| format!("[dna.secrets.{name}] {k} is a string"))).transpose() };
        let kind = match word("kind")? {
            Some("drawn") => Kind::Drawn,
            Some("slot") => Kind::Slot,
            Some(other) => return Err(format!("[dna.secrets.{name}] kind `{other}`: drawn (the habitat draws it) or slot (a person fills it)")),
            None => return Err(format!("[dna.secrets.{name}] names its kind: drawn or slot")),
        };
        let person = match word("person")? {
            Some("read") => Person::Read,
            Some("write") => Person::Write,
            Some("none") => Person::None,
            Some(other) => return Err(format!("[dna.secrets.{name}] person `{other}`: read, write or none")),
            None if kind == Kind::Drawn => Person::None,
            None => Person::Write,
        };
        if kind == Kind::Drawn && person == Person::Write {
            return Err(format!("[dna.secrets.{name}] is drawn: the habitat draws it and a person never sets it (person = \"read\" or \"none\")"));
        }
        let family = match t.get("family") {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| format!("[dna.secrets.{name}] family is true or false"))?,
        };
        if family && kind == Kind::Drawn {
            return Err(format!("[dna.secrets.{name}] is drawn, and a family is a slot's: its members are a person's, each filled under a label"));
        }
        if kind == Kind::Slot && person == Person::None {
            return Err(format!("[dna.secrets.{name}] is a slot, which a person fills (person = \"write\" or \"read\")"));
        }
        let readers: Vec<String> = match t.get("readers") {
            None => Vec::new(),
            Some(v) => v.as_array().ok_or_else(|| format!("[dna.secrets.{name}] readers is a list of strings"))?.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| format!("[dna.secrets.{name}] readers is a list of strings"))).collect::<Result<_, _>>()?,
        };
        for r in &readers {
            match r.strip_prefix("host.") {
                Some(cmd) if !hosts.iter().any(|h| h == cmd) => {
                    return Err(format!("[dna.secrets.{name}] reader `{r}`: the seed declares no host command `{cmd}` ([dna.host.{cmd}])"));
                }
                Some(_) => {}
                None if !program_ok(r) => {
                    return Err(format!("[dna.secrets.{name}] reader `{r}`: a program by its directory's name, or host.<command>"));
                }
                None => {}
            }
        }
        out.insert(name.clone(), Declared { kind, readers, person, family });
    }
    Ok(out)
}

/// The record's schema, as the DNA names it (`dna::schema_for`): `dna_`
/// and the identity lowercased, anything but a letter or digit folded to
/// `_`, at most 56 of its characters.
pub(crate) fn schema_for(record_id: &str) -> String {
    if record_id.is_empty() {
        return "dna_unscoped".to_string();
    }
    let folded: String = record_id.chars().take(56).map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    format!("dna_{folded}")
}

/// The vault entry that holds the organism's secret `name`.
pub(crate) fn vault_entry(record_id: &str, name: &str) -> String {
    format!("secret-{}-{name}", schema_for(record_id))
}

/// The vault entry of the family `name`'s member `label`: its name, `--`
/// and the label, which no other secret's entry can be (a name never holds
/// `--` nor ends in `-`). The DNA's code names members
/// (`dna::family_member_vault`), and a host command's are read by their
/// common prefix (`hosted::members`): this pins the spelling they share.
#[cfg(test)]
pub(crate) fn member_entry(record_id: &str, name: &str, label: &str) -> String {
    format!("{}--{label}", vault_entry(record_id, name))
}

/// The variable a reader is handed the entry's name in (a family's: its
/// members' entries, one a line, in their labels' order):
/// `HALE_SECRET_<NAME>`, upper-cased, `-` as `_`.
pub(crate) fn env_var(name: &str) -> String {
    format!("HALE_SECRET_{}", name.to_ascii_uppercase().replace('-', "_"))
}

/// The declaration as `hale dna declared-secrets` prints it for the DNA's
/// code, a secret a line: `<name>\t<drawn|slot>\t<read|write|none>\t<the
/// programs>\t<the host commands>\t<family or nothing>`, each list
/// space-separated.
pub(crate) fn lines(declared: &BTreeMap<String, Declared>) -> String {
    let mut out = String::new();
    for (name, d) in declared {
        let kind = if d.kind == Kind::Drawn { "drawn" } else { "slot" };
        let person = match d.person {
            Person::Read => "read",
            Person::Write => "write",
            Person::None => "none",
        };
        out.push_str(&format!("{name}\t{kind}\t{person}\t{}\t{}\t{}\n", d.programs().join(" "), d.hosts().join(" "), if d.family { "family" } else { "" }));
    }
    out
}

/// `hale dna declared-secrets [dir]`: the secrets `<dir>/hale.toml`
/// declares (`lines`); nothing when it has no manifest or declares none,
/// exit 1 and why when the declaration is refused.
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

[dna.host.node]
install = ["hale", "run", "node", "install"]
uninstall = ["hale", "run", "node", "uninstall"]

[dna.secrets.operator-token]
kind = "drawn"
readers = ["api", "host.node"]

[dna.secrets.upstream-key]
kind = "slot"
readers = ["api"]

[dna.secrets.dashboard]
kind = "drawn"
readers = ["web"]
person = "read"

[dna.secrets.model-token]
kind = "slot"
family = true
readers = ["worker", "host.node"]
"#;

    #[test]
    fn a_declaration_is_read_with_its_defaults() {
        let d = declared(SEED).unwrap();
        let op = &d["operator-token"];
        assert_eq!((op.kind, op.person), (Kind::Drawn, Person::None));
        assert_eq!((op.programs(), op.hosts()), (vec!["api"], vec!["node"]));
        assert_eq!((d["upstream-key"].kind, d["upstream-key"].person), (Kind::Slot, Person::Write));
        assert_eq!(d["dashboard"].person, Person::Read);
        assert!(declared("[dna]\nneeds = []\n").unwrap().is_empty());
        assert!(d["model-token"].family && !d["upstream-key"].family);
        assert_eq!(lines(&d), "dashboard\tdrawn\tread\tweb\t\t\nmodel-token\tslot\twrite\tworker\tnode\tfamily\noperator-token\tdrawn\tnone\tapi\tnode\t\nupstream-key\tslot\twrite\tapi\t\t\n");
        // a slot may be read back, and a drawn secret has no readers yet
        assert!(declared("[dna.secrets.k]\nkind = \"slot\"\nperson = \"read\"\n").is_ok());
        assert!(declared("[dna.secrets.k]\nkind = \"drawn\"\n").unwrap()["k"].readers.is_empty());
    }

    #[test]
    fn each_refusal_says_what_is_wrong() {
        let refuse = |toml: &str| declared(toml).unwrap_err();
        assert!(refuse("[dna.secrets.Api_Key]\nkind = \"slot\"\n").contains("lowercase letters"));
        assert!(refuse("[dna.secrets.1key]\nkind = \"slot\"\n").contains("a letter first"));
        assert!(refuse(&format!("[dna.secrets.{}]\nkind = \"slot\"\n", "k".repeat(33))).contains("at most 32"));
        assert!(refuse("[dna.secrets.k]\nkind = \"slot\"\ncolour = \"red\"\n").contains("has no `colour`"));
        assert!(refuse("[dna.secrets.k]\nkind = \"minted\"\n").contains("kind `minted`"));
        assert!(refuse("[dna.secrets.k]\nreaders = []\n").contains("names its kind"));
        assert!(refuse("[dna.secrets.k]\nkind = \"slot\"\nperson = \"admin\"\n").contains("person `admin`"));
        assert!(refuse("[dna.secrets.k]\nkind = \"drawn\"\nperson = \"write\"\n").contains("never sets it"));
        assert!(refuse("[dna.secrets.k]\nkind = \"slot\"\nperson = \"none\"\n").contains("a person fills"));
        assert!(refuse("[dna.secrets.k]\nkind = \"drawn\"\nreaders = [\"host.node\"]\n").contains("no host command `node`"));
        assert!(refuse("[dna.secrets.k]\nkind = \"drawn\"\nreaders = [\"../etc\"]\n").contains("reader `../etc`"));
        assert!(refuse("[dna.secrets.k]\nkind = \"drawn\"\nreaders = \"api\"\n").contains("list of strings"));
        assert!(refuse("[dna.secrets.k]\nkind = 1\n").contains("kind is a string"));
        assert!(refuse("[dna]\nsecrets = 1\n").contains("table of secrets"));
        // a family is a slot's, and true or false
        assert!(refuse("[dna.secrets.k]\nkind = \"drawn\"\nfamily = true\n").contains("a family is a slot's"));
        assert!(refuse("[dna.secrets.k]\nkind = \"slot\"\nfamily = \"yes\"\n").contains("family is true or false"));
        assert!(declared("[dna.secrets.k]\nkind = \"drawn\"\nfamily = false\n").is_ok());
        // a name a member's entry could be mistaken for
        assert!(refuse("[dna.secrets.a--b]\nkind = \"slot\"\n").contains("doubled"));
        assert!(refuse("[dna.secrets.a-]\nkind = \"slot\"\n").contains("doubled"));
    }

    #[test]
    fn the_entry_and_the_variable_are_named_for_the_organism_and_the_secret() {
        let id = "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(schema_for(id), format!("dna_{}", &id.to_ascii_lowercase()[..56]));
        assert_eq!(vault_entry(id, "operator-token"), format!("secret-dna_{}-operator-token", &id.to_ascii_lowercase()[..56]));
        assert_eq!(schema_for(""), "dna_unscoped");
        // spelled from its prefix: a variable named per secret is no knob
        // of the toolchain's (env_knobs_are_documented reads literals)
        let var = |rest: &str| format!("HALE_SECRET_{rest}");
        assert_eq!(env_var("operator-token"), var("OPERATOR_TOKEN"));
        assert_eq!(env_var("k2"), var("K2"));
        assert_eq!(member_entry(id, "claude-token", "work"), format!("secret-dna_{}-claude-token--work", &id.to_ascii_lowercase()[..56]));
    }

    #[test]
    fn a_members_label_is_one_word_of_lowercase_letters_digits_and_dashes() {
        for good in ["work", "personal-1", "0", &"a".repeat(63)] {
            assert!(label_ok(good), "{good}");
        }
        for bad in ["-x", "Work", "a_b", "", &"a".repeat(64), "a.b", "a/b"] {
            assert!(!label_ok(bad), "{bad}");
        }
    }
}
