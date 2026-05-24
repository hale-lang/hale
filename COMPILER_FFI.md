# FFI bindings — surface contract

> **Status:** source-of-truth Hale surface contract for the
> raylib + PTY bindings iris consumes. The contract is unchanged
> from before — every type / fn / locus declared here must be
> exposed by the underlying implementation. What changed is the
> *implementation home*: bindings now land in `pond/raylib` and
> `pond/pty` via the user-extensible `@ffi` mechanism, not in
> hale's stdlib.
>
> Edits here are binding — when the surface needs to change, this
> doc changes first, then the stubs in `iris/lib/`, then the
> downstream pond impls track both.

## Implementation model — `@ffi` mechanism (in flight)

Hale's FFI is becoming user-extensible. The compiler is landing
a single one-time `@ffi("c") fn …;` annotation plus an
`hale.toml [ffi]` section declaring link libraries and C glue
sources. Once that ships, any seed can declare extern C symbols
and the build picks up the link flags. See hale's
`notes/ffi-design.md` (commit `65e3c06`) for the three-stage
rollout (parser/typecheck/declare → toml integration → pond
conventions).

Implications for this contract:

- **Surface unchanged.** The types, fns, and loci spelled out
  below are still the binding contract. They land under
  `pond/raylib::*` and `pond/pty::*` instead of
  `std::raylib::*` / `std::pty::*`.
- **No compiler-team backlog per binding.** Adding raylib + pty
  is now ordinary pond lib work, runnable as soon as Stage 1 of
  the `@ffi` mechanism ships.
- **iris source unchanged.** Production code today uses
  `import "lib/raylib" as ray;` against the stub seed. When pond
  bindings ship, the swap is either (a) delete the stub and
  `import "vendor/pond/raylib" as ray;` (vendored), or (b) keep
  the `lib/raylib` shim and re-export pond's surface through it.
  Either path leaves the rest of the source identical.
- **`std::http::recv_chunk`** remains a stdlib extension (HTTP
  is already in stdlib; this is one new fn). Queued for the
  same hale session that finishes Stage 1.

## What lives where

| Binding | Implementation home | Status |
|---|---|---|
| raylib | `iris/lib/raylib/` (real `@ffi`-backed, Phase 5.a shipped) | live |
| `pond/heron` | tree-sitter Hale grammar + `@ffi` wrapper for libtree-sitter | live; vendored at `iris/lib/heron/` |
| `pond/pty` | new pond lib, `@ffi`-backed | not yet authored; iris still on `iris/lib/pty/` stub |
| `pond/sse` (or `iris/lib/sse`) | pure Hale over `std::http::*` | blocked on `std::http::recv_chunk` |
| `std::http::recv_chunk` | stdlib extension (NOT user-extensible) | small hale ask |

`pond/sse` could live in iris/lib/ instead — it's not raylib-
or-pty-specific, and pulling it into pond is only worth it if
another lib wants SSE too.

raylib's `@ffi` bindings now ship in `iris/lib/raylib/` —
real `@ffi("c") fn raylib_*` declarations + glue.c.
`@ffi` Stage 1 + 2 (auto-pickup from `hale.toml [ffi]`)
landed in hale and made this the standard pattern for
binding any C library.

## Migration path

1. **Today.** Iris imports `lib/raylib`, `lib/pty`, `lib/sse`.
   Stubs are no-ops (or stub-state-driven for testing).
2. **hale Stage 1 lands.** `@ffi` is usable; iris and pond
   contributors can author bindings.
3. **`pond/raylib`, `pond/pty` ship.** Vendored via
   `hale.toml` + `hale fetch` (per pond convention).
4. **Switch.** Each `iris/lib/<name>/` shim becomes either
   (a) deleted, with iris code switched to `pond/<name>::*`
   paths; or (b) kept as a thin re-export layer that delegates
   to `pond/<name>` (handy if we want to keep test-fixture
   helpers reachable).

The surface below is identical in stub-land and after the
switch, so iris's production code doesn't change between
states.

---

## `pond/raylib::*` — window, drawing, input

Thin binding. Free fns + value types + a `Window` locus where
lifecycle matters. Names mirror raylib's C API as snake_case so
upstream reference material translates directly.

### Value types

```hale
type Vec3   { x: Float = 0.0; y: Float = 0.0; z: Float = 0.0; }
type Vec2   { x: Float = 0.0; y: Float = 0.0; }
type Color  { r: Int = 0; g: Int = 0; b: Int = 0; a: Int = 255; }
type Rect   { x: Float = 0.0; y: Float = 0.0;
              width: Float = 0.0; height: Float = 0.0; }
type Camera3D {
    position:   Vec3   = Vec3 { x: 0.0, y: 5.0, z: 10.0 };
    target:     Vec3   = Vec3 { };
    up:         Vec3   = Vec3 { x: 0.0, y: 1.0, z: 0.0 };
    fovy:       Float  = 60.0;
    projection: Int    = 0;   // 0 = perspective, 1 = orthographic
}
```

