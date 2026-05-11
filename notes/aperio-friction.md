# Aperio friction log — global

> Append-only. Each entry is a real moment where the language
> got in the way of writing what should be a correct program.
> The compiler session reads this file at the start of each
> iteration and uses it to triangulate priorities for the next
> milestone.

This is the **global** friction log — entries that came up
across sessions, or that don't belong to any one app. Per-app
logs live at `apps/<name>/FRICTION.md`.

## Format

Each entry is one Markdown section, dated, with a short tag.
Don't reformat or rewrite earlier entries. Append.

```
## YYYY-MM-DD <short-tag>

**Source:** <session or app name>
**Tried:** <one sentence: what you wanted to write>
**Hit:** <one sentence: what happened — error message, missing primitive, surprising semantics>
**Workaround:** <one sentence: what you did instead, or "blocked">
**Why it matters:** <one sentence: what feature this gates, or "minor papercut">
```

## What counts as friction

A friction entry describes a moment where the language or
stdlib resisted writing a program a competent reader would
agree should be writable. Three flavours:

- **Missing primitive.** "I needed X; X does not exist."
- **Surprising semantics.** "I wrote what I thought was right;
  it compiled but did the wrong thing." (Especially valuable.)
- **Friction in shape.** "I wrote what I needed; it works; but
  the path I took feels wrong, and I want a record of it
  before the next person rediscovers it."

What is **not** a friction entry:

