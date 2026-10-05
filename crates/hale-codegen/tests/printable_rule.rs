//! What prints is one rule (F.40 phase 4, S7): `hale_types::printable`,
//! which the checker and lowering both read, so the two give one answer.
//!
//! Before it, lowering kept its own copy, and it differed from the
//! checker's twice. A `StringView` printed through `println`'s own arm
//! but not through the renderer every other path uses, so `"x=" + view`,
//! `to_string(view)` and `f"{view}"` passed `hale check` and failed at
//! build. And lowering rendered any named record without looking at its
//! fields, where the checker requires every field to print: a program the
//! check refuses (which only a harness build lowers) built a print of a
//! record the checker says has no text form.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// A view prints through every path, as its text: `println`, the
/// `String + view` coercion on either side, `to_string`, an f-string with
/// and without a spec, and inside a record and a tuple (quoted, as a
/// String is). `to_string` copies the text out, so the String it returns
/// does not follow the builder a later append moves.
#[test]
fn a_string_view_prints_through_every_path() {
    let src = r#"
        type Holder {
            n: Int;
            v: StringView;
        }

        fn main() {
            let b = std::bytes::BytesBuilder { };
            b.append_str("hello");
            let sv = b.text_view();
            println(sv);
            let c = "view=" + sv;
            let d = sv + "!";
            let t = to_string(sv);
            let f = f"[{sv}]";
            let p = f"[{sv:>8}]";
            let h = Holder { n: 1, v: sv };
            let tup = (2, sv);
            println(c);
            println(d);
            println(t);
            println(f);
            println(p);
            println(h);
            println(tup);
            b.append_str(" world");
            println(t);
            println(b.text_view());
        }
    "#;
    let bin = harness::unique_bin("view");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("a program printing a view builds");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "status {:?}", out.status);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "hello\nview=hello\nhello!\nhello\n[hello]\n[   hello]\nHolder { n: 1, v: \"hello\" }\n\
         (2, \"hello\")\nhello\nhello world\n"
    );
}

/// A record prints when its fields do. `[String; 2]` does not (a
/// sequence prints only with scalar elements), so the record holding one
/// has no text form, and lowering refuses to print it with the message
/// that names why, where it used to build the print. The check refuses the
/// program first (`diag_quality`'s
/// `printing_a_struct_with_an_unprintable_field_is_a_check_error`); only
/// a harness build reaches lowering with it.
#[test]
fn a_record_with_a_field_that_does_not_print_is_refused_by_lowering() {
    let src = r#"
        type Labels {
            id: Int;
            names: [String; 2];
        }

        fn main() {
            let l = Labels { id: 1, names: ["a", "b"] };
            println(l);
        }
    "#;
    let bin = harness::unique_bin("record");
    let err = build_opts::build_source(src, &bin, &build_opts::options())
        .expect_err("lowering refuses to print a record whose field does not print");
    let _ = std::fs::remove_file(&bin);
    assert_eq!(
        err.to_string(),
        "unsupported in codegen v0: println of a type value (TypeRef `Labels`) — one of its fields is not \
         printable; print the printable fields individually"
    );
}
