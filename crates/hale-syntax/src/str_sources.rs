//! The text `for` sources (GH #1995 S2).
//!
//! `for part in std::str::split(s, sep) { … }`,
//! `for line in std::str::lines(s) { … }` and
//! `for w in std::str::words(s) { … }` are `for` sources, not calls:
//! Hale has no sequence value for a split to return (that is why
//! `split_into` writes into caller storage), and none is built here. The
//! pass rewrites each loop into one `while` over a view of `s`, the
//! ByteView scan `crates/hale-stdlib/hl/str_view.hl` documents: one
//! strlen when the view is built, then per part one `end` hook (a find
//! from the cursor), one `next` hook and one `std::str::slice`. The
//! input is walked once, so a split over a megabyte is linear where the
//! hand loop it replaces (`rest = rest[(sp + 1)..len(rest)]`, a strlen
//! and a tail copy per part) is quadratic.
//!
//! ```text
//! for NAME in std::str::split(S, SEP) BODY
//!
//! {
//!     let __hale_str_split_s<k> = S;
//!     let __hale_str_split_sep<k> = SEP;                 // split only
//!     let __hale_str_v<k> = std::str::bytes_view(__hale_str_split_s<k>);
//!     let mut __hale_str_at<k> = FIRST;
//!     while __hale_str_at<k> <= __hale_str_v<k>.n {      // `<` for lines, words
//!         let __hale_str_lo<k> = __hale_str_at<k>;
//!         let __hale_str_hi<k> = END(v, [sep,] lo);
//!         __hale_str_at<k> = NEXT(v, [sep,] hi);
//!         let NAME = std::str::slice(v, lo, hi);
//!         BODY                                             // a nested block
//!     }
//! }
//! ```
//!
//! | source  | FIRST                     | END                | NEXT               |
//! |---------|---------------------------|--------------------|--------------------|
//! | `split` | `__str_split_first(v, p)` | `__str_split_end`  | `__str_split_next` |
//! | `lines` | `0`                       | `__str_line_end`   | `__str_line_next`  |
//! | `words` | `__str_word_start(v, 0)`  | `__str_word_end`   | `__str_word_start` |
//!
//! The hooks are Hale bodies in `str_helpers.hl`; the cursor moves before
//! the body runs, so `continue` goes straight to the next part and `break`
//! leaves the scan. The element is an ordinary `let` in the iteration's
//! block, a `String` the body owns like any other; the body is its own
//! block under it, so a `let` of the same name inside shadows it as it
//! did the loop variable. Every call the pass writes is pure, so a loop
//! over a source is legal wherever its body is (`@deterministic
//! @no_syscall` included).
//!
//! A chain over a source (`std::str::words(s).filter(len(it) > 2).count()`,
//! GH #1995 S2) is the same scan: `crate::chains`, which runs at parse,
//! before this pass, splices [`scan`]'s statements into the chain's
//! block with its stages and terminal as the body, tagged `c` and the
//! chain's number, and reads [`scan_names`] for the part an
//! element-valued terminal keeps.
//!
//! Only a call of the source's own arity is rewritten. Anything else
//! named `std::str::split` / `lines` / `words` (a call as a value, or a
//! loop over a call with the wrong arguments) is left as written, and the
//! checker refuses it with the reason ([`refusal`]).
//!
//! The pass runs in the desugar sequence before the check
//! (`hale_types::desugar_sequence`), so the checker, the analyses and
//! lowering all see the `while`. Synthesized nodes carry the source
//! call's span, with every callee at the call's own span: the marker of
//! a call the compiler wrote (`hale_types::shadowed_callees`), so a
//! binding of the program's that shares a hook's name is never called
//! instead. `S` and `SEP` keep their own spans and are bound under names
//! that say which source and parameter they are ([`argument_of`]), so
//! an argument of the wrong type is refused where it was written, in the
//! source's words, and only once.

use crate::ast::*;
use crate::span::Span;

/// The text `for` sources: the name after `std::str::` and its arity.
pub const SOURCES: &[(&str, usize)] = &[("split", 2), ("lines", 1), ("words", 1)];

/// The source a callee path names (`std::str::split` → `("split", 2)`),
/// if it names one.
pub fn source_of(callee: &Expr) -> Option<(&'static str, usize)> {
    let Expr::Path(qn) = callee else { return None };
    let [std, str_, f] = qn.segments.as_slice() else { return None };
    if std.name != "std" || str_.name != "str" {
        return None;
    }
    SOURCES.iter().copied().find(|(n, _)| *n == f.name)
}