### Window lifecycle (one window per process)

```hale
fn init_window(width: Int, height: Int, title: String) -> ();
fn close_window() -> ();
fn should_close() -> Bool;
fn set_target_fps(fps: Int) -> ();
fn get_screen_width() -> Int;
fn get_screen_height() -> Int;
fn set_window_title(title: String) -> ();
```

Plus a locus wrapper that hides raylib's implicit global state:

```hale
locus Window {
    params {
        width:  Int    = 1280;
        height: Int    = 720;
        title:  String = "iris";
        target_fps: Int = 60;
    }
    birth()    { init_window(self.width, self.height, self.title);
                 set_target_fps(self.target_fps); }
    dissolve() { close_window(); }
}
```

### Frame

```hale
fn begin_frame()              -> ();   // BeginDrawing
fn end_frame()                -> ();   // EndDrawing
fn clear_background(c: Color) -> ();
fn begin_scissor(r: Rect)     -> ();   // clip subsequent draws
fn end_scissor()              -> ();
```

### 2D drawing

```hale
fn draw_text(text: String, x: Int, y: Int, size: Int, c: Color) -> ();
fn measure_text(text: String, size: Int) -> Int;
fn draw_rect(r: Rect, c: Color)         -> ();
fn draw_rect_lines(r: Rect, c: Color)   -> ();
fn draw_line(x1: Int, y1: Int, x2: Int, y2: Int, c: Color) -> ();
fn draw_circle(x: Int, y: Int, radius: Float, c: Color)    -> ();
```

### 3D drawing

```hale
fn begin_mode_3d(cam: Camera3D) -> ();
fn end_mode_3d() -> ();
fn draw_cube(pos: Vec3, size: Vec3, c: Color)               -> ();
fn draw_sphere(center: Vec3, radius: Float, c: Color)       -> ();
fn draw_line_3d(start: Vec3, finish: Vec3, c: Color)        -> ();
fn draw_grid(slices: Int, spacing: Float)                   -> ();
fn draw_plane(center: Vec3, size: Vec3, c: Color)           -> ();
fn update_camera(cam: Camera3D, mode: Int) -> Camera3D;
// mode: 0=custom, 1=free, 2=orbital, 3=first-person, 4=third-person
```

### Input (polled per-frame; raylib refreshes during `end_frame`)

```hale
fn is_key_down(key: Int)     -> Bool;   // held this frame
fn is_key_pressed(key: Int)  -> Bool;   // newly down this frame
fn is_key_released(key: Int) -> Bool;
fn get_char_pressed()        -> Int;    // next unicode char in queue
fn get_key_pressed()         -> Int;    // next raw keycode in queue

fn is_mouse_button_down(b: Int)    -> Bool;
fn is_mouse_button_pressed(b: Int) -> Bool;
fn get_mouse_position()            -> Vec2;
fn get_mouse_delta()               -> Vec2;
fn get_mouse_wheel_move()          -> Float;
```

### Constants

Raylib's `KEY_*` and `MOUSE_BUTTON_*` exposed as `const Int`. See
`iris/lib/raylib/keys.hl` for the v0 subset; the full set lands in
`pond/raylib::keys` once the bindings ship.

### Font loading (deferred — raylib default font suffices for v0)

```hale
locus Font {
    params {
        path:   String;        // required
        size:   Float = 14.0;
        handle: Int   = -1;
    }
    birth()    { self.handle = __load_font(self.path, self.size) or fail; }
    dissolve() { if self.handle >= 0 { __unload_font(self.handle); } }
    fn draw(text: String, x: Int, y: Int, c: Color) -> ();
    fn measure(text: String) -> Vec2;
}
```

---

## `pond/pty::*` — PTY-backed subprocess

```hale
type PtyOptions {
    cols: Int    = 80;
    rows: Int    = 24;
    cwd:  String = "";    // empty = inherit parent
}

type PtyError {
    kind:   String = "";   // "fork_failed", "exec_failed",
                           // "io_failed", "closed"
    errno:  Int = 0;
    detail: String = "";
}

locus Pty {
    params {
        cmd:        String;                       // required
        args:       String = "";                  // newline-sep, like std::process
        opts:       PtyOptions = PtyOptions { };
        master_fd:  Int = -1;
        pid:        Int = -1;
        last_error: PtyError = PtyError { };
    }

    closure fatal_io { captures: last_error, pid; epoch inline; }

    birth() {
        let h = __forkpty(self.cmd, self.args, self.opts)
                or self.bridge_io(err);
        self.master_fd = h.fd;
        self.pid = h.pid;
    }

    dissolve() {
        if self.master_fd >= 0 { __close_fd(self.master_fd); }
        if self.pid > 0        { __reap(self.pid); }
    }
}

// Free fn surface — value channel (locus methods can't be fallible).
fn read_nonblocking(p: Pty, buf: std::bytes::BytesBuilder, max: Int)
        -> Int fallible(PtyError);
fn write(p: Pty, input: Bytes) -> Int fallible(PtyError);
fn resize(p: Pty, cols: Int, rows: Int) -> () fallible(PtyError);
fn is_alive(p: Pty) -> Bool;
```

