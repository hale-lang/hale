//! A standing check (F.40 phase 4): an idle reclaim makes no out-of-line
//! runtime call it does not need.
//!
//! Phase 3 shipped a reclaim protocol (the claim, the reclaim scope, the
//! cancel before the arena's destroy) whose runtime calls ran on every
//! reclaim, including one with nothing queued, held or deferred: the
//! bench's `locus_instantiation` went from 414 to 611 instructions per
//! birth+dissolve cycle, 197 of them in six calls that each returned at
//! once (`lotus_run_cancel` 40, `lotus_reclaim_defer` 36,
//! `lotus_reclaim_release_enter` 20, `lotus_reclaim_flush_owned` 18,
//! `lotus_reclaim_scope_enter` 15, `lotus_reclaim_pending` 14). The fix
//! put each behind an inline guard (`emit_unless_idle` and
//! `emit_reclaim_pending` in `src/locus/dissolve.rs`; the guards' words
//! are pinned by `reclaim_cancel_ir.rs`). The count of those calls was
//! readable in the IR days before a timing showed it; this test reads it.
//!
//! The reader runs no program. It follows ONE path through the
//! pre-optimization IR of the function holding the dissolve site: the
//! path a cycle takes when the runtime is idle. Every runtime word and
//! observation flag (`@lotus_*`, `@lotus.*`) reads zero, so each guard's
//! idle test holds and the walk takes its idle edge; the instance's own
//! words read what its birth stored (stores and loads are followed
//! through `alloca`s and field addresses); a call of a generated
//! function (`__*`) is walked into; a call of a declared function is
//! counted by name. A branch whose condition the walk cannot decide
//! fails the test with the branch, so the reader never guesses.
//!
//! The pin is the runtime calls the walk makes from the first call of a
//! reclaim function (`__reclaim*`) to the function's return: the
//! reclaim, plus what the helper does after it (its own frame arena's
//! destroy). A call that appears there is a protocol step paid on every
//! idle cycle; moving a pin on purpose is an edit of its constant, with
//! the reason in the commit.

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

use std::collections::{BTreeMap, HashMap};

/// The bench's shape: a `let`-bound locus born and dissolved in a
/// helper, with nothing queued, held or deferred.
const LET_BOUND: &str = "locus Empty {
    params { v: Int = 0; }
    fn read() -> Int { return self.v; }
}

fn instantiate_one(seed: Int) -> Int {
    let e = Empty { v: seed };
    return e.read();
}

fn main() {
    let mut i = 0;
    let mut sink = 0;
    while i < 1000 {
        sink = sink ^ instantiate_one(i);
        i = i + 1;
    }
    println(sink);
}
";

/// The same with a child field: the cascade's scope, the child's skip
/// and the child's reclaim under its owner.
const CHILD_FIELD: &str = "locus Inner {
    params { n: Int = 0; }
}

locus Outer {
    params { v: Int = 0; k: Inner = Inner { }; }
    fn read() -> Int { return self.v; }
}

fn instantiate_one(seed: Int) -> Int {
    let o = Outer { v: seed };
    return o.read();
}

fn main() {
    let mut i = 0;
    let mut sink = 0;
    while i < 1000 {
        sink = sink ^ instantiate_one(i);
        i = i + 1;
    }
    println(sink);
}
";

/// A statement literal: born and reclaimed at once (the eager teardown).
const STATEMENT: &str = "locus Empty {
    params { v: Int = 0; }
    fn read() -> Int { return self.v; }
}

fn instantiate_one(seed: Int) {
    Empty { v: seed };
}

fn main() {
    let mut i = 0;
    while i < 1000 {
        instantiate_one(i);
        i = i + 1;
    }
    println(i);
}
";

/// The idle path's runtime calls, by name and count, as measured
/// (2026-10-05, on F.40 phase 4's main). An arena-elided `Empty` (its
/// struct in the helper's frame) releases its struct; the helper then
/// destroys its own frame arena.
const LET_BOUND_IDLE: &[(&str, usize)] = &[("lotus_arena_destroy", 1), ("lotus_child_struct_release", 1)];
/// `Outer` has an arena of its own: its destroy, and its bus
/// deregistration (`lotus_bus_quarantine_self`, emitted for a locus
/// with an arena in a program with a bus). Not guarded: a subscription
/// is not a ticket, so the idle words cannot tell a subscriber with
/// nothing queued from a locus with no subscription; see
/// `fixtures/lifecycle/l11_idle_reclaim_deregisters.hl`. The field
/// `k` is released under its owner through `lotus_reclaim_request`,
/// which calls the child's release (walked: its guards hold too); each
/// struct is released; the helper's frame arena is destroyed.
const CHILD_FIELD_IDLE: &[(&str, usize)] = &[
    ("lotus_arena_destroy", 2),
    ("lotus_bus_quarantine_self", 1),
    ("lotus_child_struct_release", 2),
    ("lotus_reclaim_request", 1),
];
/// The eager teardown of a statement literal: as the `let`'s.
const STATEMENT_IDLE: &[(&str, usize)] = &[("lotus_arena_destroy", 1), ("lotus_child_struct_release", 1)];

