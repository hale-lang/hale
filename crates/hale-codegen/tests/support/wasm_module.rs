//! What a wasm module the tests built asks of its host, read from the
//! module itself.
//!
//! [`imports`] and [`exports`] read the binary's import section (id 2)
//! and export section (id 7) directly; no external tool prints them.
//! [`loader_writers`] reads the names the generated JS loader supplies
//! (its `writers` object) from the loader's own source, the `.mjs`
//! `link_wasm` writes beside the module, so the set is never transcribed
//! into a test. Every other import the loader meets it stubs as
//! `() => 0` (`codegen.rs`, `WASM_JS_LOADER`).
//!
//! [`outside_the_set`] is the import backstop's question (F.40 phase 3,
//! P3 T7; `notes/f40-capability-matrix.md` § 2.3): which imports of a
//! module are neither one of the loader's writers nor one of the
//! program's declared `@ffi("js")` names. Such an import is a symbol
//! that reached the link undefined, which `--allow-undefined` kept and
//! the loader would run as `() => 0`. It says nothing about semantics:
//! an inline stub imports nothing.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use hale_syntax::ast::{flat_decls, Program, TopDecl};

/// One entry of a module's import section.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Import {
    pub module: String,
    pub name: String,
    /// `func`, `table`, `memory`, `global` or `tag`.
    pub kind: &'static str,
}

fn leb(b: &[u8], at: &mut usize) -> Option<u32> {
    let (mut v, mut shift) = (0u32, 0);
    loop {
        let byte = *b.get(*at)?;
        *at += 1;
        v |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
    }
}

fn name(b: &[u8], at: &mut usize) -> Option<String> {
    let len = leb(b, at)? as usize;
    let s = String::from_utf8_lossy(b.get(*at..*at + len)?).into_owned();
    *at += len;
    Some(s)
}

fn limits(b: &[u8], at: &mut usize) -> Option<()> {
    let flag = *b.get(*at)?;
    *at += 1;
    leb(b, at)?;
    if flag & 1 == 1 {
        leb(b, at)?;
    }
    Some(())
}

/// The body of the section with `id`, or an empty slice when the module
/// has none; `None` when the bytes are not a wasm module.
fn section(bytes: &[u8], id: u8) -> Option<&[u8]> {
    if bytes.get(..4)? != b"\0asm" {
        return None;
    }
    let mut at = 8;
    while at < bytes.len() {
        let this = bytes[at];
        at += 1;
        let size = leb(bytes, &mut at)? as usize;
        let body = bytes.get(at..at + size)?;
        if this == id {
            return Some(body);
        }
        at += size;
    }
    Some(&[])
}

/// A wasm module's imports, in section order.
pub fn imports(bytes: &[u8]) -> Option<Vec<Import>> {
    let b = section(bytes, 2)?;
    if b.is_empty() {
        return Some(Vec::new());
    }
    let mut at = 0;
    let n = leb(b, &mut at)?;
    let mut out = Vec::new();
    for _ in 0..n {
        let module = name(b, &mut at)?;
        let field = name(b, &mut at)?;
        let kind = *b.get(at)?;
        at += 1;
        let kind = match kind {
            0 => {
                leb(b, &mut at)?;
                "func"
            }
            1 => {
                at += 1;
                limits(b, &mut at)?;
                "table"
            }
            2 => {
                limits(b, &mut at)?;
                "memory"
            }
            3 => {
                at += 2;
                "global"
            }
            4 => {
                at += 1;
                leb(b, &mut at)?;
                "tag"
            }
            _ => return None,
        };
        out.push(Import { module, name: field, kind });
    }
    Some(out)
}

/// A wasm module's export names, in section order.
pub fn exports(bytes: &[u8]) -> Option<Vec<String>> {
    let b = section(bytes, 7)?;
    if b.is_empty() {
        return Some(Vec::new());
    }
    let mut at = 0;
    let n = leb(b, &mut at)?;
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(name(b, &mut at)?);
        at += 1; // kind
        leb(b, &mut at)?;
    }
    Some(out)
}