- A bug in your own program logic that the compiler caught.
- A stylistic preference (e.g., "I wish `let` was `var`").
- A general feature wish disconnected from a specific moment of
  resistance ("Aperio should have generics" — yes, we know;
  log when generics' absence blocked a *concrete program*).
- A bug report against the compiler (file those as compiler
  issues, not friction).

## Entries

<!-- Append below this line. Do not edit existing entries. -->

## 2026-05-10 cross-locus-return-deep-copy [FIXED same session]

**Source:** corpus-extraction migration (tower-join, operational-graph)
**Tried:** End a free fn with `return jb.wrap_array(inner);` after the body called another locus method (e.g. `ta.each_body(acc, tag)`).
**Hit:** Caller observes `""` for the returned String. Standalone callsites of `jb.wrap_array("")` work fine; the bug triggers only when the fn first calls a *different* sub-locus's method, then returns the second method's result directly. Reproduced minimally with `let bodies = ta.each_body(...); return jb.wrap_array("");`.
**Workaround:** None needed — fixed in the m49 free-fn epilogue (`emit_fn_exit_epilogue` in `crates/aperio-codegen/src/codegen.rs`). The epilogue used to run `flush_dissolve_frame()` BEFORE the return-value deep-copy; this freed any let-bound sub-locus arenas while `ret_alloca` still pointed into one of them, so the subsequent `lotus_str_clone` read freed memory (the freshly-freed page often still contained the right bytes for a single-locus fn, hiding the bug; a second sub-locus dissolve was enough to clobber the chunk). Fix swaps the order: deep-copy first (caller_arena is the parent caller's region, unaffected by this fn's flush), then flush, then destroy the per-call subregion. Returning `jb.wrap_array(...)` directly now round-trips through the caller boundary correctly.
**Why it matters:** Cross-locus composition is the std seed's whole point. The fix lets `__collect_types_with_motion` (apps/tower-join) and `__collect_section` (apps/operational-graph) return `jb.wrap_array(arr)` directly instead of inline `[ + arr + ]`. Verified byte-identical fixture output for both apps and all 312 workspace tests stay green.

## 2026-05-10 sink-as-tagged-locus [FIXED 2026-05-11 — F.20 Phase A (2026-05-10) + Phase B (2026-05-11) + Sink stdlib migration (2026-05-11)]

**Source:** ferryman render OOM fix
**Tried:** Define `std::text::Sink` with multiple destination implementations (stdout streaming, in-memory buffer for tests, eventually file) so the renderer threads `sink: Sink` through the walk and writes rows as produced instead of returning concatenated Strings up the recursion.
**Hit:** Aperio v0 has no interfaces / traits. The next-closest shape is one locus with a `dest: String` param branching inside every method: `if self.dest == "string" { self.buf = self.buf + s; } else { print(s); }`. Adding a third destination edits every method; the type system can't see which destinations exist; unused params (`buf` in stdout mode) sit in every instance.
**Workaround:** Wrote the tagged-locus version. Functionally correct, ergonomically poor.
**Why it matters:** Sink-shape polymorphism keeps recurring — `std::log::StdoutSink` is bus-coupled because there was no other way to abstract a destination; the renderer's OOM was caused by the same gap pushing it toward in-memory String accumulation. A real interface mechanism would let StdoutSink / StringSink / FileSink coexist as separate loci with one surface, eliminating the inner dispatch entirely.
**Phase A resolution:** F.20 ships the structural-interface declaration (`interface Sink { fn write(s: String); ... }`) plus the structural-impl rule enforced at every call site where a fn declares an interface-typed param (typechecker fires "locus X does not satisfy interface Y: missing method Z" / arity / type / return-type diagnostics). Tests in `crates/aperio-types/src/lib.rs` cover the satisfying / missing-method / arity-mismatch cases. Library design can proceed against the locked syntax. **Phase B (codegen vtable dispatch) still pending** — until then, calling a fn with an interface-typed param errors at codegen time with a friendly message pointing at the next milestone. The Sink migration in stdlib waits for Phase B; the typecheck-only ship lets future code design with interfaces without a binary-build path yet.
**Phase B resolution (2026-05-11):** Interface values lower as fat pointers `{data, vtable}` arena-allocated at the coercion site; per-(locus, interface) static globals `__vt.<locus>.<iface>` hold fn pointers in interface-method-decl order; method calls on an interface receiver indirect through `vtable[i]` with the data pointer as the implicit self arg (m80 `build_indirect_call` machinery). End-to-end coverage in `crates/aperio-codegen/tests/interface_dispatch.rs`. Interface values are usable as fn params + method-call receivers; cross-arena uses (returning, storing in locus fields, arrays/tuples of interfaces) are a Phase B follow-up — the data pointer inside the fat pointer would dangle without deep-copy.
**Sink stdlib migration (2026-05-11):** `__StdTextSink` is now a structural interface (`fn write(s: String); fn line(s: String); fn newline();`) with three concrete implementations — `__StdTextStdoutSink`, `__StdTextStringSink` (carries a buf, exposes `result() -> String`), `__StdTextFileSink` (uses `std::io::fs::write_file_append` for streaming append). User-facing paths: `std::text::{Sink, StdoutSink, StringSink, FileSink}`. End-to-end coverage in `crates/aperio-codegen/tests/sink_polymorphism.rs`. Source-incompatible change: existing callers using `std::text::Sink { dest: "stdout" }` need to use `std::text::StdoutSink { }` (one breaking call site in `apps/ferryman/main.ap`, owned by the app-dev session). Codegen surfaces the F.20-Phase-B interface-type resolution for path-qualified stdlib interfaces (`type_expr_to_codegen_ty` consults `user_interfaces` alongside `user_loci` / `user_types`).

## 2026-05-10 reader-list_item-quadratic-concat

**Source:** ferryman render against grease (45k-line skeleton, 36 binaries)
**Tried:** Render the grease-skeleton yaml end-to-end with the Sink-streamed renderer + the new in-order Reader cursor cache. Memory is bounded on the renderer side (Sink streams to stdout) and walk count is bounded on the Reader side (cursor turns N list_item calls into O(N) total).
**Hit:** Segfault at ~3.9GB RSS in ~1.8s on the 36-binary skeleton. Same ceiling without the cursor cache. Scale test: 1 binary = 451MB peak (139MB with cursor); 2 = 601MB; 5 = 3.2GB segfault; 10 = 3.6GB segfault. Per-binary blowup is ~18,000× (yaml input bytes → peak RSS). Root cause is the body builder inside `__StdYamlReader.list_item` and `.nested`: `buf = buf + line[4..]` in a loop allocates O(N²) bytes across N continuation lines, and Aperio's arena retains the intermediate buf values until the enclosing fn returns. The deeply-nested call-tree shape of grease's outward_tower section means a single top-level node's body contains its entire subtree as continuation lines, so N is in the thousands per item.
**Workaround:** None at v0. Renders 1-2 grease binaries fine; full 36 OOM-segfaults under the 4GB ulimit. The Sink fix in the renderer was correct but only addressed the *upper* string-concat anti-pattern — the same shape remained inside Reader. Ferryman ships with the Sink + cursor wins (3× memory, 3× speed on 1-binary subset) as a partial step; partner-codebase scale waits.
**Why it matters:** The Reader cannot return a substring efficiently because Aperio v0 has no O(1) string-view primitive — `s[a..b]` allocates and copies, and immutable Strings make any builder pattern collapse to the same O(N²) shape (a list-of-chunks "StringBuilder" stored as a tagged-accumulator hits the same `buf = buf + sep + chunk` quadratic). Two paths forward at the language level: (a) add a rope / chunk-list / lazy-concat primitive to the C runtime so `s1 + s2` is O(1); (b) add a string-view type with explicit (text, start, end) so Reader can return slices without copying. Either unblocks the ferryman partner-codebase target. Until then, the codebase-onboarder ceiling is "small-to-medium codebases" — not the multi-binary Cobra monorepos the partner-demo arc was aimed at.


## 2026-05-10 small-ergonomics-roundup [FIXED 2026-05-10 ergonomics arc]

**Source:** apps/ssg + apps/log-router + apps/tcp-echo FRICTION logs (cross-referenced; not editing the per-app logs from the compiler session per the territory rule).
**Tried:** Four small primitives apps were waiting on:
1. `std::io::fs::mkdir(path)` so an SSG can self-bootstrap its output directory (apps/ssg `no-mkdir` entry).
2. `std::io::fs::write_file_append(path, content)` so a log sink can append per-event without buffer-everything-at-dissolve (apps/log-router `write-file-truncates-no-append` entry).
3. `eprintln(args...)` / `eprint(args...)` builtins so debug output doesn't contaminate stdout (apps/log-router `no-eprintln-cant-isolate-debug-output` entry; also gates std::log::StdoutSink WARN/ERROR routing per the std::log doc page).
4. `String + Int` (and Float / Bool / Decimal / Duration / Time) auto-coerces via `to_string` so `"port=" + port` works (apps/tcp-echo `to_string-int-via-concatenation` entry). Symmetric — `port + " is the port"` also works.
**Hit:** N/A — feature requests, not friction.
**Resolution:** All four shipped end-to-end. C runtime: `lotus_fs_mkdir(path)` (single-level, mode 0755), `lotus_fs_write_file_append(path, buf, len)` (`O_WRONLY | O_CREAT | O_APPEND`, no truncate). Codegen: `lower_std_io_fs_mkdir`, `lower_std_io_fs_write_file_append`, `eprintln`/`eprint` routed through `dprintf(2, ...)` (avoids the cross-libc `stderr` FILE* macro shape), and `String + <printable>` BinOp::Add gets a value_to_string coercion before lower_binop. All three fs calls return `Int` (0/-1 sentinels per existing convention). The String+Int auto-coerce mirrors the existing println/eprintln-style mixed-type compose; chained forms (`"a=" + a + " b=" + b`) work because each `+` resolves left-associatively into a String. Apps that were waiting on these can now call them at the bare-name surface without further changes; per-app FRICTION entries can be marked resolved by the next app-dev session that touches them.

## 2026-05-10 single-file-app-monolith [FIXED 2026-05-10 ergonomics arc]

**Source:** ferryman render OOM fix (immediately after the Sink entry above)
**Tried:** Split `apps/ferryman/main.ap` (2,295 lines + ~290 uncommitted) into `skeleton.ap`, `render.ap`, `topology.ap`, `main.ap` — same `apps/ferryman/` dir, shared namespace, like Go's per-directory package.
**Hit:** Aperio has no per-directory package model. Each `.ap` file is its own translation unit; user code can't reference identifiers across files. The build is `aperio build apps/ferryman/main.ap`, one file in.
**Workaround:** Keep everything in `main.ap`. Threading Sink through ~30 functions in a single file is harder to review than threading it through four small files in a directory would be.
**Why it matters:** The stdlib itself already cheats around this — `STDLIB_AP_SOURCE = concat!(include_str!(...))` of 12 files in `crates/aperio-codegen/runtime/stdlib/` proves codegen can swallow multiple files as one seed; the constraint is a surface gap, not an implementation one. Surfacing it to user code (e.g., `aperio build apps/ferryman/` treats the dir as one seed) would unlock app-level file decomposition today and remove the perverse incentive to grow monolith `.ap` files. Compounds with the Sink friction above: missing interfaces + missing per-dir packages together push every "should be 4 small files with 3 implementations" toward one big file with one tagged locus.
**Resolution:** `aperio build <dir>` now treats every `.ap` file in the directory as one seed. Top-level decls (loci, types, free fns, perspectives, consts) declared in any file are visible to every other file in the same directory, in one shared scope. Single-file `aperio build foo.ap` keeps working. Binary defaults to the directory's basename (`apps/ferryman/` → `apps/ferryman/ferryman`). File order in the merged bundle is alphabetical (deterministic; resolution is order-free because the typechecker flattens all top-level decls into one bundle scope before name resolution). Same shape Go gets from per-package visibility. Ferryman + any future multi-concern app can decompose freely. See `examples/multi-file-seed/` and `crates/aperio-codegen/tests/multi_file_build.rs`.

## 2026-05-10 closure-keyword-shadows-helper-ident [FIXED 2026-05-11]

**Source:** lotus-harness library sketches (examples/51-geom-segment)
**Tried:** Write a small Float-tolerance helper `fn approx(actual: Float, expected: Float, eps: Float, label: String)` to fill the `assert_eq_float` gap in std::test (m87 ships int / str eq only).
**Hit:** `parse error: expected function name, got Approx`. `approx` is reserved at the lexer level as the long-form spelling of the `~~` closure-assertion operator (per spec/tokens.md "Closure keywords"), so it can't appear in identifier position even where context is unambiguous.
**Workaround:** Renamed to `near_eq`. Cheap one-time rename; the friction is the reservation surface, not this fix.
**Why it matters:** Reserving every closure-vocabulary word at the lexer level (rather than only in closure-block context) shadows a natural pool of math-shaped identifier names (`approx`, `within` as a free fn, etc.). Either narrow the reservation to contextual (closure-block-only) the way mode keywords are admitted post-`.` per F.10, or have the lexer admit them as Idents and let the parser do the structural check.
**Resolution (2026-05-11):** F.10-style contextual narrowing. `approx` and `within` no longer lex as keywords — they're Idents at the lexer level, recognized contextually by `parse_closure_assertion` inside closure bodies. `closure`, `epoch`, `persists_through`, and `resets_on` stay reserved (unambiguous block-introducers / clause-leaders). Regression coverage: `ok_approx_within_as_idents_outside_closure` and `ok_approx_keyword_inside_closure_still_works` in `crates/aperio-types/src/lib.rs`.

## 2026-05-10 if-needs-block-value

**Source:** lotus-harness library sketches (examples/54-geom-leading-edge)
**Tried:** Inside the windowed-regression `fit()` method, compute a physical storage index conditionally: `let phys = if self.n < 8 { i } else { (self.head + i) % 8 };`. Rust-shaped block-value if.
**Hit:** `parse error: expected ;, got RBrace`. The bare `i` in the then-block is parsed as a statement and trips immediately — Aperio's blocks are statement-sequences with no implicit "last expression is the value." `if_expr = if_stmt` per the grammar, but `if_stmt`'s blocks contain `statement*`, not `statement* expression?`.
**Workaround:** Rewrote as `let mut phys = i; if self.n >= 8 { phys = (self.head + i) % 8; }`. Verbose but works.
**Why it matters:** Hits every place a small conditional value would be cleanest — index selection, default-fallback, ternary-ish expressions. Adding a Rust-shaped trailing-expression rule (the block's last token, if it's an expression and not followed by `;`, becomes the block's value) is a localized parser change. Match arms already have an expression branch (`match_arm = pattern [ "if" expression ] "->" ( expression | block )`), so the asymmetry with `if` is jarring.

## 2026-05-10 float-surface-gaps [PARTIAL FIX 2026-05-11 — Int→Float coerce + std::math shipped; [val; N] still pending]

**Source:** lotus-harness library sketches (examples/51..58)
**Tried:** Write numeric Segment / Decay / Ring / Correlator with the usual helpers: an Int running count and a Float accumulator, Pearson `r` (not `r²`), exponential time-decay, sqrt-based stddev.
**Hit:** Three gaps that compound:
  1. No `Int → Float` coercion or `int_to_float(...)` builtin. `let nf: Float = self.n;` where `n: Int` is rejected; no obvious workaround beyond carrying a parallel `nf: Float` field updated alongside.
  2. No `sqrt` / `exp` / `pow` in stdlib. `std::math` is sketched aspirationally in `spec/stdlib.md`'s v0 module map but not shipped.
  3. No Float array-literal repetition syntax (`[0.0; 8]`). Array defaults must enumerate every element.
**Workaround:** (1) parallel Int+Float counters in every accumulator locus; (2) report `r²` instead of `r`, use plain EMA instead of time-weighted decay, skip variance/stddev; (3) write `[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]` literally in every fixed-cap ring.
**Why it matters:** Numeric primitives are the substrate for triangulator-class apps (leading-edge geometry, multi-feed correlation, momentum). Each workaround is small; together they make every Float-heavy library noisier than it should be. `std::math::{sqrt,exp,pow,log}` would unlock real time-weighted decay (`exp(-dt/tau)`), Pearson `r` (sqrt of `r²`), proper stddev. Int→Float coercion at `let nf: Float = self.n;` would drop ~10% of every accumulator locus's surface. Array-default repetition (`[0.0; N]`) is cosmetic but compounds with the no-generic-`Ring<N>` situation.
**Resolution (2026-05-11, sub-bullets 1 + 2):** Codegen now widens Int → Float via `sitofp` at let-binding type-ascription sites (`let nf: Float = int_expr;`) and at fn-arg sites where the param is `Float` and the arg is `Int`. The widening is one-way only — `Float → Int` narrowing stays explicit; `Decimal` and other lossy mixes still reject. `std::math::{sqrt, exp, log, floor, ceil}` (unary) and `std::math::pow` (binary) ship as path-call dispatches routing to libm; `declare_builtins` adds the extern decls, `lower_std_math_unary` / `lower_std_math_binary` carry the arg coercion + libm-call sequence; arg widening Int→Float happens inside those helpers. End-to-end coverage in `crates/aperio-codegen/tests/math_and_int_float.rs`. Sub-bullet 3 (`[val; N]` array repetition) is Phase 2d in the post-F.20-Phase-B planning arc; the literal-enumeration workaround stays until that ships.

## 2026-05-11 nested-locus-child-field-reads-return-garbage

**Source:** apps/reload-demo — the reload-lotus pattern
**Tried:** Have `MarketStateL.birth()` instantiate a long-lived `ReloadL { };` (statement-position; has `bus subscribe "model.curve"`) so each market auto-spawns its reload-lotus child. Parent reads installed-model state via `for child in self.children { if child.has_model { return child.slope * t + child.intercept; } }`. The styleguide / F.6 says lifecycle-method bodies attach children to the enclosing locus, and the long-lived rule says statement-position loci with bus subscriptions become anonymous children of the enclosing scope. Should work.
**Hit:** Under `aperio build` the for-loop iterates one child (correct), but reads `child.has_model` as `true` even when the field was never set (default is `false` and `ReloadL.on_model` never fires); `child.slope` and `child.intercept` read as `6.95e-310` / `4.17e-309` (uninitialized memory). Adding explicit `ReloadL { has_model: false, slope: 0.0, intercept: 0.0 }` at instantiation doesn't change anything — the field reads through the child handle bypass the actual state. Independently, `ReloadL.on_model` itself never fires (verified by a `println` inside it that never reaches stdout); the bus subscription registered at the nested child apparently doesn't receive published cells.
**Workaround:** Restructure so `ReloadL` instances bind as top-level siblings of their corresponding `MarketStateL` in `main()` instead of as nested children. Bus delivery then works; field reads on let-bound top-level loci work. The market's extrapolate becomes `extrapolate_via(reload: ReloadL, t: Float)` taking the reload by argument rather than discovering it through `self.children`.
**Why it matters:** The user's vision for the reload-lotus pattern is that it sits *under* the market substrate ("we're building like, a 'reload lotus' that can sit under it and reparameraize"). That spatial relationship — reload as a sub-locus of market — is the vertical-only-flow expression of "the substrate stays still; the kernel installed under it can be swapped." With nested-locus bus subscribe + nested-child field reads broken at v0 codegen, the architecture is forced sideways into a top-level-siblings layout that loses the "under" relationship. Two distinct issues collapse here: (1) nested long-lived locus birthed in lifecycle methods doesn't seem to register its bus subscription correctly, and (2) field-reads through `for child in self.children` handles return memory that doesn't match the child's actual state. Either alone would break the pattern; both together cleanly map to "spawn at top level" as the only available workaround.

## 2026-05-11 self-stack-empty-on-method-call-from-free-fn

**Source:** apps/reload-demo (and examples/56-io-frame-line, examples/55-geom-triangulate retrospectively)
**Tried:** Run `apps/reload-demo` under `aperio run` (the interpreter path). The fn `main()` body does `let lf = LineFrame { }; lf.feed("...");` — standard external method call from a free fn on a let-bound locus handle.
**Hit:** `runtime error: 'self' referenced outside a locus body`. The error fires on the FIRST method body that references `self` after the call from main(). For `MarketStateL.count()` (just `return self.n;`) the error is immediate. `aperio build` of the same source works.
**Workaround:** Skip `aperio run` entirely; use `aperio build` + run-binary. All eight sketches in `examples/51..58` from the prior lotus-harness commit pass under build but fail under run for the same reason.
**Why it matters:** The interpreter and codegen paths now diverge meaningfully — code that compiles and runs natively can't be debugged with the lighter-weight `aperio run`. The script-like inner loop loses value as the codebase grows. The fix is presumably in `crates/aperio-runtime/src/eval.rs` (the self_stack push at line 822-826 fires the error; the call-site that should be pushing the receiver isn't reaching this path). Codegen handles it correctly via the locus-ABI `self_ptr` parameter at every method.

## 2026-05-11 k_max-codegen-not-wired

**Source:** apps/reload-demo header `println` of each locus's `self.k_max`
**Tried:** Build a locus with `B`, `c`, `sigma`, `phi` params and read `self.k_max` per F.16 ("`self.k_max` is a built-in computed field"). The interpreter implements this — `crates/aperio-runtime/src/eval.rs:1175-1210` returns `B / [(1-phi)c + phi*sigma]` on every `self.k_max` field access.
**Hit:** `aperio build` errors: `codegen error: unsupported in codegen v0: no field 'k_max' on locus 'MarketStateL'`. The typechecker injects `k_max: Float` on every locus type (per F.16) and the interpreter computes it, but the codegen field-lookup path doesn't have the synthetic-field branch.
**Workaround:** Remove `self.k_max` from the demo, display `B / c / sigma / phi` separately. A `fn k_max_of(B: Int, c: Int, phi: Float, sigma: Int) -> Float` free fn would replace it cleanly, but Aperio v0 has no `Int → Float` coercion (logged in float-surface-gaps above), so computing `B / [(1-phi)c + phi*sigma]` from Int params requires parallel Float fields on every locus.
**Why it matters:** F.16 is the framework signature equation — the load-bearing identity that makes capacity a first-class language concept. Apps that lean on the cascade (the harness arc per `docs/src/std/roadmap.md`) want `self.k_max` available at runtime in the native binary, not just in the interpreter. Until codegen catches up, capacity-cascade demos lose their headline display value and have to compute the formula manually.

## 2026-05-11 stale-locus-method-result-on-second-call

**Source:** apps/reload-demo — `FitterAppL.fit_and_publish_from(market)`
**Tried:** Write the natural shape — `let s = market.fit(); println("count=", to_string(s.count())); if s.count() >= 2 { ... }`. `market.fit()` returns a `SegmentL` populated from the market's tick ring; calling `s.count()` twice in sequence should both return the same Int (the segment is a let-bound locus alive for the rest of the fn).
**Hit:** First `s.count()` (inside the println) returns the correct value (5 in my test). Second `s.count()` (inside `if`) returns 0 or otherwise fails the `>= 2` check — the `if` body is silently skipped, no publish fires, and the demo's bus delivery never happens. The first hint that something was wrong was tens of minutes of debugging output where `[debug] fitter: s.count()=5` printed but `[debug] fitter: publishing model.curve` did not.
**Workaround:** Bind once at the top of the function and reuse: `let n = s.count(); let sl = s.slope(); let ic = s.intercept(); if n >= 2 { ... use sl, ic, n ... }`. Once cached, all reads stay consistent.
**Why it matters:** Returning a locus from a method (`-> SegmentL`) and calling further methods on the returned handle is a load-bearing pattern — `examples/54-geom-leading-edge`'s `fit()` does exactly this, and it works there (single-locus context). What breaks here is *cross-locus* return-then-method-chain: `MarketStateL.fit()` returns to `FitterAppL.fit_and_publish_from`. The cross-locus boundary may be deep-copying or aliasing the segment in a way that the second method call sees a dissolved / re-initialized state. Compounds with the m49 free-fn-return rules (TypeRef-struct returns from free fns are gated; locus returns from methods aren't documented to be either way). A clear ABI commitment + test for "let s = otherLocus.method() returning locus; s.method() and s.field both stay consistent within the calling fn" would settle this.

