# iris — friction log

Friction surfaced during iris build-out. Format follows the
pond convention: one entry per gap, with a tag, severity,
description, reproducer (where applicable), and workaround
or proposed resolution.

The bar for an entry: something hit during development that
made the work harder than the language design suggested it
should be. Workaround-of-the-day goes in the entry, not in
the affected source file as a hack — the entry is the
audit-log.

---

## Status snapshot (2026-05-24)

| ID  | Tag                                                         | Status   |
|-----|-------------------------------------------------------------|----------|
| F.1 | `const-rejects-struct-literal`                              | resolved |
| F.2 | `mode-keyword-blocks-natural-param-name`                    | resolved |
| F.3 | `hale-build-binary-name-inconsistency`                    | resolved |
| F.4 | `no-forward-refs-between-sibling-field-defaults`            | resolved |
| F.5 | `json-find-field-fails-on-repeat-calls-in-loop`             | resolved |
| F.6 | `json-escape-string-segfaults-in-bus-handler`               | resolved |
| F.7 | `clang-link-order-csrc-after-libs`                          | resolved |
| F.8 | `multi-string-alloc-in-mcp-dispatch-frame-segfaults`        | resolved |
| F.9 | `stdlib-missing-sin-cos-for-spatial-code`                   | resolved |
| F.10 | `codegen-unknown-cross-seed-type-in-signature`             | open     |

Open items below; resolved items further down for audit-log
continuity.

---

## Open

### F.10 — `codegen-unknown-cross-seed-type-in-signature`

**Tag:** `codegen-unknown-cross-seed-type-in-signature`
**Severity:** blocking iris's full build; typecheck passes.

**Status update (2026-05-23 PM).** `aee258d` (hale)
addressed the @form-cell-type case for qualified TypeExpr
renames. iris rebuilt against fresh hale CLI; the build
still fails with the same error for the **fn-signature
position** which `aee258d`'s `apply_qualified_path_renames`
pass apparently doesn't reach (or reaches but produces
mangled name that the codegen-side type registry doesn't
have).

**Description.**
`hale check .` passes cleanly on all 23 iris files + the
lib/lotus_viz seed + the lib/heron seed. `hale build .`
fails with:

```
codegen error: unsupported in codegen v0: unknown type name
  `__lib_raylib_types_Rect` in signature
```

The type is `ray::Rect` (defined in `lib/raylib/types.hl`),
used as a parameter type / return type / struct field type in
top-level iris files. Specifically:

- `messages.hl:36`: `bounds: ray::Rect;` (struct field)
- `layout.hl:23`: `fn rect_for(pane_name: String) -> ray::Rect`
- `layout.hl:51`: `fn status_rect() -> ray::Rect`
- (4 more `ray::Rect { ... }` literal uses in fn bodies)

**Curious asymmetry.** The same pattern works for
`ray::Color`:

- `renderer.hl:12`: `const BG: ray::Color = ray::Color { ... };`
- `pane.hl:28`: `fn focused_color() -> ray::Color { ... }`

These compile. The Rect cases above don't. Both Color and
Rect are declared in the same file (`lib/raylib/types.hl`),
both are simple struct types, both are reachable via the
same `ray::` alias. The codegen error is post-mangling
(`__lib_raylib_types_Rect` IS the mangled form) so the
mangler did its job for the use site; what's missing is
the matching registration at the type-decl site, or some
codegen pass that runs per-type that's skipping Rect.

**Reproducer.**
```
cd /home/riley/code/hale-lang/iris
hale check .   # ok: 23 file(s) typechecked
hale build .   # codegen error: unsupported in codegen v0:
                 #   unknown type name `__lib_raylib_types_Rect`
                 #   in signature
```

**Workaround on iris side.**
None — these uses are baked into the existing iris source
(messages.hl + layout.hl, pre-this-session). Removing them
would mean rewriting layout entirely. The right fix is
compiler-side.

