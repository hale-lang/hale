//! Field defaults across the api codec, as a generated differential
//! (spec/api.md § Codecs, spec/types.md § Locus params). A default reads
//! only its own fields and constants, and the check refuses one that reads
//! the scope of the literal or call that leaves it, so every default a
//! program can hold is one a codec can build. For each default shape below:
//!
//! - a shape the rule refuses: the program that builds the record
//!   (`Outer { }`) is refused by the check, with the rule's words;
//! - any other: that program builds, then the same program calling
//!   `Outer::api_decode` builds too, and the description's `required` lists
//!   the field exactly when the decoder refuses `{}`, as the rule says (a
//!   field with a default is optional, a fixed array always sent).
//!
//! The shapes are each way a default names something: a literal, a
//! constant (and one a caller's binding shadows), a module fn, a local
//! value, a local fn value, a record literal by its name, by an alias and
//! through an import, nested literals, fields whose `bounded` or
//! fixed-array type is an alias, a map, fn calls that leave out a parameter
//! (by name, through an import, generic, and one whose default calls
//! another), an enum variant through an alias, and a chain of defaults
//! that reaches itself. The reviewed programs that a closed-default rule
//! got wrong are among them (the local read, the nested literal, the alias,
//! the import, the fn value, the aliased `bounded` field, the omitted
//! argument's default).

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.env("HALE_SKIP_STALE_CHECK", "1");
    c
}

fn unique_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hale_api_default_matrix_{}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        tag
    ));
    std::fs::create_dir_all(dir.join("l")).expect("temp dir");
    dir
}

/// What the rule answers for a shape: the check refuses it; or the field
/// is required on the wire, or the codec builds its default; or the
/// control may not build today (a record that holds itself, a generic call
/// that leaves a parameter), and when it does, the field must be required
/// or not as `required` says.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Want {
    Refused,
    Required,
    Built,
    MayNotBuild { required: bool },
}

/// The rule's words, in every refusal of a default's scope.
const RULE: &str = "a default reads only its own fields and constants";

struct Cell {
    name: &'static str,
    /// The library `l`'s source, imported as `l`.
    lib: &'static str,
    decls: &'static str,
    ty: &'static str,
    default: &'static str,
    /// Bindings of `main`'s scope, the scope `Outer { }` evaluates in.
    locals: &'static str,
    want: Want,
}

const INNER: &str = "type Inner { b: Int = a + 1; }\n";
const CLOSED: &str = "type Closed { c: Int = 3; }\n";

fn cells() -> Vec<Cell> {
    let c = |name, decls, ty, default, want| Cell { name, lib: "", decls, ty, default, locals: "", want };
    vec![
        c("literal", "", "Int", "3", Want::Built),
        c("const", "const K: Int = 4;\n", "Int", "K + 1", Want::Built),
        // a caller binding that would shadow the constant, where the literal
        // leaves the default
        Cell { locals: "let K = 0;\n", ..c("const_shadowed", "const K: Int = 4;\n", "Int", "K + 1", Want::Refused) },
        c("module_fn", "fn value() -> Int { return 3; }\n", "Int", "value() + 1", Want::Built),
        c("local_value", "", "Int", "a + 1", Want::Refused),
        Cell { locals: "let compute = value;\n", ..c("local_fn", "fn value() -> Int { return 3; }\n", "Int", "compute()", Want::Refused) },
        c("record_by_name", INNER, "Inner", "Inner { }", Want::Refused),
        c("record_by_name_closed", CLOSED, "Closed", "Closed { }", Want::Built),
        c("record_by_alias", "type Inner { b: Int = a + 1; }\ntype Alias = Inner;\n", "Inner", "Alias { }", Want::Refused),
        c("record_by_alias_closed", "type Closed { c: Int = 3; }\ntype Alias = Closed;\n", "Closed", "Alias { }", Want::Built),
        Cell { lib: INNER, ..c("record_by_import", "", "l::Inner", "l::Inner { }", Want::Refused) },
        Cell { lib: CLOSED, ..c("record_by_import_closed", "", "l::Closed", "l::Closed { }", Want::Built) },
        c("nested", "type Inner { b: Int = a + 1; }\ntype Mid { inner: Inner = Inner { }; n: Int = 1; }\n", "Mid", "Mid { }", Want::Refused),
        c("nested_closed", "type Closed { c: Int = 3; }\ntype Mid { inner: Closed = Closed { }; n: Int = 1; }\n", "Mid", "Mid { }", Want::Built),
        c("aliased_bounded", "type Tags = bounded[Int; 4];\ntype Tagged { tags: Tags; }\n", "Tagged", "Tagged { }", Want::Built),
        c("aliased_array_field", "type Pair = [Int; 2];\n", "Pair", "[1, 2]", Want::Required),
        c("aliased_array_in_literal", "type Pair = [Int; 2];\ntype Holder { p: Pair = [1, 2]; }\n", "Holder", "Holder { }", Want::Built),
        // a map is spelled out: the check refuses `indexed_by` on an alias
        // of the list and on an alias of its records
        c(
            "map",
            "type Item { name: String; qty: Int = 1; }\ntype Shelf { items: bounded[Item; 4] indexed_by name; n: Int = 1; }\n",
            "Shelf",
            "Shelf { }",
            Want::Built,
        ),
        // a call that leaves out a parameter evaluates its default where
        // the call is written; that default is held to the rule where it
        // is declared
        c("fn_open_default", "fn value(n: Int = a + 1) -> Int { return n; }\n", "Int", "value()", Want::Refused),
        c("fn_closed_default", "fn value(n: Int = 2) -> Int { return n; }\n", "Int", "value()", Want::Built),
        Cell { lib: "fn value(n: Int = a + 1) -> Int { return n; }\n", ..c("fn_by_import", "", "Int", "l::value()", Want::Refused) },
        Cell { lib: "fn value(n: Int = 2) -> Int { return n; }\n", ..c("fn_by_import_closed", "", "Int", "l::value()", Want::Built) },
        c("fn_generic", "fn pick<T>(x: T, n: Int = a + 1) -> T { return x; }\n", "Int", "pick(3)", Want::Refused),
        // a generic call that leaves a parameter does not build today (the
        // checker walks no typed body for it), so this holds the rule for
        // the day it does
        c("fn_generic_closed", "fn pick<T>(x: T, n: Int = 1) -> T { return x; }\n", "Int", "pick(3)", Want::MayNotBuild { required: false }),
        c(
            "fn_nested",
            "fn inner(n: Int = a + 1) -> Int { return n; }\nfn outer(m: Int = inner()) -> Int { return m; }\n",
            "Int",
            "outer()",
            Want::Refused,
        ),
        c(
            "fn_nested_closed",
            "fn inner(n: Int = 2) -> Int { return n; }\nfn outer(m: Int = inner()) -> Int { return m; }\n",
            "Int",
            "outer()",
            Want::Built,
        ),
        c("variant_by_alias", "type Color = enum { red, blue };\ntype Hue = Color;\n", "Color", "Hue::blue", Want::Built),
        c("cycle", "type A { x: Int = 1; b: B = B { }; }\ntype B { y: Int = 2; a: A = A { }; }\n", "A", "A { }", Want::MayNotBuild { required: false }),
    ]
}