/// The checker's refusal of a source left as written: a call used as a
/// value, or one with the wrong number of arguments.
pub fn refusal(name: &str, arity: usize, got: usize) -> String {
    let params = if arity == 2 { "s, sep" } else { "s" };
    if got != arity {
        return format!(
            "`std::str::{name}({params})` takes {arity} argument{}, got {got}",
            if arity == 1 { "" } else { "s" }
        );
    }
    format!(
        "`std::str::{name}` is a `for` source and a chain's source, not a value: write `for part in \
         std::str::{name}({params}) {{ … }}`, or a chain over it (`std::str::{name}({params}).filter(…).count()`). \
         Hale has no sequence value to return; to keep the parts, write them into storage you own with \
         `std::str::split_into(s, sep, v)` or a chain's `into(v)` (spec/semantics.md § Text sources)"
    )
}

/// The binding the desugar gives a source's argument: `__hale_str_` then
/// the source, the parameter and the scan's tag, a loop's number or `c`
/// and a chain's (`__hale_str_split_sep3`, `__hale_str_words_sc2`).
fn argument_binding(source: &str, param: &str, tag: &str) -> String {
    format!("__hale_str_{source}_{param}{tag}")
}

/// The source and the parameter a binding the desugar wrote holds
/// (`__hale_str_split_sep3` → `("split", "sep")`), so the checker can
/// refuse an argument of the wrong type in the source's words.
pub fn argument_of(binding: &str) -> Option<(&'static str, &'static str)> {
    let rest = binding.strip_prefix("__hale_str_")?;
    for (source, _) in SOURCES {
        let Some(tail) = rest.strip_prefix(source).and_then(|t| t.strip_prefix('_')) else { continue };
        for param in ["s", "sep"] {
            // a loop's number, or a chain's (`c` then its number, the
            // chain source's scan, `crate::chains`)
            let numbered = |k: &str| !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit());
            if tail.strip_prefix(param).is_some_and(|k| numbered(k) || k.strip_prefix('c').is_some_and(numbered)) {
                return Some((source, param));
            }
        }
    }
    None
}

/// The checker's refusal of a source's argument that is no string.
pub fn argument_refusal(source: &str, param: &str, got: &str) -> String {
    let params = if source == "split" { "s, sep" } else { "s" };
    format!("`std::str::{source}({params})`: `{param}` takes a `String`, not a `{got}`")
}

/// Rewrite every loop over a text source in `program`.
pub fn desugar_str_sources(program: &mut Program) {
    let mut n = 0usize;
    items(&mut program.items, &mut n);
}

fn items(items_: &mut [TopDecl], n: &mut usize) {
    for item in items_ {
        match item {
            TopDecl::Fn(f) => body(&mut f.body, n),
            TopDecl::Locus(l) => {
                for m in &mut l.members {
                    match m {
                        LocusMember::Fn(f) => body(&mut f.body, n),
                        LocusMember::Lifecycle(lc) => body(&mut lc.body, n),
                        LocusMember::Mode(md) => body(&mut md.body, n),
                        LocusMember::Failure(f) => body(&mut f.body, n),
                        _ => {}
                    }
                }
            }
            TopDecl::Perspective(p) => {
                for m in &mut p.members {
                    if let PerspectiveMember::Fn(f) = m {
                        body(&mut f.body, n);
                    }
                }
            }
            TopDecl::Module(m) => items(&mut m.items, n),
            _ => {}
        }
    }
}

/// Innermost first (`each_stmt_mut`), so a source loop nested in
/// another's body is rewritten before the outer one moves that body.
fn body(b: &mut Block, n: &mut usize) {
    crate::desugar::each_stmt_mut(b, &mut |s| {
        let Stmt::For { name, iter, body, span, .. } = s else { return };
        let Expr::Call { callee, args, span: call_span, .. } = iter else { return };
        let Some((source, arity)) = source_of(callee) else { return };
        if args.len() != arity {
            return;
        }
        let k = *n;
        *n += 1;
        let user_body = std::mem::replace(body, Block { stmts: Vec::new(), tail: None, span: *span });
        let stmts = scan(source, std::mem::take(args), *call_span, &k.to_string(), name.clone(), user_body, *span);
        *s = Stmt::Block(Block { stmts, tail: None, span: *span });
    });
}

