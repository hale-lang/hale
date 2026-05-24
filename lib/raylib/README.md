# lib/raylib — real @ffi-backed bindings (Phase 5)

Bindings to upstream [raylib](https://www.raylib.com/) for iris's
window + 3D + input substrate. Phase 5 graduated this from
no-op stubs to real `@ffi("c")` declarations + a C glue layer.

## Surface

Consumer code is unchanged from the prior stub era —
`import "lib/raylib" as ray;` + `ray::init_window(w, h, title)`
etc. all work the same. What changed is that the calls now
hit libraylib instead of returning zeros.

## System requirements

iris builds need **libraylib** installed on the host. This is a
one-time setup step; iris's `hale.toml [ffi]` block picks the
library up automatically once it's available.

### Linux

The Debian / Ubuntu / Pop!_OS apt repositories don't carry
raylib directly. Two paths:

**Option A — build from source** (recommended; matches upstream):

```bash
# Install raylib's dev-time dependencies
sudo apt install -y \
    build-essential git cmake \
    libgl1-mesa-dev libx11-dev libxrandr-dev libxinerama-dev \
    libxcursor-dev libxi-dev libwayland-dev libxkbcommon-dev

# Clone + build
git clone --depth 1 https://github.com/raysan5/raylib.git /tmp/raylib
cd /tmp/raylib/src
make PLATFORM=PLATFORM_DESKTOP -j$(nproc)

# Install. Raylib's Makefile uses RAYLIB_INSTALL_PATH as the *lib*
# directory (not a prefix), so set both explicitly to the standard
# /usr/local/{lib,include} layout — otherwise libraylib.a lands at
# /usr/local/libraylib.a and the linker can't find it via -lraylib.
sudo make install \
    RAYLIB_INSTALL_PATH=/usr/local/lib \
    RAYLIB_H_INSTALL_PATH=/usr/local/include
sudo ldconfig
```

If you already installed raylib with `RAYLIB_INSTALL_PATH=/usr/local`
(the misleading variant), the static lib will be at the wrong path
and the linker will fail with `cannot find -lraylib`. Fix:

```bash
sudo mv /usr/local/libraylib.a /usr/local/lib/libraylib.a
sudo ldconfig
```

**Option B — PPA** (Ubuntu/derivatives only, may lag upstream):

```bash
sudo add-apt-repository ppa:texus/raylib
sudo apt update
sudo apt install libraylib-dev
```

### Verification

After install:

```bash
echo '#include <raylib.h>' | gcc -E -x c - >/dev/null && echo OK
pkg-config --modversion raylib 2>/dev/null   # if pkg-config found it
ldconfig -p | grep raylib
```

If all three succeed, `hale build .` from `iris/` will link
cleanly.

### macOS / Windows

Not yet attempted in this build-out. raylib supports both
platforms; the iris-side glue.c is standard C99 and should
port. The build path would use `brew install raylib` (macOS)
or vcpkg (Windows).

## Layout

```
lib/raylib/
├── README.md       ← this file
├── hale.toml     ← [ffi] link surface (auto-picked up by iris's build)
├── glue.c          ← C-side adapter; converts Hale struct layouts to raylib's
├── raylib.hl       ← @ffi("c") decls + the user-facing wrapper surface
├── types.hl        ← Vec3 / Vec2 / Color / Rect / Camera3D
└── keys.hl         ← KEY_* + MOUSE_BUTTON_* + CAMERA_* constants
```

### Why a C glue layer

Hale user-struct fields are `i64` (Int) / `double` (Float) at
the C-ABI boundary. raylib's structs use tight `u8` / `float`
fields. The glue.c converts field-by-field at each call
(`(uint8_t)c->r, (uint8_t)c->g, ...`). No way to match the
layouts directly without either widening raylib (impractical)
or narrowing Hale's numeric types per call site
(architectural mess).

The glue is small — one conversion helper per struct type
(Vec3, Color, Rect, Camera3D), then one trivial wrapper per
upstream raylib function.

## Migration notes (Phase 5)

Previously this seed shipped stubs that returned zeros and
no-op'd every call. The Hale-side **surface** (function names,
signatures, return types) is unchanged — only the bodies were
swapped to call `@ffi("c") fn raylib_*(...)` extern decls
declared at the top of `raylib.hl`.

Consumer code (iris's `Renderer`, `VizPane`, etc.) didn't need
edits.

## Limitations

- **`fn measure_text` in the stub returned a rough estimate
  (text length × size / 2). The real binding returns raylib's
  actual font metrics**, which can differ. Code that relied on
  the stub's specific formula will see different numbers.
- **No Font locus yet.** Custom TTF font loading is COMPILER_FFI
  surface but currently absent from `raylib.hl`. Raylib's default
  font is used everywhere; landing custom fonts is straightforward
  once a workload demands.
- **`update_camera` returns a fresh Camera3D via sret.** The
  prior stub passed through unchanged; the real binding applies
  raylib's WASD / orbit logic (per the `mode` int).
