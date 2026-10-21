//! A face, declared in the genome (Phase 3 DNA item 6).
//!
//! A face is a role an application fills: the browser's door to the
//! organism. A genome names its faces in its manifest, beside `needs`
//! (what it asks of the habitat) and `fleet`:
//!
//! ```text
//! [dna.faces.admin]
//! application = "standard"            # the toolchain's own; the default
//! consumes = ["HeadCommands"]         # the surfaces it calls, by name
//! acts_as = ["owner"]                 # the roles it acts under
//! url = "http://127.0.0.1:8792"       # where it is reached
//! ```
//!
//! A genome that names none has one face, the standard face
//! (`dna/face/standard`), grafted like the habitat genome and the packs:
//! the toolchain ships it. An unnamed face is satisfied by it, and an
//! `application` left out of a row is `standard`. This slice starts the
//! standard face; a row that names another application, and more than one
//! row, are parsed and refused by name until the faces that mount under a
//! navigation land.

/// The application the toolchain ships.
pub(crate) const STANDARD: &str = "standard";

/// One face row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Face {
    pub name: String,
    pub application: String,
    pub consumes: Vec<String>,
    pub acts_as: Vec<String>,
    pub url: Option<String>,
}

impl Face {
    /// The face a genome that names none has.
    fn unnamed() -> Face {
        Face {
            name: String::new(),
            application: STANDARD.to_string(),
            consumes: Vec::new(),
            acts_as: Vec::new(),
            url: None,
        }
    }

    /// The loopback port `url` names, when it names one.
    pub(crate) fn port(&self) -> Option<u16> {
        let rest = self.url.as_deref()?.strip_prefix("http://127.0.0.1:")?;
        rest.trim_end_matches('/')
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
    }
}

fn word(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
}

fn names(row: &str, key: &str, v: Option<&toml::Value>) -> Result<Vec<String>, String> {
    match v {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|x| match x.as_str() {
                Some(s) if word(s) => Ok(s.to_string()),
                _ => Err(format!("[dna.faces.{row}] {key} holds {x}, not a name")),
            })
            .collect(),
        Some(other) => Err(format!(
            "[dna.faces.{row}] {key} is {other}, not a list of names"
        )),
    }
}

/// The faces the genome's manifest declares, in name order; the unnamed
/// standard face when it declares none. A row with a key it does not know,
/// a name or value that is not one, or a url that is not a loopback URL is
/// refused by name.
pub(crate) fn faces_of(manifest_text: Option<&str>) -> Result<Vec<Face>, String> {
    let Some(text) = manifest_text else {
        return Ok(vec![Face::unnamed()]);
    };
    let table: toml::Table = text
        .parse()
        .map_err(|e| format!("hale.toml does not parse: {e}"))?;
    let rows = match table.get("dna").and_then(|d| d.get("faces")) {
        None => return Ok(vec![Face::unnamed()]),
        Some(toml::Value::Table(t)) if !t.is_empty() => t,
        Some(other) => {
            return Err(format!(
                "[dna.faces] is {other}, not a table of named faces"
            ))
        }
    };
    let mut faces = Vec::new();
    for (name, row) in rows {
        if !name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "[dna.faces.{name}]: a face is named in lower case (`admin`, `product`)"
            ));
        }
        let toml::Value::Table(row) = row else {
            return Err(format!("[dna.faces.{name}] is {row}, not a table"));
        };
        if let Some(k) = row
            .keys()
            .find(|k| !matches!(k.as_str(), "application" | "consumes" | "acts_as" | "url"))
        {
            return Err(format!("[dna.faces.{name}] has `{k}`, which a face row does not (application, consumes, acts_as, url)"));
        }
        let application = match row.get("application") {
            None => STANDARD.to_string(),
            Some(toml::Value::String(s)) if !s.is_empty() && !s.contains(['\n', '\r']) => s.clone(),
            Some(other) => {
                return Err(format!(
                    "[dna.faces.{name}] application is {other}, not the application's name or path"
                ))
            }
        };
        let url = match row.get("url") {
            None => None,
            Some(toml::Value::String(s)) if s.starts_with("http://127.0.0.1:") => Some(s.clone()),
            Some(other) => {
                return Err(format!(
                "[dna.faces.{name}] url is {other}, not a loopback URL (http://127.0.0.1:<port>)"
            ))
            }
        };
        let face = Face {
            name: name.clone(),
            application,
            consumes: names(name, "consumes", row.get("consumes"))?,
            acts_as: names(name, "acts_as", row.get("acts_as"))?,
            url,
        };
        if face.url.is_some() && face.port().is_none() {
            return Err(format!("[dna.faces.{name}] url names no port"));
        }
        faces.push(face);
    }
    Ok(faces)
}