/// The module's function names, by index, from the `name` custom
/// section's function-names subsection (wasm-ld writes it unless the
/// link strips it).
fn function_names(bytes: &[u8]) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    let mut at = 8;
    while at < bytes.len() {
        let id = bytes[at];
        at += 1;
        let Some(size) = leb(bytes, &mut at) else { break };
        let end = at + size as usize;
        if id == 0 {
            let mut p = at;
            if name(bytes, &mut p).as_deref() == Some("name") {
                while p < end {
                    let sub = bytes[p];
                    p += 1;
                    let Some(len) = leb(bytes, &mut p) else { break };
                    let sub_end = p + len as usize;
                    if sub == 1 {
                        let n = leb(bytes, &mut p).unwrap_or(0);
                        for _ in 0..n {
                            let (Some(idx), Some(s)) = (leb(bytes, &mut p), name(bytes, &mut p)) else { break };
                            out.insert(idx, s);
                        }
                    }
                    p = sub_end;
                }
            }
        }
        at = end;
    }
    out
}

/// Skip one instruction's immediates after its opcode; `None` on an
/// opcode this reader does not know (SIMD, atomics), which the module
/// a wasm test builds does not contain.
fn skip_immediates(b: &[u8], at: &mut usize, op: u8) -> Option<()> {
    match op {
        // block, loop, if: a block type (empty, a value type, or a type index).
        0x02..=0x04 => {
            let t = *b.get(*at)?;
            if t == 0x40 || (0x6f..=0x7f).contains(&t) {
                *at += 1;
            } else {
                leb(b, at)?;
            }
        }
        0x0c | 0x0d | 0x10 | 0x12 | 0x20..=0x26 | 0xd2 => {
            leb(b, at)?;
        }
        0x0e => {
            let n = leb(b, at)?;
            for _ in 0..=n {
                leb(b, at)?;
            }
        }
        0x11 | 0x13 => {
            leb(b, at)?;
            leb(b, at)?;
        }
        0x1c => {
            let n = leb(b, at)?;
            *at += n as usize;
        }
        0x28..=0x3e => {
            leb(b, at)?;
            leb(b, at)?;
        }
        0x3f | 0x40 | 0xd0 => *at += 1,
        0x41 | 0x42 => {
            leb(b, at)?;
        }
        0x43 => *at += 4,
        0x44 => *at += 8,
        0x00 | 0x01 | 0x05 | 0x0b | 0x0f | 0x1a | 0x1b | 0x45..=0xc4 | 0xd1 => {}
        0xfc => match leb(b, at)? {
            0..=7 => {}
            8 => {
                leb(b, at)?;
                *at += 1;
            }
            9 | 13 | 15..=17 => {
                leb(b, at)?;
            }
            10 => *at += 2,
            11 => *at += 1,
            12 | 14 => {
                leb(b, at)?;
                leb(b, at)?;
            }
            _ => return None,
        },
        _ => return None,
    }
    Some(())
}

/// For each imported function, the module's functions that call it,
/// by name (or `#<index>` when the name section lacks one), read from
/// the code section. `None` when the module is not one this reader
/// can decode.
pub fn import_callers(bytes: &[u8]) -> Option<BTreeMap<String, BTreeSet<String>>> {
    let imported: Vec<String> = imports(bytes)?.into_iter().filter(|i| i.kind == "func").map(|i| i.name).collect();
    let names = function_names(bytes);
    let code = section(bytes, 10)?;
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if code.is_empty() {
        return Some(out);
    }
    let mut at = 0;
    let n = leb(code, &mut at)?;
    for k in 0..n {
        let size = leb(code, &mut at)? as usize;
        let end = at + size;
        let body = code.get(at..end)?;
        let mut p = 0;
        let groups = leb(body, &mut p)?;
        for _ in 0..groups {
            leb(body, &mut p)?;
            p += 1;
        }
        let me = imported.len() as u32 + k;
        while p < body.len() {
            let op = body[p];
            p += 1;
            if op == 0x10 || op == 0x12 {
                let mut q = p;
                let callee = leb(body, &mut q)? as usize;
                if let Some(import) = imported.get(callee) {
                    let caller = names.get(&me).cloned().unwrap_or_else(|| format!("#{me}"));
                    out.entry(import.clone()).or_default().insert(caller);
                }
            }
            skip_immediates(body, &mut p, op)?;
        }
        at = end;
    }
    Some(out)
}