#[test]
fn an_idle_let_bound_reclaim_calls_only_its_release() {
    assert_idle_calls("let_bound", "Empty", LET_BOUND, LET_BOUND_IDLE);
}

#[test]
fn an_idle_child_field_reclaim_calls_only_its_releases() {
    assert_idle_calls("child_field", "Outer", CHILD_FIELD, CHILD_FIELD_IDLE);
}

#[test]
fn an_idle_statement_literal_reclaim_calls_only_its_release() {
    assert_idle_calls("statement", "Empty", STATEMENT, STATEMENT_IDLE);
}

fn assert_idle_calls(tag: &str, locus: &str, src: &str, want: &[(&str, usize)]) {
    let bin = harness::unique_bin(&format!("reclaim_idle_calls_{tag}"));
    let ir = harness::build_source_ir_text(src, &bin).unwrap_or_else(|e| panic!("{tag}: build: {e:?}"));
    let _ = std::fs::remove_file(&bin);
    let got = idle_reclaim_calls(&ir, "instantiate_one");
    let want: BTreeMap<String, usize> = want.iter().map(|(n, c)| (n.to_string(), *c)).collect();
    if got == want {
        return;
    }
    let mut moved = Vec::new();
    for (name, n) in &got {
        match want.get(name) {
            None => moved.push(format!("the idle reclaim of `{locus}` now calls `{name}` ({n}x)")),
            Some(w) if w != n => moved.push(format!("the idle reclaim of `{locus}` calls `{name}` {n}x, pinned {w}x")),
            _ => {}
        }
    }
    for (name, w) in &want {
        if !got.contains_key(name) {
            moved.push(format!("the idle reclaim of `{locus}` no longer calls `{name}` (pinned {w}x)"));
        }
    }
    panic!(
        "{tag}: {}.\n\
         Phase 3 paid 197 instructions per birth+dissolve cycle for six such calls, each returning at once \
         (lotus_run_cancel, lotus_reclaim_defer, lotus_reclaim_release_enter, lotus_reclaim_flush_owned, \
         lotus_reclaim_scope_enter, lotus_reclaim_pending); a runtime call of the reclaim belongs behind a guard \
         that skips it when its words say there is nothing to do (`emit_unless_idle`, src/locus/dissolve.rs). \
         A call that is meant to be there moves the pin: edit `{}_IDLE` in this file, with the reason in the commit.\n\
         measured: {got:?}\npinned:   {want:?}",
        moved.join("; "),
        tag.to_uppercase(),
    )
}

// ---------------------------------------------------------------------
// The reader: one path through the pre-optimization IR, idle runtime.
// ---------------------------------------------------------------------

/// A value on the idle path.
#[derive(Clone, Debug, PartialEq)]
enum V {
    Int(i128),
    Null,
    /// A non-null pointer: an `alloca`, a global, a field address, or
    /// what an allocator returned. The key names the memory it points at.
    Ptr(String),
    /// Something the walk does not know (a call's answer, a parameter);
    /// branching on it fails the test.
    Unknown(String),
}

struct Func<'a> {
    /// Each parameter's type and name.
    params: Vec<(&'a str, &'a str)>,
    /// Label, then the block's instructions (metadata stripped).
    blocks: Vec<(&'a str, Vec<&'a str>)>,
}

struct Walk<'a> {
    defined: HashMap<&'a str, Func<'a>>,
    memory: HashMap<String, V>,
    frames: usize,
    steps: usize,
    in_reclaim: bool,
    calls: BTreeMap<String, usize>,
}

/// The runtime calls the idle path through `entry` makes from its first
/// call of a reclaim function on.
fn idle_reclaim_calls(ir: &str, entry: &str) -> BTreeMap<String, usize> {
    let mut walk = Walk {
        defined: functions(ir),
        memory: HashMap::new(),
        frames: 0,
        steps: 0,
        in_reclaim: false,
        calls: BTreeMap::new(),
    };
    // A pointer parameter is the caller's arena: non-null.
    let params = &walk.defined.get(entry).unwrap_or_else(|| panic!("`{entry}` is not defined")).params;
    let args = params
        .iter()
        .map(|(ty, p)| if *ty == "ptr" { V::Ptr(format!("{entry}:{p}")) } else { V::Unknown(format!("{entry}:{p}")) })
        .collect();
    walk.call(entry, args);
    assert!(walk.in_reclaim, "the idle path through `{entry}` reaches no reclaim function");
    walk.calls
}

