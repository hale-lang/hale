//! GH #2134 (F.63): the build half of `@vectorize`.
//!
//! A fn marked `@vectorize` asserts that every innermost loop written in
//! its body is vectorized by the optimization pipeline the build runs,
//! on the build's target. The check never guesses from syntax: it reads
//! what LLVM's loop vectorizer reported about each loop.
//!
//! How the build knows (measured on LLVM 18 through inkwell; see F.63):
//!
//! - The vectorizer reports every innermost loop it looks at as an
//!   optimization remark: `vectorized loop (vectorization width: W,
//!   interleaved count: I)`, `interleaved loop (interleaved count: I)`
//!   (scalar, only unrolled), or `loop not vectorized[: reason]`. A remark
//!   reaches the context's diagnostic handler once the `-pass-remarks*`
//!   options name `loop-vectorize` ([`enable_vectorizer_remarks`]), and its
//!   description starts with the location of the loop (or of the
//!   instruction that stopped it) as `file:line:col`.
//! - Lowering gives every loop of a `@vectorize` fn a location the remark
//!   can be traced back from. A build that emits DWARF (`hale build`)
//!   already has the statement's real file, line and column, and a loop
//!   owns the remarks located inside its span. Otherwise, and for a
//!   Hale-source stdlib fn (which carries no DWARF), every instruction of
//!   the loop is located at line `n` of the synthetic file
//!   [`SYNTH_FILE`], `n` the loop's number; that debug info exists only for
//!   the check and is stripped before the object is written.
//! - An outer loop is never vectorized by LLVM's loop vectorizer and
//!   gets no remark: only the loops that contain no loop are held to the
//!   assertion. A loop the optimizer removed (folded, or fully unrolled)
//!   has no remark and nothing left to run scalar, and passes.
//!
//! The pipeline vectorizes at O2 and above, so a dev (O1) build, an
//! AddressSanitizer build (which skips the pipeline) and a wasm build
//! (no `simd128`) do not check the assertion; [`skipped`] says why.

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

use inkwell::llvm_sys;
use inkwell::llvm_sys::prelude::LLVMDiagnosticInfoRef;
use inkwell::llvm_sys::LLVMDiagnosticSeverity;

use crate::codegen::CodegenError;

/// The file the check's synthetic locations name.
pub(crate) const SYNTH_FILE: &str = "<hale-vectorize>";

static REMARKS_ON: Once = Once::new();

/// Turn on the loop vectorizer's remarks for every LLVM context in the
/// process. LLVM keeps the `-pass-remarks*` patterns in process-global
/// options, so this runs once, at the start of every build, before any
/// build of the process runs a pass: a later first call would write the
/// options while another build's pipeline reads them. Every build
/// installs [`Remarks`] on its context, so no remark reaches stderr.
pub(crate) fn enable_vectorizer_remarks() {
    REMARKS_ON.call_once(|| {
        let args = [
            "hale",
            "-pass-remarks=loop-vectorize",
            "-pass-remarks-missed=loop-vectorize",
            "-pass-remarks-analysis=loop-vectorize",
        ];
        let owned: Vec<CString> = args.iter().map(|a| CString::new(*a).expect("no NUL")).collect();
        let ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        let overview = CString::new("hale").expect("no NUL");
        // SAFETY: the argument strings outlive the call, and the call is
        // made once, before any pass of the process runs.
        unsafe {
            llvm_sys::support::LLVMParseCommandLineOptions(ptrs.len() as i32, ptrs.as_ptr(), overview.as_ptr());
        }
    });
}

/// The remarks one build's context received. Every other diagnostic is
/// printed as LLVM's default handler prints it, and an error still ends
/// the process as it did.
#[derive(Default)]
pub(crate) struct Remarks {
    /// Whether the build checks `@vectorize`: a build that does not
    /// drops every remark unread (they are made for every pass once the
    /// options are on, and only this build's check reads them).
    collect: AtomicBool,
    seen: Mutex<Vec<String>>,
}

impl Remarks {
    /// Install `self` as `context`'s diagnostic handler. `self` must
    /// outlive the context (declare it before the context).
    pub(crate) fn install(&self, context: &inkwell::context::Context) {
        // SAFETY: the handler reads `self` through the pointer for as long
        // as the context lives, which the caller guarantees `self` outlives.
        unsafe {
            llvm_sys::core::LLVMContextSetDiagnosticHandler(
                context.raw(),
                Some(on_diagnostic),
                self as *const Remarks as *mut c_void,
            );
        }
    }

