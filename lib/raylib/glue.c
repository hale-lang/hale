// iris/lib/raylib/glue.c
//
// C-side adapter between Hale's @ffi("c") declarations in
// raylib.hl and upstream raylib's actual API.
//
// Hale's user structs are passed by pointer (per spec/ffi.md
// "User-type structs") with i64-shaped numeric fields. Raylib
// uses tight u8 / float-shaped structs. This file does the
// field-by-field conversion at each call.
//
// Build picks this up automatically because hale.toml declares
// it under [ffi] csrc.

#include <stdint.h>
#include <raylib.h>

// ---- Hale-side struct layouts (must match raylib.hl's
//      `type` declarations field-by-field).
//
// Field types follow spec/ffi.md marshalling:
//   Int   → int64_t
//   Float → double
//   Bool  → int32_t

typedef struct {
    double x;
    double y;
    double z;
} ApVec3;

typedef struct {
    double x;
    double y;
} ApVec2;

typedef struct {
    int64_t r;
    int64_t g;
    int64_t b;
    int64_t a;
} ApColor;

typedef struct {
    double x;
    double y;
    double width;
    double height;
} ApRect;

typedef struct {
    ApVec3  position;
    ApVec3  target;
    ApVec3  up;
    double  fovy;
    int64_t projection;
} ApCamera3D;

// ---- Conversion helpers (Hale → raylib).

static inline Vector3 to_rl_vec3(const ApVec3 *v) {
    return (Vector3){ (float)v->x, (float)v->y, (float)v->z };
}

static inline Color to_rl_color(const ApColor *c) {
    return (Color){
        (uint8_t)c->r, (uint8_t)c->g,
        (uint8_t)c->b, (uint8_t)c->a,
    };
}

static inline Rectangle to_rl_rect(const ApRect *r) {
    return (Rectangle){
        (float)r->x, (float)r->y,
        (float)r->width, (float)r->height,
    };
}

static inline Camera3D to_rl_camera(const ApCamera3D *c) {
    Camera3D out;
    out.position   = to_rl_vec3(&c->position);
    out.target     = to_rl_vec3(&c->target);
    out.up         = to_rl_vec3(&c->up);
    out.fovy       = (float)c->fovy;
    out.projection = (int)c->projection;
    return out;
}

// ---- Window lifecycle.

void raylib_init_window(int64_t w, int64_t h, const char *title) {
    InitWindow((int)w, (int)h, title);
}

void raylib_close_window(void) {
    CloseWindow();
}

int32_t raylib_should_close(void) {
    return WindowShouldClose() ? 1 : 0;
}

void raylib_set_target_fps(int64_t fps) {
    SetTargetFPS((int)fps);
}

int64_t raylib_get_screen_width(void) {
    return (int64_t)GetScreenWidth();
}

int64_t raylib_get_screen_height(void) {
    return (int64_t)GetScreenHeight();
}

void raylib_set_window_title(const char *title) {
    SetWindowTitle(title);
}

// ---- Frame.

void raylib_begin_frame(void) {
    BeginDrawing();
}

void raylib_end_frame(void) {
    EndDrawing();
}

void raylib_clear_background(const ApColor *c) {
    ClearBackground(to_rl_color(c));
}

void raylib_begin_scissor(const ApRect *r) {
    BeginScissorMode((int)r->x, (int)r->y, (int)r->width, (int)r->height);
}

void raylib_end_scissor(void) {
    EndScissorMode();
}

// ---- 2D drawing.

void raylib_draw_text(const char *text, int64_t x, int64_t y, int64_t size, const ApColor *c) {
    DrawText(text, (int)x, (int)y, (int)size, to_rl_color(c));
}

int64_t raylib_measure_text(const char *text, int64_t size) {
    return (int64_t)MeasureText(text, (int)size);
}

void raylib_draw_rect(const ApRect *r, const ApColor *c) {
    DrawRectangle((int)r->x, (int)r->y, (int)r->width, (int)r->height, to_rl_color(c));
}

void raylib_draw_rect_lines(const ApRect *r, const ApColor *c) {
    DrawRectangleLines((int)r->x, (int)r->y, (int)r->width, (int)r->height, to_rl_color(c));
}