/// The names the generated loader supplies: the keys of its `const
/// writers = { … };` object, read from the loader's source. `None` when
/// the source has no such object.
pub fn loader_writers(loader: &str) -> Option<BTreeSet<String>> {
    let start = loader.find("const writers = {")? + "const writers = {".len();
    // The object's entries, comments dropped, split at the commas that
    // sit at its own depth (an arrow's parameters and body nest).
    let body: String = loader[start..]
        .lines()
        .map(|l| l.find("//").map_or(l, |c| &l[..c]))
        .collect::<Vec<_>>()
        .join("\n");
    let mut depth = 0i32;
    let mut entry = String::new();
    let mut out = BTreeSet::new();
    let take = |entry: &mut String, out: &mut BTreeSet<String>| {
        if let Some((key, _)) = entry.split_once(':') {
            let key = key.trim();
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.insert(key.to_string());
            }
        }
        entry.clear();
    };
    for c in body.chars() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '}' if depth == 0 => {
                take(&mut entry, &mut out);
                return Some(out);
            }
            '}' => depth -= 1,
            ',' if depth == 0 => {
                take(&mut entry, &mut out);
                continue;
            }
            _ => {}
        }
        entry.push(c);
    }
    None
}

/// The program's declared `@ffi("js")` names: the host imports its own
/// glue supplies through `run(glue)`.
pub fn ffi_js_names(program: &Program) -> BTreeSet<String> {
    flat_decls(&program.items)
        .filter_map(|i| match i {
            TopDecl::Fn(f) if f.ffi.as_ref().is_some_and(|a| a.abi == "js") => Some(f.name.name.clone()),
            _ => None,
        })
        .collect()
}

/// The imports of the module at `wasm` that are neither the writers of
/// the loader beside it (`wasm` with the `.mjs` extension) nor the
/// program's declared `@ffi("js")` names.
pub fn outside_the_set(program: &Program, wasm: &Path) -> Result<Vec<Import>, String> {
    let bytes = std::fs::read(wasm).map_err(|e| format!("{}: {e}", wasm.display()))?;
    let loader_path = wasm.with_extension("mjs");
    let loader = std::fs::read_to_string(&loader_path).map_err(|e| format!("{}: {e}", loader_path.display()))?;
    let writers = loader_writers(&loader).ok_or_else(|| format!("{}: no `const writers` object", loader_path.display()))?;
    let declared = ffi_js_names(program);
    let imports = imports(&bytes).ok_or_else(|| format!("{}: not a wasm module", wasm.display()))?;
    Ok(imports
        .into_iter()
        .filter(|i| !(i.kind == "func" && i.module == "env" && (writers.contains(&i.name) || declared.contains(&i.name))))
        .collect())
}

/// An import outside the set that a module may still carry, and why:
/// each is reached by a path that can run on wasm32, or is emitted by
/// codegen rather than the runtime's C, and so waits on a ruling
/// instead of being compiled out. `callers` names the only functions
/// allowed to reference it (the runtime's, which are fixed), or `None`
/// when the callers are the program's own generated functions. Each
/// entry is asserted to be still imported
/// (`wasm_import_backstop::every_known_open_import_is_still_imported`),
/// so a fix that closes one fails until its entry goes.
pub struct KnownOpen {
    pub name: &'static str,
    pub callers: Option<&'static [&'static str]>,
    pub why: &'static str,
}

const UNABSORBED_REPORT: &str = "generated code: the report of a violation no handler absorbs \
     (`fflush(stdout)`, `dprintf(2, ...)`, `exit(1)`) calls libc directly; it runs on wasm32 whenever \
     such a violation happens, and the loader's `() => 0` drops the message";