    /// Keep the remarks from here on (the build checks `@vectorize`).
    pub(crate) fn collect(&self) {
        self.collect.store(true, Ordering::Relaxed);
    }

    pub(crate) fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.seen.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

extern "C" fn on_diagnostic(info: LLVMDiagnosticInfoRef, ctx: *mut c_void) {
    // SAFETY: LLVM hands a live diagnostic; `ctx` is the `Remarks`
    // `install` registered, alive for the context's lifetime.
    let remarks = unsafe { &*(ctx as *const Remarks) };
    let severity = unsafe { llvm_sys::core::LLVMGetDiagInfoSeverity(info) };
    if severity == LLVMDiagnosticSeverity::LLVMDSRemark && !remarks.collect.load(Ordering::Relaxed) {
        return;
    }
    let text = unsafe {
        let raw = llvm_sys::core::LLVMGetDiagInfoDescription(info);
        let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
        llvm_sys::core::LLVMDisposeMessage(raw);
        text
    };
    match severity {
        LLVMDiagnosticSeverity::LLVMDSRemark => {
            remarks.seen.lock().unwrap_or_else(|p| p.into_inner()).push(text);
        }
        LLVMDiagnosticSeverity::LLVMDSError => {
            eprintln!("error: {text}");
            std::process::exit(1);
        }
        LLVMDiagnosticSeverity::LLVMDSWarning => eprintln!("warning: {text}"),
        LLVMDiagnosticSeverity::LLVMDSNote => eprintln!("note: {text}"),
    }
}

/// Why this build does not check `@vectorize`, or `None` when it does.
pub fn skipped(dev_profile: bool, asan: bool, wasm: bool) -> Option<&'static str> {
    if wasm {
        Some("a wasm build is not built with `simd128`, so no loop of it vectorizes")
    } else if asan {
        Some("an AddressSanitizer build does not run the optimization pipeline")
    } else if dev_profile {
        Some("a dev build (`--dev`, `HALE_DEV`, `hale test`) optimizes at O1, which does not vectorize")
    } else {
        None
    }
}

/// Where one loop's remarks are located.
#[derive(Debug, Clone)]
pub(crate) enum LoopKey {
    /// Line `n` of [`SYNTH_FILE`].
    Synthetic(u32),
    /// The loop's real span: the file's name as its DIFile spells it, and
    /// the first and last (line, column).
    Real { file: String, start: (u32, u32), end: (u32, u32) },
}

/// One loop of a `@vectorize` fn, as lowering met it.
#[derive(Debug, Clone)]
pub(crate) struct VecLoop {
    pub(crate) span: hale_syntax::Span,
    /// The fn, as written (`f`, `Locus.m`).
    pub(crate) fn_name: String,
    pub(crate) min_width: Option<u32>,
    /// Contains no loop: the only kind the vectorizer looks at, and the
    /// only kind the assertion holds.
    pub(crate) innermost: bool,
    /// A Hale-source stdlib fn's loop (its span is in the stdlib's space).
    pub(crate) stdlib: bool,
    pub(crate) key: LoopKey,
}

/// A remark's location and message.
struct Remark<'a> {
    file: &'a str,
    line: u32,
    col: u32,
    msg: &'a str,
}

/// `file:line:col: message`, or `None` for a remark with no location.
fn parse(desc: &str) -> Option<Remark<'_>> {
    // The message follows the first `: ` that comes after `:<line>:<col>`.
    let mut from = 0;
    while let Some(i) = desc[from..].find(": ") {
        let at = from + i;
        let head = &desc[..at];
        let mut parts = head.rsplitn(3, ':');
        let col = parts.next()?;
        let line = parts.next();
        let file = parts.next();
        if let (Some(line), Some(file)) = (line, file) {
            if let (Ok(line), Ok(col)) = (line.parse::<u32>(), col.parse::<u32>()) {
                return Some(Remark { file, line, col, msg: &desc[at + 2..] });
            }
        }
        from = at + 2;
    }
    None
}