void raylib_draw_line(int64_t x1, int64_t y1, int64_t x2, int64_t y2, const ApColor *c) {
    DrawLine((int)x1, (int)y1, (int)x2, (int)y2, to_rl_color(c));
}

void raylib_draw_circle(int64_t x, int64_t y, double radius, const ApColor *c) {
    DrawCircle((int)x, (int)y, (float)radius, to_rl_color(c));
}

// ---- 3D drawing.

void raylib_begin_mode_3d(const ApCamera3D *cam) {
    BeginMode3D(to_rl_camera(cam));
}

void raylib_end_mode_3d(void) {
    EndMode3D();
}

void raylib_draw_cube(const ApVec3 *pos, const ApVec3 *size, const ApColor *c) {
    Vector3 p = to_rl_vec3(pos);
    Vector3 s = to_rl_vec3(size);
    DrawCube(p, s.x, s.y, s.z, to_rl_color(c));
}

void raylib_draw_sphere(const ApVec3 *center, double radius, const ApColor *c) {
    DrawSphere(to_rl_vec3(center), (float)radius, to_rl_color(c));
}

void raylib_draw_line_3d(const ApVec3 *start, const ApVec3 *finish, const ApColor *c) {
    DrawLine3D(to_rl_vec3(start), to_rl_vec3(finish), to_rl_color(c));
}

void raylib_draw_grid(int64_t slices, double spacing) {
    DrawGrid((int)slices, (float)spacing);
}

void raylib_draw_plane(const ApVec3 *center, const ApVec3 *size, const ApColor *c) {
    Vector3 s = to_rl_vec3(size);
    // raylib's DrawPlane takes a Vector2 for size (x, z dims).
    Vector2 sz = (Vector2){ s.x, s.z };
    DrawPlane(to_rl_vec3(center), sz, to_rl_color(c));
}

// update_camera returns a Camera3D — sret pattern: out-pointer
// is the hidden first arg.
void raylib_update_camera(ApCamera3D *out, const ApCamera3D *cam, int64_t mode) {
    Camera3D tmp = to_rl_camera(cam);
    UpdateCamera(&tmp, (int)mode);
    // Convert back into Hale's layout.
    out->position.x   = (double)tmp.position.x;
    out->position.y   = (double)tmp.position.y;
    out->position.z   = (double)tmp.position.z;
    out->target.x     = (double)tmp.target.x;
    out->target.y     = (double)tmp.target.y;
    out->target.z     = (double)tmp.target.z;
    out->up.x         = (double)tmp.up.x;
    out->up.y         = (double)tmp.up.y;
    out->up.z         = (double)tmp.up.z;
    out->fovy         = (double)tmp.fovy;
    out->projection   = (int64_t)tmp.projection;
}

// ---- Input.

int32_t raylib_is_key_down(int64_t key) {
    return IsKeyDown((int)key) ? 1 : 0;
}

int32_t raylib_is_key_pressed(int64_t key) {
    return IsKeyPressed((int)key) ? 1 : 0;
}

int32_t raylib_is_key_released(int64_t key) {
    return IsKeyReleased((int)key) ? 1 : 0;
}

int64_t raylib_get_char_pressed(void) {
    return (int64_t)GetCharPressed();
}

int64_t raylib_get_key_pressed(void) {
    return (int64_t)GetKeyPressed();
}

int32_t raylib_is_mouse_button_down(int64_t b) {
    return IsMouseButtonDown((int)b) ? 1 : 0;
}

int32_t raylib_is_mouse_button_pressed(int64_t b) {
    return IsMouseButtonPressed((int)b) ? 1 : 0;
}

void raylib_get_mouse_position(ApVec2 *out) {
    Vector2 v = GetMousePosition();
    out->x = (double)v.x;
    out->y = (double)v.y;
}

void raylib_get_mouse_delta(ApVec2 *out) {
    Vector2 v = GetMouseDelta();
    out->x = (double)v.x;
    out->y = (double)v.y;
}

double raylib_get_mouse_wheel_move(void) {
    return (double)GetMouseWheelMove();
}