/// Every `define`d function: its parameters and its blocks.
fn functions(ir: &str) -> HashMap<&str, Func<'_>> {
    let mut out = HashMap::new();
    let mut lines = ir.lines();
    while let Some(line) = lines.next() {
        if !line.starts_with("define ") {
            continue;
        }
        let at = line.find(" @").expect("a defined name") + 2;
        let open = at + line[at..].find('(').expect("a parameter list");
        let name = line[at..open].trim_matches('"');
        let close = open + matching_paren(&line[open..]);
        let params = split_top(&line[open + 1..close])
            .into_iter()
            .filter_map(|p| {
                let ty = p.split_whitespace().next()?;
                Some((ty, p.split_whitespace().last().filter(|t| t.starts_with('%'))?))
            })
            .collect();
        let mut blocks: Vec<(&str, Vec<&str>)> = vec![("", Vec::new())];
        for l in lines.by_ref() {
            if l == "}" {
                break;
            }
            if !l.starts_with(' ') && !l.is_empty() {
                if let Some((label, _)) = l.split_once(':') {
                    blocks.push((label.trim_matches('"'), Vec::new()));
                }
                continue;
            }
            let l = l.trim();
            if l.is_empty() || l.starts_with(';') || l.starts_with("#dbg_") {
                continue;
            }
            blocks.last_mut().expect("a block").1.push(strip_metadata(l));
        }
        if blocks[0].0.is_empty() && blocks[0].1.is_empty() {
            blocks.remove(0);
        }
        out.insert(name, Func { params, blocks });
    }
    out
}

/// An instruction without its trailing `, !dbg !N` / `, !prof !N`.
fn strip_metadata(l: &str) -> &str {
    match l.find(", !") {
        Some(i) => &l[..i],
        None => l,
    }
}

/// The index of the `)` closing the `(` that `s` starts with.
fn matching_paren(s: &str) -> usize {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced: {s}")
}

/// `s` split at its top-level commas.
fn split_top(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    if !s[start..].trim().is_empty() {
        out.push(s[start..].trim());
    }
    out
}

/// The operand at the end of a typed operand (`i64 %x`, `ptr null`).
fn last_token(s: &str) -> &str {
    s.split_whitespace().last().unwrap_or("")
}

impl<'a> Walk<'a> {
    /// Walk `name` with `args`; its return value.
    fn call(&mut self, name: &str, args: Vec<V>) -> V {
        let frame = self.frames;
        self.frames += 1;
        let func = self.defined.get(name).unwrap_or_else(|| panic!("`{name}` is not defined"));
        let mut env: HashMap<&str, V> = HashMap::new();
        for ((_, p), a) in func.params.iter().zip(args) {
            env.insert(p, a);
        }
        let blocks: Vec<(&'a str, Vec<&'a str>)> = func.blocks.iter().map(|(l, b)| (*l, b.clone())).collect();
        let index: HashMap<&str, usize> = blocks.iter().enumerate().map(|(i, (l, _))| (*l, i)).collect();
        let (mut at, mut from) = (0usize, "");
        loop {
            let (label, body) = &blocks[at];
            let mut next: Option<&str> = None;
            for &ins in body {
                self.steps += 1;
                assert!(self.steps < 200_000, "the idle path through `{name}` does not end");
                let (dest, rhs) = match ins.split_once(" = ") {
                    Some((d, r)) if d.starts_with('%') => (Some(d), r),
                    _ => (None, ins),
                };
                let op = rhs.split_whitespace().next().unwrap_or("");
                let value = match op {
                    "br" => {
                        next = Some(self.branch(name, label, rhs, &env));
                        break;
                    }
                    "ret" => {
                        let v = rhs.strip_prefix("ret ").unwrap_or("");
                        return if v == "void" { V::Unknown("void".into()) } else { self.operand(last_token(v), &env, frame) };
                    }
                    "unreachable" | "switch" | "indirectbr" | "resume" | "invoke" => {
                        panic!("`{name}` `{label}`: the idle path reaches `{rhs}`, which the reader does not follow")
                    }
                    "alloca" => V::Ptr(format!("f{frame}:{}", dest.expect("named"))),
                    "store" => {
                        self.store(rhs, &env, frame);
                        continue;
                    }
                    "load" => self.load(rhs, dest.expect("named"), &env, frame),
                    "getelementptr" => self.gep(rhs, &env, frame),
                    "icmp" => self.icmp(rhs, &env, frame),
                    "and" | "or" | "xor" | "add" | "sub" => self.arith(op, rhs, &env, frame),
                    "phi" => self.phi(rhs, from, &env, frame),
                    "zext" | "sext" | "trunc" | "bitcast" | "ptrtoint" | "inttoptr" => {
                        let v = rhs.split_once(' ').map(|x| x.1).unwrap_or("");
                        let v = v.split(" to ").next().unwrap_or("");
                        self.operand(last_token(v), &env, frame)
                    }
                    "call" | "tail" | "musttail" | "notail" => self.call_site(rhs, &env, frame),
                    _ => V::Unknown(format!("`{rhs}`")),
                };
                if let Some(d) = dest {
                    env.insert(d, value);
                }
            }
            let next = next.unwrap_or_else(|| panic!("`{name}` `{label}` has no terminator"));
            from = label;
            at = *index.get(next).unwrap_or_else(|| panic!("`{name}` has no block `{next}`"));
        }
    }