/// The loop a remark belongs to: the innermost loop whose location holds
/// it, checked or not (an outer loop's remark — one LLVM made after it
/// unrolled the inner loop away — is attributed to it, and dropped).
fn owner(loops: &[VecLoop], r: &Remark<'_>) -> Option<usize> {
    let mut best: Option<(usize, u64)> = None;
    for (i, l) in loops.iter().enumerate() {
        match &l.key {
            LoopKey::Synthetic(n) => {
                if r.file == SYNTH_FILE && r.line == *n {
                    return Some(i);
                }
            }
            LoopKey::Real { file, start, end } => {
                if r.file != file {
                    continue;
                }
                let at = (r.line, r.col);
                if at < *start || at > *end {
                    continue;
                }
                let size = u64::from(end.0 - start.0) << 32 | u64::from(end.1);
                if best.is_none_or(|(_, s)| size < s) {
                    best = Some((i, size));
                }
            }
        }
    }
    best.map(|(i, _)| i)
}

/// The vectorizer's reason, in the program's terms, with LLVM's words
/// beside it.
fn reason(msg: &str) -> String {
    let detail = msg.strip_prefix("loop not vectorized: ").unwrap_or(msg);
    let hale = if detail.contains("reorder floating-point operations") {
        "a `Float` sum or product whose order this target cannot keep in vector registers \
         (Hale adds floats in the written order, and this target has no in-order vector reduction)"
    } else if detail.contains("could not determine number of loop iterations") {
        "its trip count is not known when the loop starts (an exit that depends on what the loop computes)"
    } else if detail.contains("call instruction cannot be vectorized") {
        "a call that was not inlined and has no vector form"
    } else if detail.contains("vectorization is not beneficial") {
        "the target's cost model found no gain (an operation this target has no fast vector \
         instruction for, such as a 64-bit integer multiply on NEON)"
    } else if detail.contains("control flow cannot be substituted for a select") {
        "a branch in the body that cannot become a select (a conditional store, call or exit)"
    } else if detail.contains("could not be identified as reduction") {
        "a value carried out of the loop that is not a sum, product, minimum or maximum"
    } else if detail.contains("loop control flow is not understood") {
        "the loop has more than one exit or back edge"
    } else if detail.contains("cannot identify array bounds") || detail.contains("cannot check memory dependencies") {
        "memory it reads and writes that may overlap, and that cannot be checked at run time"
    } else if detail.contains("unsafe dependent memory operations") {
        "a value written in one iteration is read by a later one"
    } else if detail.contains("instruction cannot be vectorized") {
        "an operation with no vector form on this target"
    } else {
        return format!("LLVM: {detail}");
    };
    format!("{hale} (LLVM: {detail})")
}

/// A loop's verdict: `Ok` or why it is not vectorized.
fn verdict(l: &VecLoop, remarks: &[&str]) -> Result<(), String> {
    let mut widths: Vec<u32> = Vec::new();
    let mut failed = false;
    let mut why: Vec<String> = Vec::new();
    for m in remarks {
        if let Some(rest) = m.strip_prefix("vectorized loop (vectorization width: ") {
            // `4, interleaved count: 2)` or `vscale x 4, …`
            let w = rest.split(',').next().unwrap_or("");
            let w = w.rsplit(' ').next().unwrap_or(w);
            widths.push(w.trim().parse().unwrap_or(0));
        } else if m.starts_with("interleaved loop") {
            failed = true;
        } else if m.contains("already been vectorized") {
            // the scalar remainder of a loop it vectorized
        } else if *m == "loop not vectorized" {
            failed = true;
        } else if m.starts_with("loop not vectorized") || m.contains("vectorization is not beneficial") {
            failed = true;
            let r = reason(m);
            if !why.contains(&r) {
                why.push(r);
            }
        }
    }
    if failed {
        if why.is_empty() {
            why.push("LLVM gave no reason".to_string());
        }
        return Err(why.join("; "));
    }
    if let (Some(min), Some(&w)) = (l.min_width, widths.iter().min()) {
        if w < min {
            return Err(format!("it is vectorized {w} lanes wide, narrower than `min_width: {min}`"));
        }
    }
    Ok(())
}