fn ident(name: &str, span: Span) -> Expr {
    Expr::Ident(Ident::new(name, span))
}

/// A call the compiler wrote: the callee at the call's own span.
fn call(segments: &[&str], args: Vec<Expr>, span: Span) -> Expr {
    let callee = if let [one] = segments {
        ident(one, span)
    } else {
        Expr::Path(QualifiedName { segments: segments.iter().map(|s| Ident::new(*s, span)).collect(), span })
    };
    Expr::Call { callee: Box::new(callee), args, span, id: NodeId::NONE }
}

fn let_(name: &str, is_mut: bool, value: Expr, span: Span) -> Stmt {
    Stmt::Let { is_mut, name: Ident::new(name, span), ty: None, value, span, id: NodeId::NONE }
}

/// The names a scan tagged `tag` binds: the view of the text, and the
/// current part's start and end offsets in it (each part is
/// `std::str::slice(view, lo, hi)`). A chain over a source reads them
/// after and during its scan (`crate::chains`).
pub fn scan_names(tag: &str) -> (String, String, String) {
    (format!("__hale_str_v{tag}"), format!("__hale_str_lo{tag}"), format!("__hale_str_hi{tag}"))
}

/// The statements of one scan over a source (the module's table): the
/// arguments' and the view's `let`s, then the `while` whose iteration
/// binds `name` to the part and runs `user_body`, a block of its own.
/// `tag` names the scan's bindings: a loop's number, or `c` and a
/// chain's (`crate::chains`, which splices the statements into its own
/// block and reads [`scan_names`] after them). `sp` is the source call's
/// span, `for_span` the loop's.
pub fn scan(source: &str, mut args: Vec<Expr>, sp: Span, tag: &str, name: Ident, user_body: Block, for_span: Span) -> Vec<Stmt> {
    let (v, lo, hi) = scan_names(tag);
    let at = format!("__hale_str_at{tag}");
    let text = argument_binding(source, "s", tag);
    let sep = argument_binding(source, "sep", tag);
    let mut outer = vec![let_(&text, false, args.remove(0), sp)];
    let with_sep = source == "split";
    if with_sep {
        outer.push(let_(&sep, false, args.remove(0), sp));
    }
    outer.push(let_(&v, false, call(&["std", "str", "bytes_view"], vec![ident(&text, sp)], sp), sp));
    // The hook arguments before the offset: the view, and split's separator.
    let lead = |with_sep: bool| {
        let mut a = vec![ident(&v, sp)];
        if with_sep {
            a.push(ident(&sep, sp));
        }
        a
    };
    let (first, end, next, inclusive) = match source {
        "split" => (call(&["__str_split_first"], lead(true), sp), "__str_split_end", "__str_split_next", true),
        "lines" => (Expr::Literal(Literal::Int(0), sp), "__str_line_end", "__str_line_next", false),
        _ => {
            let mut a = lead(false);
            a.push(Expr::Literal(Literal::Int(0), sp));
            (call(&["__str_word_start"], a, sp), "__str_word_end", "__str_word_start", false)
        }
    };
    outer.push(let_(&at, true, first, sp));
    let with = |e: &str| {
        let mut a = lead(with_sep);
        a.push(ident(e, sp));
        a
    };
    let view_n = Expr::Field { receiver: Box::new(ident(&v, sp)), name: Ident::new("n", sp), span: sp };
    let cond = Expr::Binary {
        op: if inclusive { BinOp::LtEq } else { BinOp::Lt },
        left: Box::new(ident(&at, sp)),
        right: Box::new(view_n),
        span: sp,
    };
    let body_span = user_body.span;
    let step = vec![
        let_(&lo, false, ident(&at, sp), sp),
        let_(&hi, false, call(&[end], with(&lo), sp), sp),
        Stmt::Assign {
            target: LValue { head: Ident::new(&at, sp), tail: Vec::new(), span: sp },
            op: AssignOp::Eq,
            value: call(&[next], with(&hi), sp),
            span: sp,
            id: NodeId::NONE,
        },
        Stmt::Let {
            is_mut: false,
            name,
            ty: None,
            value: call(&["std", "str", "slice"], vec![ident(&v, sp), ident(&lo, sp), ident(&hi, sp)], sp),
            span: sp,
            id: NodeId::NONE,
        },
        Stmt::Block(user_body),
    ];
    outer.push(Stmt::While { cond, body: Block { stmts: step, tail: None, span: body_span }, span: for_span });
    outer
}