Pattern matches `std::process::Child`: locus owns the OS handle +
child lifecycle; free fns carry the fallible surface.
`read_nonblocking` returns 0 when no data is available so callers
can yield between polls.

---

## `pond/sse::*` (or `iris/lib/sse`) — Server-Sent Events parser

Pure Hale on top of `std::http::*` streaming-recv. **No compiler
changes needed** if `std::http` already exposes streaming receive.
Verify before treating SSE as a separate effort — if streaming-recv
is missing, that's a small `std::http` extension, not new FFI.

```hale
type SseEvent {
    event: String = "";   // e.g. "content_block_delta"
    data:  String = "";   // raw payload (usually JSON)
    id:    String = "";   // optional event id
}

locus SseStream {
    params {
        http_response: std::http::Response;
        buffer:        std::bytes::BytesBuilder
                     = std::bytes::BytesBuilder { initial_cap: 8192 };
        closed:        Bool = false;
    }
}

// Pulls the next event, yielding cooperatively while waiting for
// more bytes. Fails with kind="closed" at stream end.
fn next(s: SseStream) -> SseEvent fallible(IoError);
```

---

## Iris consumption examples

Made concrete so the surface earns its keep.

### Renderer (uses raylib)

```hale
import "lib/raylib" as ray;

locus Renderer {
    params {
        window: ray::Window = ray::Window { };
        layout: Layout;
    }
    run() {
        loop {
            if ray::should_close() {
                "ide.cmd.quit" <- QuitRequest { };
                break;
            }
            ray::begin_frame();
            ray::clear_background(BG);
            for pane in self.layout.visible() {
                pane.render(RenderSurface { });
            }
            ray::end_frame();
            yield;
        }
    }
}

const BG: ray::Color = ray::Color { r: 26, g: 26, b: 26 };
```

### ShellPane (uses pty)

```hale
import "lib/pty" as ptylib;

locus ShellPane {
    params {
        proc:    ptylib::Pty
               = ptylib::Pty { cmd: shell_path() };
        scroll:  std::bytes::BytesBuilder
               = std::bytes::BytesBuilder { initial_cap: 65536 };
        focused: Bool = false;
    }
    run() {
        loop {
            let _ = ptylib::read_nonblocking(self.proc, self.scroll, 4096)
                      or self.bridge_io(err);
            yield;
        }
    }
    fn on_key(k: KeyEvent) {
        if !self.focused { return; }
        ptylib::write(self.proc, keystroke_to_bytes(k))
            or self.bridge_io(err);
    }
}
```

### AgentHarness (uses sse)

```hale
import "lib/sse" as sse;

fn on_user_input(p: UserPrompt) {
    let resp   = std::http::post_stream(...) or self.bridge_io(err);
    let stream = sse::SseStream { http_response: resp };
    loop {
        let evt = sse::next(stream) or break;
        if evt.event == "content_block_delta" {
            "ide.event.agent.chunk" <- Chunk { text: extract_delta(evt.data) };
        }
        yield;
    }
}
```

---

## Stub seeds (current state)

Live under `iris/lib/`. Each stub:

- Mirrors the surface declared above (same names, signatures, locus
  shapes).
- Implements no-op behavior for production calls (draw/window/input
  fns return zeros, draw_* are silent).
- Exposes a small **stub-helper surface** for tests to inject inputs
  and observe outputs. Stub helpers live in a `stub_helpers.hl`
  sibling in each lib seed so they're easy to spot and delete when
  real bindings ship.

| Lib | Stub helpers (test-only) |
|---|---|
| `lib/raylib` | none — Iris doesn't unit-test raylib calls; integration tests run against the real impl. |
| `lib/pty` | `stub_inject_output(p, bytes)`, `stub_drain_input(p) -> Bytes`, `stub_set_alive(p, alive)` |
| `lib/sse` | `stub_enqueue_event(s, event, data)`, `stub_close(s)` |

`iris/tests/stub_smoke/main.hl` is a single smoke-test program that
exercises all three surfaces, lives in its own seed (so it has its
own `fn main()`), and runs via `hale build && ./stub_smoke`.

## Where the work lives

- **`@ffi` mechanism (Stage 1)** — one-time compiler change in
  hale. Scoped + sequenced in hale's
  `notes/ffi-design.md`. iris is not on the critical path; the
  contract here is forward-compatible.
- **`pond/raylib`, `pond/pty`** — author once `@ffi` ships.
  iris team or pond contributors can land them; no further
  compiler involvement needed. Surface fixed by this document.
- **`std::http::recv_chunk`** — small hale ask, queued
  alongside Stage 1.
