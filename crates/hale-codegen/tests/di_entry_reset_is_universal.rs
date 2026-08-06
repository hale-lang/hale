//! Every function-body entry point must call `di_begin_function`.
//!
//! A function's prologue — entry-block allocas, deferred-dissolve
//! slots, the method scratch arena, the caller-arena snapshot — is
//! emitted before the body's first statement establishes a debug
//! location, so it inherits whatever location the PREVIOUS function
//! left live. When that happens LLVM rejects the entire module:
//!
//!     !dbg attachment points at wrong subprogram for function
//!
//! That is a hard build failure with no user-side workaround, since
//! debug info is always on. It is also invisible until some
//! arrangement of loci happens to leave a location live across the
//! boundary — the free-fn path carried the reset for months while
//! all five locus-method entry points went without, and the gap only
//! surfaced at three levels of nesting.
//!
//! So this is enforced structurally rather than by review: a new
//! `append_basic_block(f, "entry")` that positions the builder there
//! and forgets the reset fails the build here, not in someone's
//! program.

use std::path::Path;

/// Source sites that create a function's entry block and position
/// the builder at it. Anything matching must reset the location
/// within a few lines.
fn scan(src: &str, path: &str, out: &mut Vec<String>) {
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains("append_basic_block(") || !line.contains("\"entry\"") {
            continue;
        }
        // The builder must actually be positioned at this block for
        // the prologue to land in it; a declared-but-unused entry
        // block cannot inherit anything.
        let window_end = (i + 12).min(lines.len());
        let window = lines[i..window_end].join("\n");
        if !window.contains("position_at_end(entry") {
            continue;
        }
        if window.contains("di_begin_function()") {
            continue;
        }
        out.push(format!(
            "{}:{}: entry block positioned without \
             `self.di_begin_function();`\n    {}",
            path,
            i + 1,
            line.trim()
        ));
    }
}

#[test]
fn di_entry_reset_is_universal() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut scanned = 0usize;

    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir src") {
            let entry = entry.expect("dir entry");
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let src = std::fs::read_to_string(&p).expect("read source");
            let rel = p
                .strip_prefix(&root)
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned();
            scan(&src, &rel, &mut offenders);
            scanned += 1;
        }
    }

    assert!(scanned > 0, "scanned no sources — path wrong?");
    assert!(
        offenders.is_empty(),
        "function-body entry points missing the debug-location \
         reset.\n\nEach of these emits a prologue that can inherit \
         the previously emitted function's !dbg location, which \
         makes LLVM reject the whole module. Add \
         `self.di_begin_function();` right after the \
         `position_at_end(entry)`.\n\n{}\n",
        offenders.join("\n")
    );
}

/// The scanner has to actually be able to see a violation — a lint
/// that cannot fail is worse than no lint, because it reads as
/// coverage.
#[test]
fn the_scanner_catches_a_missing_reset() {
    let bad = r#"
        let entry = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(entry);
        let self_ptr = func.get_nth_param(0);
    "#;
    let mut out = Vec::new();
    scan(bad, "synthetic.rs", &mut out);
    assert_eq!(out.len(), 1, "scanner missed an unguarded entry site");

    let good = r#"
        let entry = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(entry);
        self.di_begin_function();
        let self_ptr = func.get_nth_param(0);
    "#;
    let mut out = Vec::new();
    scan(good, "synthetic.rs", &mut out);
    assert!(out.is_empty(), "scanner flagged a guarded entry site");
}