const OBSERVATION_PROBE: &str = "generated code: the observation probes, behind `lotus_obs_live`. \
     lotus_obs.c is not linked into a wasm32 module and Record/Replay are refused there, so the flag \
     (an undefined data symbol `--allow-undefined` resolves to address 0) reads 0 and the probes do \
     not run; the calls are codegen's, so the runtime's C cannot compile them out";

pub const KNOWN_OPEN: &[KnownOpen] = &[
    KnownOpen { name: "dprintf", callers: None, why: UNABSORBED_REPORT },
    KnownOpen { name: "fflush", callers: None, why: UNABSORBED_REPORT },
    KnownOpen {
        name: "fwrite",
        callers: Some(&["lotus_bus_hold_delivery", "lotus_bus_park_if_unready", "lotus_reclaim_defer", "lotus_replay_gate_cell"]),
        why: "the runtime's out-of-memory diagnostics before abort(): the shim's fprintf is an inline \
              no-op, but clang rewrites `fprintf(stderr, \"<literal>\")` into an fwrite nothing defines \
              (the arena is compiled without -fno-builtin). It runs on wasm32 when malloc fails. \
              (lotus_replay_gate_cell's runs only under replay, which wasm32 refuses.)",
    },
    KnownOpen { name: "lotus_obs_locus_birth", callers: None, why: OBSERVATION_PROBE },
    KnownOpen { name: "lotus_obs_locus_dissolve", callers: None, why: OBSERVATION_PROBE },
    KnownOpen { name: "lotus_obs_note_publisher", callers: None, why: OBSERVATION_PROBE },
    KnownOpen {
        name: "pthread_cond_broadcast",
        callers: Some(&["lotus_bus_quarantine_self", "lotus_bus_ready", "lotus_mailbox_drain_pending"]),
        why: "the readiness window's wake: lotus_bus_ready (and lotus_bus_ready_forget, inlined into \
              lotus_bus_quarantine_self) broadcast at every subscriber's readiness on wasm32. Its only \
              waiter, the cap wait, is compiled out there, so the `() => 0` wakes no one, but the call \
              runs. (lotus_mailbox_drain_pending's never runs: no mailbox exists on wasm32.)",
    },
];

/// The import backstop over one module a test built, named `origin`:
/// every import is a function the loader's writers supply or one of
/// the program's declared `@ffi("js")` names, or a [`KNOWN_OPEN`] one
/// referenced only by its stated callers. An import outside that is a
/// symbol that reached the link undefined and would run as `() => 0`;
/// the error names the program, the import and the functions calling
/// it.
pub fn backstop(origin: &str, program: &Program, wasm: &Path) -> Result<(), String> {
    let outside = outside_the_set(program, wasm)?;
    if outside.is_empty() {
        return Ok(());
    }
    let bytes = std::fs::read(wasm).map_err(|e| format!("{}: {e}", wasm.display()))?;
    let callers =
        import_callers(&bytes).ok_or_else(|| format!("{origin}: {}: the code section does not decode", wasm.display()))?;
    let mut unresolved = Vec::new();
    for i in outside {
        let by = callers.get(&i.name).cloned().unwrap_or_default();
        let known = KNOWN_OPEN.iter().find(|k| k.name == i.name && i.kind == "func");
        let admitted = known.is_some_and(|k| k.callers.is_none_or(|allowed| by.iter().all(|c| allowed.contains(&c.as_str()))));
        if !admitted {
            let by: Vec<&str> = by.iter().map(|s| s.as_str()).collect();
            unresolved.push(format!("  {}.{} ({}) <- {}", i.module, i.name, i.kind, by.join(", ")));
        }
    }
    if unresolved.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{origin}: the wasm module imports what neither the loader's writers nor the program's \
         `@ffi(\"js\")` names supply, so it reached the link undefined and the loader would run it \
         as `() => 0` (P3 T7):\n{}",
        unresolved.join("\n")
    ))
}
