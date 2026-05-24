# Bindings handoff — pond/pty for iris

This file is the brief for whoever lands `pond/pty` as a real
`@ffi`-backed pond library. Paste it (or share the file) at
the top of a fresh session; it's written to be self-contained.

The interface contract — every type / fn / locus the
implementation must expose — lives in
[`COMPILER_FFI.md`](./COMPILER_FFI.md). This file is the
*how-to-pick-it-up* brief; the contract is the *what-to-build*.

> **Status (2026-05-24).** raylib already shipped per this
> playbook — `iris/lib/raylib/` has real `@ffi("c")` bindings
> + glue.c + auto-pickup via `hale.toml [ffi]`. The same
> pattern is what `pond/pty` needs. `pond/heron` (tree-sitter
> grammar) also shipped using this shape — see
> `hale-lang/pond/heron/` as a worked second example.
>
> **Original predecessor:** earlier scoping targeted
> `std::raylib` / `std::pty` in hale's stdlib. lotus-
> lang pivoted to a user-extensible `@ffi("c") fn …;`
> annotation + `hale.toml [ffi]` link declarations instead
> — a one-time compiler change that unblocked all future C
> bindings. This handoff reflects that pivot.

---

# Task: land pond/pty using `@ffi`

You're authoring a new pond lib in
`/home/riley/code/hale-lang/pond/`. The lib uses hale's
`@ffi("c")` extern-fn annotation to declare C symbols directly
in `.hl` source; the build picks up link flags from
`hale.toml [ffi]`.

Worked examples already in pond/iris using this pattern:
- `iris/lib/raylib/` — raylib bindings (Phase 5.a shipped)
- `pond/heron/` — tree-sitter grammar wrapper

Copy whichever is closer in shape (`pond/heron/glue.c` is
small + clean; `iris/lib/raylib/glue.c` is larger with more
types).

## Prerequisites

- **hale `@ffi` Stage 1 must be shipped.** Stage 1 covers
  parser + typecheck + LLVM `declare` for `@ffi`-annotated fns,
  plus manual build flags. See hale's `notes/ffi-design.md`
  (commit `65e3c06`). If Stage 1 isn't up yet, this handoff is
  premature — stand by.
- **`libraylib.so`** installed on the build host
  (`apt install libraylib-dev` or equivalent). Pinned version
  TBD — agree with the iris team if a specific version is
  load-bearing.