    fn operand(&self, tok: &str, env: &HashMap<&str, V>, _frame: usize) -> V {
        let tok = tok.trim();
        match tok {
            "null" => V::Null,
            "true" => V::Int(1),
            "false" => V::Int(0),
            _ if tok.starts_with('%') => env.get(tok).cloned().unwrap_or_else(|| V::Unknown(format!("{tok} undefined"))),
            _ if tok.starts_with('@') => V::Ptr(tok.to_string()),
            _ => tok.parse::<i128>().map(V::Int).unwrap_or_else(|_| V::Unknown(tok.to_string())),
        }
    }

    /// `br label %x` or `br i1 %c, label %t, label %f`: the label taken.
    fn branch(&self, name: &str, label: &str, rhs: &'a str, env: &HashMap<&str, V>) -> &'a str {
        let labels: Vec<&str> = rhs.split("label %").skip(1).map(|l| l.trim_end_matches([',', ' ']).trim_matches('"')).collect();
        if labels.len() == 1 {
            return labels[0];
        }
        let cond = rhs["br i1 ".len()..].split(',').next().expect("a condition");
        match self.operand(cond, env, 0) {
            V::Int(0) => labels[1],
            V::Int(_) => labels[0],
            other => panic!(
                "`{name}` `{label}`: the idle path cannot decide `{rhs}` ({cond} is {other:?}); \
                 teach the reader what an idle runtime answers there"
            ),
        }
    }

    fn store(&mut self, rhs: &str, env: &HashMap<&str, V>, frame: usize) {
        let body = rhs.trim_start_matches("store ").trim_start_matches("atomic ");
        let (value, dest) = body.split_once(", ptr ").unwrap_or_else(|| panic!("a store: {rhs}"));
        let dest = dest.split([',', ' ']).next().expect("a destination");
        let value = self.operand(last_token(value), env, frame);
        if let V::Ptr(key) = self.operand(dest, env, frame) {
            self.memory.insert(key, value);
        }
    }

    /// A load reads what was stored there; a runtime word or flag no
    /// store reached reads zero (idle); anything else is unknown.
    fn load(&self, rhs: &str, dest: &str, env: &HashMap<&str, V>, frame: usize) -> V {
        let body = rhs.trim_start_matches("load ").trim_start_matches("atomic ");
        let (ty, src) = body.split_once(", ptr ").unwrap_or_else(|| panic!("a load: {rhs}"));
        let src = src.split([',', ' ']).next().expect("a source");
        let V::Ptr(key) = self.operand(src, env, frame) else {
            return V::Unknown(format!("a load through {src}"));
        };
        if let Some(v) = self.memory.get(&key) {
            return v.clone();
        }
        let runtime = key.starts_with("@lotus_") || key.starts_with("@lotus.");
        match (runtime, ty.trim()) {
            (true, "ptr") => V::Ptr(format!("{key}*")),
            (true, _) => V::Int(0),
            (false, _) => V::Unknown(format!("{dest}: nothing stored at {key}")),
        }
    }

    /// A field address: its base's key and the constant indices.
    fn gep(&self, rhs: &str, env: &HashMap<&str, V>, frame: usize) -> V {
        let parts = split_top(rhs.split_once(' ').map(|x| x.1).unwrap_or(""));
        let base = parts.iter().position(|p| p.starts_with("ptr ")).expect("a base");
        let mut key = match self.operand(last_token(parts[base]), env, frame) {
            V::Ptr(k) => k,
            other => return V::Unknown(format!("a field of {other:?}")),
        };
        for idx in &parts[base + 1..] {
            match self.operand(last_token(idx), env, frame) {
                V::Int(i) => key.push_str(&format!("/{i}")),
                _ => key.push_str("/?"),
            }
        }
        V::Ptr(key)
    }

    fn icmp(&self, rhs: &str, env: &HashMap<&str, V>, frame: usize) -> V {
        let mut words = rhs.split_whitespace().skip(1);
        let pred = words.next().expect("a predicate");
        let ops = split_top(rhs.splitn(4, ' ').nth(3).expect("operands"));
        let (a, b) = (self.operand(last_token(ops[0]), env, frame), self.operand(last_token(ops[1]), env, frame));
        let truth = match (&a, &b) {
            (V::Int(x), V::Int(y)) => match pred {
                "eq" => x == y,
                "ne" => x != y,
                "slt" | "ult" => x < y,
                "sle" | "ule" => x <= y,
                "sgt" | "ugt" => x > y,
                "sge" | "uge" => x >= y,
                _ => return V::Unknown(format!("icmp {pred}")),
            },
            (V::Ptr(_), V::Null) | (V::Null, V::Ptr(_)) => pred == "ne",
            (V::Null, V::Null) => pred == "eq",
            _ => return V::Unknown(format!("icmp {pred} of {a:?} and {b:?}")),
        };
        V::Int(truth as i128)
    }

    fn arith(&self, op: &str, rhs: &str, env: &HashMap<&str, V>, frame: usize) -> V {
        let ops = split_top(rhs.split_once(' ').map(|x| x.1).unwrap_or(""));
        match (self.operand(last_token(ops[0]), env, frame), self.operand(last_token(ops[1]), env, frame)) {
            (V::Int(x), V::Int(y)) => V::Int(match op {
                "and" => x & y,
                "or" => x | y,
                "xor" => x ^ y,
                "add" => x + y,
                _ => x - y,
            }),
            (a, b) => V::Unknown(format!("{op} of {a:?} and {b:?}")),
        }
    }

    fn phi(&self, rhs: &str, from: &str, env: &HashMap<&str, V>, frame: usize) -> V {
        for arm in rhs.split('[').skip(1) {
            let arm = arm.split(']').next().unwrap_or("");
            let (v, pred) = arm.split_once(", %").expect("a phi arm");
            if pred.trim().trim_matches('"') == from {
                return self.operand(v, env, frame);
            }
        }
        panic!("a phi with no arm from `{from}`: {rhs}")
    }

    /// A call: a generated function (`__*`) is walked into; a declared
    /// one is counted once the reclaim has begun; a user function is
    /// stepped over (its answer unknown).
    fn call_site(&mut self, rhs: &str, env: &HashMap<&str, V>, frame: usize) -> V {
        let Some(at) = rhs.find('@') else {
            panic!("an indirect call on the idle path: {rhs}");
        };
        let open = at + rhs[at..].find('(').expect("an argument list");
        let callee = rhs[at + 1..open].trim_matches('"');
        if callee.starts_with("llvm.") {
            return V::Unknown(callee.to_string());
        }
        let close = open + matching_paren(&rhs[open..]);
        let args: Vec<V> =
            split_top(&rhs[open + 1..close]).into_iter().map(|a| self.operand(last_token(a), env, frame)).collect();
        if callee.starts_with("__reclaim") {
            self.in_reclaim = true;
        }
        if self.defined.contains_key(callee) {
            return if callee.starts_with("__") { self.call(callee, args) } else { V::Unknown(format!("{callee}()")) };
        }
        if self.in_reclaim {
            *self.calls.entry(callee.to_string()).or_default() += 1;
        }
        // `lotus_reclaim_request(child, owner, release)` sets the owner
        // hint and calls `release(child)`: the child's guarded release
        // runs inside it, so the walk follows it there.
        if callee == "lotus_reclaim_request" {
            match &args[2] {
                V::Ptr(f) if self.defined.contains_key(&f[1..]) => {
                    let f = f[1..].to_string();
                    self.call(&f, vec![args[0].clone()]);
                }
                other => panic!("lotus_reclaim_request's release is not an emitted function: {other:?}"),
            }
        }
        // What a declared function answers: an allocator's pointer is
        // non-null; anything else is not known.
        let ret = rhs[..at].split_whitespace().rev().find(|w| *w != "(ptr," && !w.starts_with('(')).unwrap_or("");
        if ret == "ptr" {
            V::Ptr(format!("f{frame}:{callee}#{}", self.steps))
        } else {
            V::Unknown(format!("{callee}()"))
        }
    }
}