/// Judge every innermost loop of the `@vectorize` fns against the
/// remarks the pipeline made. `site` renders a span as `file:line:col`
/// where the build can; `target` names the target (`aarch64 (apple-m1)`).
pub(crate) fn judge(
    loops: &[VecLoop],
    descriptions: &[String],
    target: &str,
    site: &dyn Fn(&VecLoop) -> Option<String>,
) -> Result<(), CodegenError> {
    let mut per_loop: Vec<Vec<&str>> = vec![Vec::new(); loops.len()];
    for d in descriptions {
        let Some(r) = parse(d) else { continue };
        if let Some(i) = owner(loops, &r) {
            per_loop[i].push(r.msg);
        }
    }
    let mut failures: Vec<(usize, String)> = Vec::new();
    for (i, l) in loops.iter().enumerate() {
        if !l.innermost {
            continue;
        }
        if let Err(why) = verdict(l, &per_loop[i]) {
            failures.push((i, why));
        }
    }
    let Some((first, why)) = failures.first() else { return Ok(()) };
    let l = &loops[*first];
    let mut msg = format!("`@vectorize` on `{}`: this loop does not vectorize on {target}: {why}", l.fn_name);
    for (i, why) in failures.iter().skip(1) {
        let other = &loops[*i];
        let at = site(other).map(|s| format!(" at {s}")).unwrap_or_default();
        msg.push_str(&format!("\n  and the loop{at} in `{}`: {why}", other.fn_name));
    }
    Err(CodegenError::NotVectorized(msg, l.span, l.stdlib))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lp(key: LoopKey) -> VecLoop {
        VecLoop {
            span: hale_syntax::Span::new(0, 1),
            fn_name: "f".to_string(),
            min_width: None,
            innermost: true,
            stdlib: false,
            key,
        }
    }

    #[test]
    fn a_remark_is_parsed_from_its_location() {
        let r = parse("k1.hl:20:9: loop not vectorized: instruction cannot be vectorized").unwrap();
        assert_eq!((r.file, r.line, r.col), ("k1.hl", 20, 9));
        assert_eq!(r.msg, "loop not vectorized: instruction cannot be vectorized");
        let r = parse("<hale-vectorize>:3:1: vectorized loop (vectorization width: 4, interleaved count: 2)").unwrap();
        assert_eq!((r.file, r.line), (SYNTH_FILE, 3));
        assert!(parse("loop not vectorized").is_none());
    }

    #[test]
    fn a_remark_belongs_to_the_innermost_loop_that_holds_it() {
        let loops = vec![
            lp(LoopKey::Real { file: "a.hl".into(), start: (2, 5), end: (9, 5) }),
            lp(LoopKey::Real { file: "a.hl".into(), start: (4, 9), end: (6, 9) }),
        ];
        let r = parse("a.hl:5:13: loop not vectorized").unwrap();
        assert_eq!(owner(&loops, &r), Some(1));
        let r = parse("a.hl:8:1: loop not vectorized").unwrap();
        assert_eq!(owner(&loops, &r), Some(0));
        let r = parse("b.hl:5:13: loop not vectorized").unwrap();
        assert_eq!(owner(&loops, &r), None);
    }

    #[test]
    fn the_verdicts() {
        let l = lp(LoopKey::Synthetic(1));
        assert!(verdict(&l, &["vectorized loop (vectorization width: 2, interleaved count: 4)"]).is_ok());
        assert!(verdict(&l, &[]).is_ok(), "a loop the optimizer removed");
        let e = verdict(&l, &["interleaved loop (interleaved count: 4)", "the cost-model indicates that vectorization is not beneficial"])
            .unwrap_err();
        assert!(e.contains("cost model found no gain"), "{e}");
        let e = verdict(&l, &["loop not vectorized", "loop not vectorized: cannot prove it is safe to reorder floating-point operations"])
            .unwrap_err();
        assert!(e.contains("`Float` sum"), "{e}");
        let mut w = lp(LoopKey::Synthetic(1));
        w.min_width = Some(4);
        let e = verdict(&w, &["vectorized loop (vectorization width: 2, interleaved count: 4)"]).unwrap_err();
        assert!(e.contains("2 lanes wide, narrower than `min_width: 4`"), "{e}");
        assert!(verdict(&w, &["vectorized loop (vectorization width: vscale x 4, interleaved count: 1)"]).is_ok());
    }
}