**Proposed resolution.**
Extend `aee258d`'s `apply_qualified_path_renames` to cover
fn-signature parameter + return types AND struct/type field
types — or wire registration so that whatever entry-point
already handles Color also fires for Rect. The asymmetry
(Color works, Rect doesn't, in apparently parallel use)
is the clue to where the gap lies.

Quick triage suggestion: `grep` codegen for places where
Color resolves successfully vs Rect doesn't, walk the type-
registry mechanism to find where Rect would be expected to
land.

---

## Resolved

### F.4 — `no-forward-refs-between-sibling-field-defaults` (RESOLVED)

**Status:** fixed in hale `4a24b77` (2026-05-24).

The recursive instantiation pass was setting `params_init_self`
to THIS locus rather than the caller, so `self.X` inside a
sibling-field default couldn't see the caller's already-
initialized siblings. Fixed by threading the caller's
`current_self` through the params-init expression lowering.

iris cleanup: the workaround pattern (Dispatcher as
namespace lotus; gate passed per-call via
`handle(iris, p)`) was the v0 shape for Iris's gate +
dispatcher wiring. The proper-borrow shape
(`dispatcher: ProposalDispatcher = ProposalDispatcher
{ gate: self.gate }`) is now expressible if a future
restructure wants it; existing code keeps working as-is.

---

### F.9 — `stdlib-missing-sin-cos-for-spatial-code` (RESOLVED)

**Status:** fixed in hale `4a24b77` (2026-05-23).

`std::math` now ships `sin` / `cos` / `tan` / `asin` /
`acos` / `atan` / `atan2` — the full trig surface spatial
code needs.

Iris cleanup: `iris/lib/lotus_viz/layout.hl` replaced its
inlined 4-term Taylor approximation with thin wrappers over
`std::math::sin` / `std::math::cos`. The `sin_approx` /
`cos_approx` wrapper names stay so call sites in
animator.hl / scene.hl don't need to change. (Could be
inlined further when convenient.)

---

### F.8 — `multi-string-alloc-in-mcp-dispatch-frame-segfaults` (RESOLVED)

**Status:** fixed in hale (uncommitted at the time of this
sweep; verified against the freshly-rebuilt hale binary at
target/release/hale dated 2026-05-22 20:49).

Compiler team did the broader `emit_set_caller_arena()?;`
prologue sweep across String-returning stdlib lowerings:

* `lotus_str_builder_finish`
* `lotus_bytes_builder_finish`
* `lotus_file_read_line_global`
* `lotus_fs_list_dir_at`
* `lotus_udp_recv_bytes_global`
* `lotus_process_pipe_read_nonblocking`

Each now publishes the current arena into the caller-arena
TLS before the C call, so the C-side's
`lotus_bus_payload_arena_alloc` routes returned strings
through the calling frame's arena instead of inheriting
stale state from prior nested calls.

iris reverted: `DemoCatalog.step` removed the no-op
`demo.step.flush` proposal hack; pure-step demos (hello,
scene_info) now round-trip cleanly through MCP. Verified at
commit-after-`35cba13` with all 5 demos.

### F.7 — `clang-link-order-csrc-after-libs` (RESOLVED)

**Status:** fixed in hale (uncommitted at the time of this
sweep; verified against the freshly-rebuilt hale binary).

Compiler team swapped the two for-loops in
`build_executable_with_options`: csrc files now precede `-l`
flags in the clang invocation, matching GNU ld's left-to-right
symbol-resolution convention. The fix is exactly what the
friction entry proposed.

iris reverted: `lib/raylib/hale.toml` returned to the natural
shape — `link = ["raylib", "GL", "m", "pthread", "dl", "rt",
"X11"]` + `csrc = ["glue.c"]`. The hardcoded
`/usr/local/lib/libraylib.a` path is gone; the install layout
is portable across PPA / source-build / homebrew now.

### F.6 — `json-escape-string-segfaults-in-bus-handler` (RESOLVED)

**Status:** fixed in hale (same uncommitted sweep that
closed F.8). `std::json::escape_string` returns a String
through `lotus_str_builder_finish`'s allocation path, which is
one of the six lowerings that got the
`emit_set_caller_arena()?;` prologue.