fn program(cell: &Cell, body: &str) -> String {
    let import = if cell.lib.is_empty() { "" } else { "import \"./l\" as l;\n" };
    format!(
        "{import}{}type Outer {{ f: {} = {}; }}\nfn main() {{\n    let a = 2;\n    {}    let original = Outer {{ }};\n{body}}}\n",
        cell.decls, cell.ty, cell.default, cell.locals
    )
}

const CONTROL: &str = "    println(\"control\");\n";
const DECODE: &str =
    "    let back = Outer::api_decode(\"{}\") or {\n        println(\"refused\");\n        return;\n    };\n    println(\"decoded\");\n";

fn build(dir: &Path, name: &str, src: &str) -> Result<PathBuf, String> {
    let file = dir.join(format!("{name}.hl"));
    std::fs::write(&file, src).unwrap();
    let bin = dir.join(name);
    let o = hale().arg("build").arg(&file).arg("-o").arg(&bin).output().expect("hale build");
    if o.status.success() {
        Ok(bin)
    } else {
        Err(String::from_utf8_lossy(&o.stderr).into_owned())
    }
}

/// One cell: what it found, or why it fails.
fn run_cell(cell: &Cell) -> Result<(), String> {
    let dir = unique_dir(cell.name);
    if !cell.lib.is_empty() {
        std::fs::write(dir.join("l/main.hl"), cell.lib).unwrap();
    }
    let control = build(&dir, "control", &program(cell, CONTROL));
    let result = (|| {
        match (&control, cell.want) {
            (Err(e), Want::Refused) if e.contains(RULE) => return Ok(()),
            (Err(e), Want::Refused) => return Err(format!("the control is refused, not by the rule:\n{e}")),
            (Ok(_), Want::Refused) => return Err("the check admits a default the rule refuses".to_string()),
            (Err(_), Want::MayNotBuild { .. }) => return Ok(()),
            (Err(e), _) => return Err(format!("the control does not build:\n{e}")),
            (Ok(_), _) => {}
        }
        // the codec copies only what it can evaluate: the control built, so
        // this builds
        let bin = build(&dir, "decode", &program(cell, DECODE)).map_err(|e| format!("the control builds, the codec call does not:\n{e}"))?;
        let out = Command::new(&bin).output().expect("run");
        let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let refused = match said.as_str() {
            "refused" => true,
            "decoded" => false,
            other => return Err(format!("the program said {other:?}")),
        };
        // the description, of the same declarations served
        let served = format!(
            "{}locus L {{ fn get(o: Outer) -> Outer {{ return o; }} }}\napi S {{ rpc L::get; }}\n",
            program(cell, CONTROL)
        );
        let file = dir.join("served.hl");
        std::fs::write(&file, served).unwrap();
        let out_dir = dir.join("out");
        let e = hale().args(["api", "export", "--surface", "S", "--out"]).arg(&out_dir).arg(&file).output().expect("export");
        if !e.status.success() {
            return Err(format!("export: {}", String::from_utf8_lossy(&e.stderr)));
        }
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out_dir.join("S.description.json")).unwrap()).unwrap();
        let required = doc["schemas"]["Outer"]["required"].as_array().cloned().unwrap_or_default().iter().any(|k| k == "f");
        if required != refused {
            return Err(format!("the description says required: {required}, the decoder refuses `{{}}`: {refused}"));
        }
        let want = match cell.want {
            Want::Required => true,
            Want::Built | Want::Refused => false,
            Want::MayNotBuild { required } => required,
        };
        if required != want {
            return Err(format!("required: {required}, the rule says {want}"));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn every_default_the_check_admits_is_one_the_codec_builds() {
    let cells = cells();
    let failures: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = cells
            .chunks(cells.len().div_ceil(4))
            .map(|chunk| {
                s.spawn(move || {
                    chunk.iter().filter_map(|c| run_cell(c).err().map(|e| format!("{}: {e}", c.name))).collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    assert!(failures.is_empty(), "{} of {} cells:\n{}", failures.len(), cells.len(), failures.join("\n\n"));
}
