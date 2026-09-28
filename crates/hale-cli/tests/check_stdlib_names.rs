//! A name the stdlib declares is declared once (spec/semantics.md
//! § "Declarations inside `module { }`"), as `hale check` says it. The CLI
//! demangles the stdlib's internal names in a message, so the refusal is
//! worded to read true after that: a program locus spelling
//! `__StdIoTcpStream` is told it took the internal name of the stdlib's
//! `std::io::tcp::Stream`, with the caret under what it wrote.

use std::path::Path;
use std::process::Command;

#[test]
fn a_program_locus_taking_a_stdlib_name_is_told_which_one() {
    let d = std::env::temp_dir().join(format!("hale_stdlib_names_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, "locus __StdIoTcpStream {\n    params { n: Int = 0; }\n}\nfn main() { print(1); }\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", &f.to_string_lossy()])
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&d);
    assert!(!out.status.success(), "refused: {text}");
    assert!(
        text.contains("main.hl:1:7: type error: this locus's name is the stdlib's internal name for its locus `std::io::tcp::Stream`"),
        "located at the program's declaration, naming the stdlib's public name: {text}"
    );
    assert!(text.contains("note: the stdlib's declaration (in the standard library, io_tcp.hl:"), "and the stdlib's declaration: {text}");
}