/// The face `hale dna head` starts: the one this slice can, or why not.
pub(crate) fn startable(faces: &[Face]) -> Result<&Face, String> {
    if faces.len() > 1 {
        let named: Vec<&str> = faces.iter().map(|f| f.name.as_str()).collect();
        return Err(format!("the genome names {} faces ({}); the head starts one face, and faces mounted under a navigation are not built yet", faces.len(), named.join(", ")));
    }
    let face = &faces[0];
    if face.application != STANDARD {
        return Err(format!("the face `{}` is filled by `{}`; only the toolchain's `standard` face application starts yet", face.name, face.application));
    }
    Ok(face)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_genome_naming_none_has_the_standard_face() {
        for text in [None, Some(""), Some("[dna]\nrecord = \"x\"\n")] {
            let faces = faces_of(text).unwrap();
            assert_eq!(faces, vec![Face::unnamed()]);
            assert_eq!(startable(&faces).unwrap().application, STANDARD);
        }
    }

    #[test]
    fn a_row_names_what_fills_the_role() {
        let text = "[dna.faces.admin]\nconsumes = [\"HeadCommands\"]\nacts_as = [\"owner\"]\nurl = \"http://127.0.0.1:9100\"\n";
        let faces = faces_of(Some(text)).unwrap();
        assert_eq!(faces.len(), 1);
        let f = &faces[0];
        assert_eq!(
            (f.name.as_str(), f.application.as_str()),
            ("admin", STANDARD)
        );
        assert_eq!(
            (f.consumes.clone(), f.acts_as.clone(), f.port()),
            (
                vec!["HeadCommands".to_string()],
                vec!["owner".to_string()],
                Some(9100)
            )
        );
        assert!(startable(&faces).is_ok());
    }

    #[test]
    fn what_is_not_a_face_row_is_refused_by_name() {
        for (text, needle) in [
            ("[dna.faces.Admin]\n", "lower case"),
            ("[dna.faces.admin]\ncolour = \"red\"\n", "`colour`"),
            (
                "[dna.faces.admin]\nurl = \"https://example.invalid\"\n",
                "loopback URL",
            ),
            (
                "[dna.faces.admin]\nurl = \"http://127.0.0.1:x\"\n",
                "no port",
            ),
            (
                "[dna.faces.admin]\nconsumes = \"HeadCommands\"\n",
                "not a list",
            ),
            (
                "[dna.faces.admin]\nacts_as = [\"two words\"]\n",
                "not a name",
            ),
            ("[dna]\nfaces = \"admin\"\n", "not a table"),
        ] {
            let e = faces_of(Some(text)).unwrap_err();
            assert!(e.contains(needle), "{text:?}: {e}");
        }
    }

    #[test]
    fn this_slice_starts_the_standard_face_alone() {
        let own = faces_of(Some("[dna.faces.admin]\napplication = \"ui/admin\"\n")).unwrap();
        assert!(startable(&own)
            .unwrap_err()
            .contains("only the toolchain's `standard`"));
        let two = faces_of(Some("[dna.faces.admin]\n[dna.faces.product]\n")).unwrap();
        assert!(startable(&two).unwrap_err().contains("admin, product"));
    }
}
