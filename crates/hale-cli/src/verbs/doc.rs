use hale_lsp as lsp;
use std::collections::BTreeMap;
use std::process::ExitCode;
use std::path::PathBuf;
use std::env;
use std::fs;
/// `hale doc [file | dir] [--json] [-o <path>]` — the API-reference
/// generator (spec/testing.md). Zero config: the convention is
/// `///` doc comments on the lines directly above a declaration
/// (decorator lines like `@hot` may sit between); the generator
/// renders every public top-level declaration — fns, loci (with
/// their params and documented methods), types, topics, interfaces,
/// consts — as Markdown (default, stdout or `-o`) or JSON records.
/// Names starting with `__` are internal and skipped. A file that
/// doesn't parse is reported and skipped (exit 1).
pub(crate) fn run_doc(rest: &[String]) -> ExitCode {
    let mut json = false;
    let mut stdlib = false;
    let mut out_path: Option<PathBuf> = None;
    let mut target: Option<PathBuf> = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            "--stdlib" => {
                stdlib = true;
                i += 1;
            }
            "-o" | "--out" => match rest.get(i + 1) {
                Some(v) => {
                    out_path = Some(PathBuf::from(v));
                    i += 2;
                }
                None => {
                    eprintln!("hale doc: {} requires a path", rest[i]);
                    return ExitCode::from(2);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("hale doc: unknown flag {}", other);
                return ExitCode::from(2);
            }
            other => {
                target = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    if stdlib {
        return run_doc_stdlib(json, out_path);
    }
    let target = target
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // A seed = one directory (F.19): file target docs that file,
    // dir target docs every .hl directly in it.
    let mut files: Vec<PathBuf> = Vec::new();
    if target.is_file() {
        files.push(target.clone());
    } else if target.is_dir() {
        if let Ok(rd) = fs::read_dir(&target) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "hl") {
                    files.push(p);
                }
            }
        }
        files.sort();
    } else {
        eprintln!("hale doc: {} not found", target.display());
        return ExitCode::from(1);
    }

    let mut failed = false;
    let mut md = String::new();
    let seed_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| target.display().to_string());
    md.push_str(&format!("# API — {}\n", seed_name));
    let mut json_items: Vec<serde_json::Value> = Vec::new();

    for f in &files {
        let src = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("hale doc: could not read {}: {}", f.display(), e);
                failed = true;
                continue;
            }
        };
        let program = match hale_syntax::parse_source(&src) {
            Ok(p) => p,
            Err(_) => {
                eprintln!(
                    "hale doc: {}: does not parse — skipped",
                    f.display()
                );
                failed = true;
                continue;
            }
        };
        let entries = doc_entries_for(&src, &program);
        if entries.is_empty() {
            continue;
        }
        md.push_str(&format!("\n## {}\n", f.display()));
        for e in &entries {
            md.push_str(&format!("\n### {}\n\n```hale\n{}\n```\n", e.name, e.signature));
            if !e.doc.is_empty() {
                md.push_str(&format!("\n{}\n", e.doc));
            }
            for m in &e.members {
                md.push_str(&format!(
                    "\n- `{}`{}\n",
                    m.signature,
                    if m.doc.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", m.doc.replace('\n', " "))
                    }
                ));
            }
            if json {
                json_items.push(serde_json::json!({
                    "file": f.display().to_string(),
                    "kind": e.kind,
                    "name": e.name,
                    "signature": e.signature,
                    "doc": e.doc,
                    "members": e.members.iter().map(|m| serde_json::json!({
                        "signature": m.signature, "doc": m.doc
                    })).collect::<Vec<_>>(),
                }));
            }
        }
    }

    let rendered = if json {
        serde_json::to_string_pretty(&json_items)
            .unwrap_or_else(|_| "[]".into())
            + "\n"
    } else {
        md
    };
    match out_path {
        Some(p) => {
            if let Err(e) = fs::write(&p, rendered) {
                eprintln!("hale doc: could not write {}: {}", p.display(), e);
                return ExitCode::from(1);
            }
            eprintln!("wrote {}", p.display());
        }
        None => print!("{}", rendered),
    }
    if failed {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// `hale doc --stdlib` — the `std::` API reference. Merges three
/// sources of truth: the rename table (public `std::` path per
/// mangled decl), the bundled stdlib source (decl shapes + `///`
/// doc comments — public method surface of each locus), and the
/// typecheck signature table (the C-primitive-backed free fns that
/// have no .hl decl). Grouped by namespace; Markdown or JSON like
/// the seed mode.
pub(crate) fn run_doc_stdlib(json: bool, out_path: Option<PathBuf>) -> ExitCode {
    use hale_syntax::ast::{LocusMember, TopDecl};
    let src = hale_codegen::stdlib_doc_source();
    let program = match hale_syntax::parse_source(src) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("hale doc --stdlib: bundled stdlib does not parse (bug)");
            return ExitCode::from(1);
        }
    };
    // mangled name -> public path segments
    let mut public: BTreeMap<&str, String> = BTreeMap::new();
    for (segs, mangled) in hale_codegen::stdlib_path_renames() {
        public.insert(*mangled, segs.join("::"));
    }
    // Signatures written against internal names (a locus param
    // typed `__StdMetricsMap`) display their public paths.
    let demangle = |sig: &str| -> String {
        let mut out = sig.to_string();
        for (mangled, pubpath) in &public {
            if out.contains(mangled) {
                out = out.replace(mangled, pubpath);
            }
        }
        out
    };

    // namespace ("std::metrics") -> entries
    let mut groups: BTreeMap<String, Vec<DocEntry>> = BTreeMap::new();
    let ns_of = |path: &str| -> String {
        match path.rfind("::") {
            Some(i) => path[..i].to_string(),
            None => path.to_string(),
        }
    };

    for item in &program.items {
        match item {
            TopDecl::Fn(fd) => {
                let Some(path) = public.get(fd.name.name.as_str()) else {
                    continue;
                };
                // Leaf name first (demangle would otherwise expand
                // the fn's own mangled name to its full path), then
                // demangle the param/return types.
                let leaf = path.rsplit("::").next().unwrap_or(path);
                let sig = demangle(
                    &doc_fn_signature(fd).replacen(&fd.name.name, leaf, 1),
                );
                groups.entry(ns_of(path)).or_default().push(DocEntry {
                    kind: "fn",
                    name: path.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        fd.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Type(t) => {
                let Some(path) = public.get(t.name.name.as_str()) else {
                    continue;
                };
                use hale_syntax::ast::TypeDeclBody;
                let leaf = path.rsplit("::").next().unwrap_or(path);
                let sig = match &t.body {
                    TypeDeclBody::Struct(fields) => {
                        let fs = fields
                            .iter()
                            .filter(|f| !f.name.name.starts_with("__"))
                            .map(|f| {
                                format!(
                                    "{}: {};",
                                    f.name.name,
                                    demangle(&lsp::type_expr_str(&f.ty))
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        format!("type {} {{ {} }}", leaf, fs)
                    }
                    TypeDeclBody::Enum(vs) => {
                        let names = vs
                            .iter()
                            .map(|v| v.name.name.clone())
                            .collect::<Vec<_>>()
                            .join(" | ");
                        format!("type {} = enum {{ {} }}", leaf, names)
                    }
                    TypeDeclBody::Alias(inner) => format!(
                        "type {} = {}",
                        leaf,
                        demangle(&lsp::type_expr_str(inner))
                    ),
                };
                groups.entry(ns_of(path)).or_default().push(DocEntry {
                    kind: "type",
                    name: path.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        t.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Locus(l) => {
                let Some(path) = public.get(l.name.name.as_str()) else {
                    continue;
                };
                let mut members = Vec::new();
                let mut params_sig = String::new();
                for m in &l.members {
                    match m {
                        LocusMember::Params(pb) => {
                            // Skip __-named params AND params whose
                            // type demangles to nothing public
                            // (internal owned-storage wiring like
                            // Router's entry list).
                            let ps = pb
                                .params
                                .iter()
                                .filter(|p| !p.name.name.starts_with("__"))
                                .filter_map(|p| match &p.ty {
                                    Some(t) => {
                                        let ty =
                                            demangle(&lsp::type_expr_str(t));
                                        if ty.contains("__") {
                                            None
                                        } else {
                                            Some(format!(
                                                "{}: {}",
                                                p.name.name, ty
                                            ))
                                        }
                                    }
                                    None => Some(p.name.name.clone()),
                                })
                                .collect::<Vec<_>>()
                                .join("; ");
                            params_sig = ps;
                        }
                        LocusMember::Fn(fd) => {
                            if fd.name.name.starts_with("__") {
                                continue;
                            }
                            members.push(DocMember {
                                signature: demangle(&doc_fn_signature(fd)),
                                doc: doc_comment_above(
                                    src,
                                    fd.name.span.start.as_usize(),
                                ),
                            });
                        }
                        _ => {}
                    }
                }
                let leaf = path.rsplit("::").next().unwrap_or(path);
                let sig = if params_sig.is_empty() {
                    format!("locus {}", leaf)
                } else {
                    demangle(&format!(
                        "locus {} {{ params {{ {} }} }}",
                        leaf, params_sig
                    ))
                };
                groups.entry(ns_of(path)).or_default().push(DocEntry {
                    kind: "locus",
                    name: path.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        l.name.span.start.as_usize(),
                    ),
                    members,
                });
            }
            _ => {}
        }
    }

    // Signature-table fns with no .hl decl (C-primitive-backed).
    let covered: std::collections::BTreeSet<String> = groups
        .values()
        .flatten()
        .map(|e| e.name.clone())
        .collect();
    for surface in hale_types::stdlib_surface::SURFACES {
        for entry in surface.public() {
            let f = entry.name;
            if f.starts_with("__") {
                continue;
            }
            let mut segs: Vec<&str> = vec!["std"];
            segs.extend(surface.ns.iter().copied());
            segs.push(f);
            let path = segs.join("::");
            if covered.contains(&path) {
                continue;
            }
            let sig = match hale_types::stdlib_surface::signature_for(&segs)
            {
                Some(sig) => {
                    let ps = sig
                        .params
                        .iter()
                        .map(|t| lsp::sig_ty_str(t).to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut d = format!(
                        "fn {}({}) -> {}",
                        f,
                        ps,
                        lsp::sig_ty_str(&sig.ret)
                    );
                    if let Some(e) = sig.fallible {
                        d.push_str(&format!(" fallible({})", e));
                    }
                    d
                }
                None => format!("fn {}(…)", f),
            };
            groups.entry(ns_of(&path)).or_default().push(DocEntry {
                kind: "fn",
                name: path,
                signature: sig,
                doc: String::new(),
                members: Vec::new(),
            });
        }
    }

    // Render.
    let mut md = String::from("# API — std\n");
    let mut json_items: Vec<serde_json::Value> = Vec::new();
    for (ns, entries) in &groups {
        md.push_str(&format!("\n## {}\n", ns));
        for e in entries {
            md.push_str(&format!(
                "\n### {}\n\n```hale\n{}\n```\n",
                e.name, e.signature
            ));
            if let Some(cls) = effect_line(&e.name) {
                md.push_str(&format!("\n{}\n", cls));
            }
            if !e.doc.is_empty() {
                md.push_str(&format!("\n{}\n", e.doc));
            }
            for m in &e.members {
                md.push_str(&format!(
                    "\n- `{}`{}\n",
                    m.signature,
                    if m.doc.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", m.doc.replace('\n', " "))
                    }
                ));
            }
            if json {
                json_items.push(serde_json::json!({
                    "kind": e.kind,
                    "name": e.name,
                    "signature": e.signature,
                    "effects": effect_classes(&e.name),
                    "doc": e.doc,
                    "members": e.members.iter().map(|m| serde_json::json!({
                        "signature": m.signature, "doc": m.doc
                    })).collect::<Vec<_>>(),
                }));
            }
        }
    }
    let rendered = if json {
        serde_json::to_string_pretty(&json_items)
            .unwrap_or_else(|_| "[]".into())
            + "\n"
    } else {
        md
    };
    match out_path {
        Some(p) => {
            if let Err(e) = fs::write(&p, rendered) {
                eprintln!("hale doc: could not write {}: {}", p.display(), e);
                return ExitCode::from(1);
            }
            eprintln!("wrote {}", p.display());
        }
        None => print!("{}", rendered),
    }
    ExitCode::SUCCESS
}

pub(crate) struct DocMember {
    pub(crate) signature: String,
    pub(crate) doc: String,
}

pub(crate) struct DocEntry {
    pub(crate) kind: &'static str,
    pub(crate) name: String,
    pub(crate) signature: String,
    pub(crate) doc: String,
    pub(crate) members: Vec<DocMember>,
}

/// The effect classes for a `std::` path, for `--json` consumers.
/// Empty vec = pure; `None` = no registry row (a locus or type).
pub(crate) fn effect_classes(path: &str) -> Option<Vec<String>> {
    let segs: Vec<&str> = path.split("::").collect();
    let set = hale_types::stdlib_surface::effects_for(&segs)?;
    Some(hale_types::frontier::render_effects(set))
}

/// The effect classification for a `std::` path, as a doc line.
///
/// Read straight out of the registry rather than written down here:
/// every surface entry already carries an `EffectSet`, and the
/// generator was walking those entries to print signatures while
/// ignoring the column sitting next to them. Deriving it means the
/// published catalogue cannot drift from what the checker enforces —
/// a hand-maintained table of 327 rows certainly would.
///
/// `None` for anything with no registry row (locus and type paths,
/// which are tracked separately) so those entries render unchanged.
pub(crate) fn effect_line(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split("::").collect();
    let set = hale_types::stdlib_surface::effects_for(&segs)?;
    let classes = hale_types::frontier::render_effects(set);
    if classes.is_empty() {
        // PURE is a real answer, and a useful one: it is what makes a
        // fn callable from a `@no_syscall` / `@deterministic` context.
        return Some("**Effects:** none — callable under any assertion.".into());
    }
    Some(format!(
        "**Effects:** {}",
        classes
            .iter()
            .map(|c| format!("`{}`", c))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// The `///` block directly above the line holding `anchor`
/// (byte offset). Decorator lines (`@hot`, `@form(...)`) between
/// the docs and the declaration are stepped over.
pub(crate) fn doc_comment_above(src: &str, anchor: usize) -> String {
    let lines: Vec<&str> = src.lines().collect();
    // Line index containing the anchor offset.
    let mut off = 0usize;
    let mut anchor_line = 0usize;
    for (i, l) in lines.iter().enumerate() {
        let end = off + l.len() + 1;
        if anchor < end {
            anchor_line = i;
            break;
        }
        off = end;
    }
    let mut i = anchor_line;
    // Step over decorator-only lines above the decl.
    while i > 0 {
        let prev = lines[i - 1].trim();
        if prev.starts_with('@') {
            i -= 1;
        } else {
            break;
        }
    }
    let mut docs: Vec<&str> = Vec::new();
    while i > 0 {
        let prev = lines[i - 1].trim();
        if let Some(text) = prev.strip_prefix("///") {
            docs.push(text.strip_prefix(' ').unwrap_or(text));
            i -= 1;
        } else {
            break;
        }
    }
    docs.reverse();
    docs.join("\n").trim().to_string()
}

pub(crate) fn doc_fn_signature(fd: &hale_syntax::ast::FnDecl) -> String {
    let ps = fd
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name.name, lsp::type_expr_str(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sig = format!("fn {}({})", fd.name.name, ps);
    if let Some(r) = &fd.ret {
        sig.push_str(&format!(" -> {}", lsp::type_expr_str(r)));
    }
    if let Some(e) = &fd.fallible {
        sig.push_str(&format!(" fallible({})", lsp::type_expr_str(e)));
    }
    sig
}

pub(crate) fn doc_entries_for(
    src: &str,
    program: &hale_syntax::ast::Program,
) -> Vec<DocEntry> {
    use hale_syntax::ast::{LocusMember, TopDecl, TypeDeclBody};
    let mut out = Vec::new();
    for item in &program.items {
        match item {
            TopDecl::Fn(fd) => {
                if fd.name.name.starts_with("__")
                    || fd.name.name == "main"
                {
                    continue;
                }
                out.push(DocEntry {
                    kind: "fn",
                    name: fd.name.name.clone(),
                    signature: doc_fn_signature(fd),
                    doc: doc_comment_above(
                        src,
                        fd.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Locus(l) => {
                if l.name.name.starts_with("__") {
                    continue;
                }
                let mut sig = format!("locus {}", l.name.name);
                let mut members = Vec::new();
                for m in &l.members {
                    match m {
                        LocusMember::Params(pb) => {
                            let ps = pb
                                .params
                                .iter()
                                .map(|p| match &p.ty {
                                    Some(t) => format!(
                                        "{}: {}",
                                        p.name.name,
                                        lsp::type_expr_str(t)
                                    ),
                                    None => p.name.name.clone(),
                                })
                                .collect::<Vec<_>>()
                                .join("; ");
                            sig.push_str(&format!(
                                " {{ params {{ {} }} }}",
                                ps
                            ));
                        }
                        LocusMember::Fn(fd) => {
                            if fd.name.name.starts_with("__") {
                                continue;
                            }
                            members.push(DocMember {
                                signature: doc_fn_signature(fd),
                                doc: doc_comment_above(
                                    src,
                                    fd.name.span.start.as_usize(),
                                ),
                            });
                        }
                        _ => {}
                    }
                }
                out.push(DocEntry {
                    kind: "locus",
                    name: l.name.name.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        l.name.span.start.as_usize(),
                    ),
                    members,
                });
            }
            TopDecl::Type(t) => {
                if t.name.name.starts_with("__") {
                    continue;
                }
                let sig = match &t.body {
                    TypeDeclBody::Struct(fields) => {
                        let fs = fields
                            .iter()
                            .map(|f| {
                                format!(
                                    "{}: {};",
                                    f.name.name,
                                    lsp::type_expr_str(&f.ty)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        format!("type {} {{ {} }}", t.name.name, fs)
                    }
                    TypeDeclBody::Enum(vs) => {
                        let names = vs
                            .iter()
                            .map(|v| v.name.name.clone())
                            .collect::<Vec<_>>()
                            .join(" | ");
                        format!("type {} = enum {{ {} }}", t.name.name, names)
                    }
                    TypeDeclBody::Alias(inner) => format!(
                        "type {} = {}",
                        t.name.name,
                        lsp::type_expr_str(inner)
                    ),
                };
                out.push(DocEntry {
                    kind: "type",
                    name: t.name.name.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        t.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Topic(t) => {
                let mut sig = format!(
                    "topic {} {{ payload: {}",
                    t.name.name,
                    lsp::type_expr_str(&t.payload)
                );
                if let Some(k) = &t.keyed_by {
                    sig.push_str(&format!("; keyed_by {}", k.name));
                }
                sig.push_str(" }");
                out.push(DocEntry {
                    kind: "topic",
                    name: t.name.name.clone(),
                    signature: sig,
                    doc: doc_comment_above(
                        src,
                        t.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Interface(iface) => {
                let ms = iface
                    .methods
                    .iter()
                    .map(|m| {
                        let ps = m
                            .params
                            .iter()
                            .map(|p| {
                                format!(
                                    "{}: {}",
                                    p.name.name,
                                    lsp::type_expr_str(&p.ty)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        let ret = m
                            .ret
                            .as_ref()
                            .map(|r| {
                                format!(" -> {}", lsp::type_expr_str(r))
                            })
                            .unwrap_or_default();
                        format!("fn {}({}){};", m.name.name, ps, ret)
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push(DocEntry {
                    kind: "interface",
                    name: iface.name.name.clone(),
                    signature: format!(
                        "interface {} {{ {} }}",
                        iface.name.name, ms
                    ),
                    doc: doc_comment_above(
                        src,
                        iface.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            TopDecl::Const(c) => {
                out.push(DocEntry {
                    kind: "const",
                    name: c.name.name.clone(),
                    signature: format!(
                        "const {}: {}",
                        c.name.name,
                        lsp::type_expr_str(&c.ty)
                    ),
                    doc: doc_comment_above(
                        src,
                        c.name.span.start.as_usize(),
                    ),
                    members: Vec::new(),
                });
            }
            _ => {}
        }
    }
    out
}