## 2026-05-11 bus-payload-primitives-only

**Source:** apps/market-book — gateway/book typed messages
**Tried:** Use the natural shape for the bus payloads — `type SnapshotLevelMsg { source: String; seq: Int; side: Int; price: Fixed; qty: Fixed; }` where `Fixed` is a user-declared single-field struct (`type Fixed { raw: Int; }`). The gateway publishes `m: SnapshotLevelMsg` and the book reads `m.price.raw`, `m.qty.raw` at handler entry. Lets the price/qty travel as the abstract money-math type rather than leaking the storage representation onto the wire.
**Hit:** `codegen error: unsupported in codegen v0: bus payload field 'price: TypeRef("Fixed")' — m70 wire format supports primitives and String only; nested structs / enums / arrays / tuples cross-process are post-v1 polish`. The gating is at codegen, not typechecking — `aperio check` passes; the error fires at `aperio build`.
**Workaround:** Flatten Fixed.raw into Int fields on the message: `price_raw: Int; qty_raw: Int`. Publisher writes `price_raw: price.raw`, subscriber reconstitutes via `let price = fixed_from_raw(m.price_raw)` at handler entry.
**Why it matters:** Bus payloads are the inter-lotus contract surface; in a cross-process deployment they're the only contract surface. Forcing them to primitives breaks the lotus principle that types are the shape vocabulary — every value-shaped abstraction (Fixed, Decimal, Duration-via-struct, structured IDs, n-tuple coordinates) decomposes back to primitives at the publish site and reconstitutes at the subscribe site. Workaround is mechanical but each one pushes more wire-shape mechanics into both the publisher and the subscriber. v0 wire format is presumably "flatten one level of primitive fields with a known C ABI"; widening to "flatten arbitrarily-nested user types whose leaves are primitives" is a single-arc widening (recursive struct walker emitting the same memcpy pattern + a leaf-only check). Compounds with the lack of Decimal as a payload type — a real money lib wants Fixed-or-Decimal on the wire, not Int-with-out-of-band-scale-convention.

