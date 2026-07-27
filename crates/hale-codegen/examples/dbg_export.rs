fn main() {
    let src = r#"
@export fn hale_add(a: Int, b: Int) -> Int {
    return a + b;
}
fn main() { println("ok"); }
"#;
    let p = hale_syntax::parse_source(src).expect("parse");
    for item in &p.items {
        if let hale_syntax::ast::TopDecl::Fn(f) = item {
            eprintln!("fn {} export={}", f.name.name, f.export);
        }
    }
}