iris reverted: `SessionStore.build_json` restored
`std::json::escape_string(self.current_source_path)`. Session
round-trip verified cleanly.

### F.1 — `const-rejects-struct-literal` (RESOLVED)

**Status:** fixed in hale `e75d2ef` (2026-05-22).

Top-level `const X: T = Color { r: 26, … };` now lowers
through `user_const_exprs` (parallel to `param_value` for
literals). Both intra-seed and cross-seed reads work.

Iris reverted at commit-after-`74c8cb2`: `bg_color()` fn in
`renderer.hl` replaced with `const BG: ray::Color = ray::Color { ... };`.

---

### F.2 — `mode-keyword-blocks-natural-param-name` (RESOLVED)

**Status:** fixed in hale `e75d2ef` (2026-05-22).

`mode` is now a contextual keyword (lexes as Ident; parser
recognizes it as the locus-member introducer at member
position only). Same pattern as `bindings` / `birth_check` /
`pool` / `heap`.

Iris reverted: `iris/lib/raylib/raylib.hl` —
`update_camera(cam, cam_mode)` restored to `update_camera(cam,
mode)`, matching raylib's natural signature.

---

### F.3 — `hale-build-binary-name-inconsistency` (RESOLVED)

**Status:** fixed in hale `e75d2ef` (2026-05-22).

`hale build .` now canonicalizes the path to recover the
real directory basename, so it produces `./iris` not `./main`
from inside `iris/`.

Iris cleanup: dropped the `/main` line from `.gitignore`. The
old stray `/main` binary never lands again.

---

### F.5 — `json-find-field-fails-on-repeat-calls-in-loop` (RESOLVED)

**Status:** fixed in hale `962a745` (2026-05-22).

Root cause was actually in `std::io::stdin::read_line`'s
lowering, not the JSON parser — the C-side `set_caller_arena`
prologue was missing, leaving stale TLS state that subsequent
allocations inherited and tripped on. F.6 (above) is the same
class of bug at the `std::json::escape_string` lowering;
likely fixes with the same one-line prologue.

iris's `iris --mcp` multi-request stdio loop works no-caveat
now; verified 7 sequential requests + an error case in one
process all round-trip cleanly.

---

## Useful findings (not friction — surprising in a good way)

### Bus delivery is synchronous

Bus publishes invoke subscribers immediately in the same call
stack. No yield required; the cooperative scheduler isn't in
the round-trip path. This means:

- `app.harness.submit_user_input(x)` → backend publishes
  `AgentChunk` → AgentPane subscriber appends to scrollback
  → control returns to caller — all in one synchronous chain.
- Render path after `handle_key()` sees the updated state.
- Logic-level integration tests work without a frame loop.

Discovered at commit `33d77f9` (Phase 2a.2) by inspecting
AgentPane's scrollback immediately after a submit and seeing
"echo> hello, iris" already there.

Implication for the buildout: a lot of PLAN.md's "needs a
frame loop" assumptions can be relaxed. The frame loop wraps
visuals; it isn't load-bearing for the bus / state model.

### Cross-shape conversions round-trip

`std::str::from_bytes(b.view())` and `std::bytes::from_string(s)`
compose cleanly. AgentPane uses both:

- `std::bytes::from_string(chunk.text)` to append a String chunk
  into a Bytes-backed scrollback.
- `std::str::from_bytes(self.scrollback.view())` to materialize
  the scrollback as a String for inspection / display.

No encoding gotchas observed at v0 with ASCII text. (UTF-8
multi-byte handling untested.)

### `@ffi` mechanism shipped (hale 2026-05-22)

The `@ffi("c") fn …;` annotation + `hale.toml [ffi]`
auto-pickup landed in hale Stage 1 (`a5f71c7`) +
Stages 2+3 (`018f926`). Iris's `pond/raylib` + `pond/pty`
binding work is no longer blocked; ordinary pond lib work
once the wire-up sessions begin.