## 2026-05-11 self-array-field-index-assign-unsupported

**Source:** apps/market-book — BookL._set_bid / _set_ask / _remove_*
**Tried:** Mutate one slot of a fixed-cap array-typed locus field in place: `self.bid_prices[i] = price_raw;`. Standard pattern when maintaining a sorted ladder — find insertion point, shift right, drop in the new entry, increment count.
**Hit:** `codegen error: unsupported in codegen v0: assignment target 'self.bid_prices' with 2 segment(s) not yet supported`. Indexed assignment is supported on local let-bound arrays (`let mut next = ...; next[i] = x;` works fine — used heavily in reload-demo's feed_tick) but not when the indexed target's base is a locus field.
**Workaround:** Copy-out / mutate-locally / write-back-whole-array: `let mut next = self.bid_prices; next[i] = x; self.bid_prices = next;`. Functionally correct but pessimistic on memory — every single-slot update allocates and writes a full N-element array.
**Why it matters:** Locus state heavy on fixed-cap arrays is *the* canonical shape for windowed accumulators and ladder-style state machines — BookL has six such arrays, and every mutating handler touches one or two slots per call. The copy-out / write-back pattern works but masks the intent (the helper bodies in book.ap read like "atomic snapshot replacement" when they're actually "increment one slot"). One-line codegen extension: in `lower_assign_target`, route 2-segment `self.<field>[idx]` paths through the same GEP machinery used for the let-bound-array path (the addressing is identical; only the base pointer source differs — `self_ptr + field_offset` instead of a local alloca). Compounds with the lack of array-default repetition (`[0; 8]`) — every BookL ctor enumerates eight zeros twice per side; with `[0; 8]`-and-self-field-index-assign together, the helper bodies could collapse from ~20 lines each to ~5.
