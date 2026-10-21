//! GH #1530 (fourth pass, section 3): the `@repo(T)` laws
//! (`hale-types::repo`, spec/api.md § Repositories).

#[path = "support/entries.rs"]
mod entries;
use entries::check_files;

fn check(src: &str) -> Vec<String> {
    check_files(&[("main.hl", src)])
        .into_iter()
        .map(|d| d.message)
        .collect()
}

const HEAD: &str = r#"
type Item { id: Int `db:"id,key"`; n: Int; }
type ItemError = enum { @not_found Missing };
"#;

const GOOD: &str = r#"
@repo(Item)
locus Items {
    @effects(is: { reads(Item) })
    fn get(id: Int) -> Item fallible(ItemError) { fail ItemError::Missing; }
    @effects(is: { creates(Item) }) @idempotent
    fn create(o: Item) -> Item fallible(ItemError) { return o; }
    @effects(is: { writes(Item) }) @not_idempotent
    fn put(id: Int, o: Item) -> Item fallible(ItemError) { return o; }
    @effects(is: { deletes(Item) }) @idempotent
    fn remove(id: Int) fallible(ItemError) { }
}
fn main() { let i = Items { }; }
"#;

fn with(replace: &str, by: &str) -> Vec<String> {
    assert!(GOOD.contains(replace), "{replace}");
    check(&format!("{HEAD}{}", GOOD.replace(replace, by)))
}

fn has(msgs: &[String], needle: &str) -> bool {
    msgs.iter().any(|m| m.contains(needle))
}

#[test]
fn a_complete_repo_is_accepted() {
    let msgs = check(&format!("{HEAD}{GOOD}"));
    assert!(msgs.is_empty(), "{msgs:?}");
}

#[test]
fn a_missing_method_is_named() {
    let msgs = with(
        "    @effects(is: { deletes(Item) }) @idempotent\n    fn remove(id: Int) fallible(ItemError) { }\n",
        "",
    );
    assert!(has(&msgs, "has no `remove`"), "{msgs:?}");
}

#[test]
fn a_method_with_no_class_is_refused() {
    let msgs = with(
        "    @effects(is: { reads(Item) })\n    fn get",
        "    fn get",
    );
    assert!(
        has(
            &msgs,
            "`Items::get` of a `@repo(Item)` declares no state class"
        ),
        "{msgs:?}"
    );
}

#[test]
fn the_class_must_agree_with_the_method() {
    let msgs = with(
        "writes(Item) }) @not_idempotent",
        "creates(Item) }) @not_idempotent",
    );
    assert!(
        has(
            &msgs,
            "`Items::put` of a `@repo(Item)` is `creates(Item)`; `put` is `writes(Item)`"
        ),
        "{msgs:?}"
    );
}

#[test]
fn a_write_class_needs_an_idempotency_marker() {
    let msgs = with("creates(Item) }) @idempotent", "creates(Item) })");
    assert!(
        has(
            &msgs,
            "write `@idempotent` or `@not_idempotent`; there is no default"
        ),
        "{msgs:?}"
    );
}

#[test]
fn a_read_carries_no_marker() {
    let msgs = with(
        "reads(Item) })\n    fn get",
        "reads(Item) }) @idempotent\n    fn get",
    );
    assert!(has(&msgs, "which is always safe to repeat"), "{msgs:?}");
}

#[test]
fn the_two_markers_are_exclusive() {
    let msgs = with(
        "creates(Item) }) @idempotent",
        "creates(Item) }) @idempotent @not_idempotent",
    );
    assert!(
        has(&msgs, "both `@idempotent` and `@not_idempotent`"),
        "{msgs:?}"
    );
}

#[test]
fn a_repo_declares_no_class_of_another_type() {
    let msgs = check(&format!(
        "type Other {{ id: Int; }}\n{HEAD}{}",
        GOOD.replace("fn main", "@repo(Item) locus Two {} fn main")
            .replace(
                "    @effects(is: { reads(Item) })\n    fn get",
                "    @effects(is: { reads(Other) })\n    fn get"
            ),
    ));
    assert!(has(&msgs, "no state class of another type"), "{msgs:?}");
}