- **`libutil`** for `forkpty` (on Linux it's separate from
  `libc`; on macOS / BSD it's bundled).

## Read these first

1. **`/home/riley/code/hale-lang/iris/COMPILER_FFI.md`** —
   the binding contract. Every type / fn / locus declared there
   under `pond/raylib::*` and `pond/pty::*` must be exposed by
   your implementation. The doc is the source of truth; edit it
   first if a binding choice doesn't make sense, then track in
   your pond impl.

2. **`/home/riley/code/hale-lang/iris/lib/raylib/` and
   `/home/riley/code/hale-lang/iris/lib/pty/`** — the working
   stub seeds. They define the user-facing surface (types,
   `Window` / `Pty` loci, free fns) that iris consumes today.
   Your pond impls should expose the same surface; the only
   difference is the bodies — stubs are no-ops, your pond
   versions call into `@ffi` externs.

3. **hale's `notes/ffi-design.md`** — `@ffi` syntax,
   semantics, hale.toml schema.

## Deliverables

Two pond libs. Each is a directory of `.hl` files (one F.19
seed) plus optional C glue.

### `pond/raylib/`

```
pond/raylib/
├── README.md           # usage + version pin + link reqs
├── hale.toml         # [ffi] block declaring libraylib link
├── types.hl            # Vec2, Vec3, Color, Rect, Camera3D
├── keys.hl             # KEY_* + MOUSE_BUTTON_* + CAMERA_* consts
├── ffi.hl              # @ffi("c") extern declarations
├── window.hl           # Window locus wrapping init/close
├── frame.hl            # begin/end/clear/scissor
├── draw2d.hl           # text/rect/line/circle
├── draw3d.hl           # mode_3d/cube/sphere/line_3d/grid/plane/update_camera
├── input.hl            # is_key_*/get_char_pressed/mouse_*
└── glue/               # optional — only if upstream names need wrapping
    └── raylib_glue.c
```

The `ffi.hl` file is where `@ffi("c") fn raylib_init_window(...)`
etc. live. Wrapper fns (the user-facing `init_window(w, h, title)`)
either call the extern directly or live in their own `.hl` files
(window.hl / frame.hl / etc.).

### `pond/pty/`

```
pond/pty/
├── README.md
├── hale.toml         # links libutil for forkpty
├── pty.hl              # types + Pty locus + free fns
├── ffi.hl              # @ffi("c") extern declarations
└── glue/
    └── pty_glue.c      # forkpty wrapper + signal handling
```

PTY needs a small C glue layer because `forkpty`'s parameter
shape (output `int*` for the master fd, output `pid_t`, optional
struct args) doesn't map cleanly to a single Hale extern. The
glue exposes a flatter ABI:

```c
// pond/pty/glue/pty_glue.c
int lotus_pond_pty_forkpty(const char *cmd, const char *args,
                           int cols, int rows,
                           int *out_master_fd, int *out_pid);
```

Then the `.hl` side:

```hale
@ffi("c")
fn lotus_pond_pty_forkpty(cmd: String, args: String,
                          cols: Int, rows: Int,
                          out_fd: ?, out_pid: ?) -> Int;
```

(Out-pointer ABI for the master_fd + pid pair depends on what
`@ffi` Stage 1 admits — could be Int returns, could be a small
struct, etc. Pin shape against the actual `@ffi` syntax once
Stage 1 is up.)

## The pattern (per fn)

For each new fn `pond/raylib::draw_cube(pos, size, c)`:

1. **`@ffi` extern decl** in `pond/raylib/ffi.hl`:
   ```hale
   @ffi("c")
   fn raylib_draw_cube(pos: Vec3, size: Vec3, c: Color);
   ```
   Symbol defaults to the fn name — `raylib_draw_cube` resolves
   to the C symbol `raylib_draw_cube`. If you need to wrap
   raylib's actual `DrawCube`, either (a) write a one-line glue
   `void raylib_draw_cube(...) { DrawCube(...); }`, or (b) use
   `@ffi("c", symbol = "DrawCube")` if/when Stage 1 supports
   explicit renames.

2. **User-facing wrapper** in `draw3d.hl` (or call the extern
   directly — wrapper is for ergonomics / type coercion):
   ```hale
   fn draw_cube(pos: Vec3, size: Vec3, c: Color) {
       raylib_draw_cube(pos, size, c);
   }
   ```

3. **Link declaration** in `pond/raylib/hale.toml`:
   ```toml
   [ffi]
   link = ["raylib"]
   sources = ["glue/raylib_glue.c"]   # only if glue.c exists
   ```

4. **Test** in `pond/raylib/examples/` (or upstream pond's
   convention) covering window open → draw → close.

## Verification

When both libs ship:

1. Build pond/raylib + pond/pty clean.
2. Vendor into iris (`iris/hale.toml` adds dep entries; run
   `hale fetch`). Sources land at `iris/vendor/pond/raylib/`
   etc.
3. Two paths for iris to consume:
   - **Direct.** Swap `import "lib/raylib" as ray;` → `import
     "vendor/pond/raylib" as ray;`. Delete `iris/lib/raylib/`.
   - **Shim.** Keep `iris/lib/raylib/` as a thin re-export
     layer that forwards to `vendor/pond/raylib`. Iris source
     unchanged; only the stub `.hl` files inside `lib/raylib/`
     get rewritten as forwarding wrappers.
4. Run `iris/tests/stub_smoke/main.hl` — the surface should
   match, so it typechecks against the real bindings.
5. Run `./iris/iris` — a real window opens.

## Friction iris has already surfaced

[`FRICTION.md`](./FRICTION.md) has three items. Two are
relevant to you (quality-of-life around how the bindings get
authored):

- **F.1 `const-rejects-struct-literal`** — v1 codegen requires
  `const X: T = …;` to be a primitive literal. Theme constants
  like `const BG: Color = Color { r: 26, … };` don't parse;
  must be a fn. If hale lifts this, your pond/raylib can
  expose meaningful color constants (`pond/raylib::BLACK`,
  `WHITE`, etc.).
- **F.2 `mode-keyword-blocks-natural-param-name`** — `mode` is
  reserved; raylib's `UpdateCamera(camera, mode)` had to bind
  as `update_camera(cam, cam_mode)`. If hale promotes
  `mode` to a contextual keyword (same pattern as `pool` /
  `heap` / `with`), your bindings can use raylib's natural
  param names.

Bring these up with hale if you hit them during the
binding work — both are small compiler-side asks.

## Coordination rules

- **Don't modify iris.** Communication through
  `iris/COMPILER_FFI.md` only.
- **The contract document is mutable** — if a binding choice
  doesn't work out of the box, edit COMPILER_FFI.md (in iris)
  first, commit, then track in your pond impl. iris reads the
  doc; the doc IS the agreement.
- **Sessions concurrently working on iris exist.** Stay in
  pond. If you find an iris-side issue, leave a note in
  `iris/FRICTION.md` (with a tagged entry) or open an issue —
  don't fix it.

## Order to work in

1. Confirm `@ffi` Stage 1 is shipped + working (cargo build
   of hale succeeds; `@ffi` parses + typechecks).
2. Read COMPILER_FFI.md end-to-end.
3. Read iris/lib/raylib/ + iris/lib/pty/ stubs to absorb the
   surface.
4. Plan: what subset to ship in the first iteration. Suggested
   minimal first slice — Window locus + init_window +
   close_window + should_close + begin_frame + end_frame +
   clear_background. That alone unblocks iris's Phase 0 (empty
   window opens).
5. Iterate outward: drawing fns → input → 3D.
6. `pond/pty` is independent — can be parallel-tracked or
   sequential. Smaller surface (one locus, ~4 free fns).