#[test]
fn an_extra_method_needs_a_class() {
    let src = GOOD.replace(
        "    @effects(is: { deletes",
        "    fn find(n: Int) -> Int { return n; }\n    @effects(is: { deletes",
    );
    let msgs = check(&format!("{HEAD}{src}"));
    assert!(
        has(
            &msgs,
            "`Items::find` of a `@repo(Item)` declares no state class"
        ),
        "{msgs:?}"
    );
}

#[test]
fn the_type_needs_exactly_one_key() {
    let msgs = check(
        &GOOD
            .replace("@repo(Item)", "type Bare { id: Int; }\n@repo(Bare)")
            .replace("{HEAD}", ""),
    );
    assert!(has(&msgs, "has no key field"), "{msgs:?}");
    let two = HEAD.replace("n: Int;", "n: Int `db:\"n,key\"`;");
    let msgs = check(&format!("{two}{GOOD}"));
    assert!(has(&msgs, "more than one key field"), "{msgs:?}");
}

#[test]
fn a_marker_belongs_to_a_repo_method() {
    let msgs = check("locus L { @idempotent fn f() { } }\nfn main() { }\n");
    assert!(
        has(&msgs, "marks a method of a `@repo(T)` locus"),
        "{msgs:?}"
    );
}

#[test]
fn a_method_set_with_the_wrong_shape_is_named() {
    let msgs = with("fn put(id: Int, o: Item)", "fn put(id: Int)");
    assert!(has(&msgs, "`Items::put` of a `@repo(Item)` must be `fn put(id: Int, o: Item) -> Item fallible(E)`"), "{msgs:?}");
}

#[test]
fn an_imported_repo_names_its_renamed_record() {
    let lib = format!(
        "{HEAD}{}",
        GOOD.replace("fn main() { let i = Items { }; }\n", "")
    );
    let main = "import \"./store\" as r;\nfn main() { let i = r::Items { }; }\n";
    let msgs: Vec<String> = check_files(&[("main.hl", main), ("store/store.hl", &lib)])
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert!(msgs.is_empty(), "{msgs:?}");
}

fn lib_with(wrapper: &str) -> String {
    format!(
        "{HEAD}{}\n{wrapper}\n",
        GOOD.replace("fn main() { let i = Items { }; }\n", "")
    )
}

fn two_modules(lib: &str, root_repo: bool) -> Vec<String> {
    let root = if root_repo {
        format!(
            "{HEAD}{}",
            GOOD.replace("fn main() { let i = Items { }; }\n", "")
        )
    } else {
        String::new()
    };
    let main = format!(
        "import \"./store\" as r;\n{root}\nfn main() {{ let remote = r::Items {{ }}; r::action(remote); }}\n"
    );
    check_files(&[("main.hl", &main), ("store/store.hl", lib)])
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn an_imported_contract_names_the_librarys_own_record() {
    let put = "@effects(none: { writes(Item) })\nfn action(db: Items) {\n    let got = db.put(1, Item { id: 1, n: 2 }) or Item { id: 0, n: 0 };\n}\n";
    // the library's prohibition holds against the library's own write,
    // whether or not the root has an unrelated `Item` repository
    for root_repo in [false, true] {
        let msgs = two_modules(&lib_with(put), root_repo);
        assert!(
            has(&msgs, "effect assertion violated"),
            "root_repo={root_repo}: {msgs:?}"
        );
    }
    let get = "@effects(none: { writes(Item) })\nfn action(db: Items) {\n    let got = db.get(1) or Item { id: 0, n: 0 };\n}\n";
    for root_repo in [false, true] {
        let msgs = two_modules(&lib_with(get), root_repo);
        assert!(msgs.is_empty(), "root_repo={root_repo}: {msgs:?}");
    }
}
