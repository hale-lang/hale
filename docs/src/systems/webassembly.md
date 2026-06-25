# WebAssembly & the browser

> **Coming from the web stack?** Hale compiles to a self-contained
> `.wasm` plus a small `.mjs` loader — no Emscripten, no bundler.
> The same locus/bus/`std::*` program you run natively can run in
> the browser; you choose the target at build time. The browser
> APIs you can't reimplement (fetch, WebSocket, WebGL, the DOM)
> come in as thin host functions, and Hale functions you want the
> page to call go out as exports.

## Building for wasm

```sh
hale build client/main.hl --target wasm32
```

This emits `client/main.wasm` (self-contained — a tiny bundled
libc, no external runtime) and `client/main.mjs` (a loader that
instantiates the module and wires the host functions). The program
declares the target so the typechecker can gate the parts of the
standard library that need syscalls:

```hale
target wasm { }
```

Under `target wasm`, the portable stdlib works as usual
(`std::str`, `std::bytes`, `std::json`, `std::math`, …), but the
POSIX-backed namespaces (`std::io::tcp`, `std::process`,
`std::http`, …) are rejected at typecheck — the browser sandbox has
no syscalls. Reach the outside world through host functions instead.

The **in-process typed bus** — `topic` / `bus { publish … }` /
`bus { subscribe … }` across loci — runs under wasm exactly as it
does natively: a `Subject <- payload` is delivered to every matching
subscriber's handler in the same module, payload-copied through the
synthesized wire codec. Only the *cross-process / network* transports
(`shm_ring`, `unix`, CONNECT-role bindings) are unavailable in the
sandbox — those need syscalls. So the idiomatic locus + topic + bus
shape is fully available client-side.

## Calling the host: `@ffi("js")`

`@ffi("js")` is the wasm sibling of [`@ffi("c")`](./binding-c.md):
it declares a function the JavaScript loader provides.

```hale
target wasm { }
@ffi("js") fn console_log(msg: String);
@ffi("js") fn draw_line(x1: Float, y1: Float, z1: Float,
                        x2: Float, y2: Float, z2: Float);
```

Marshalling: `Float` and `Int` both arrive as a plain JS `number` —
an `@ffi("js")` `Int` crosses as f64, *not* a `BigInt`, so your host
handler gets a number with no `Number(x)` step, and an `Int`-returning
import takes a plain number back. (The one caveat is f64's range:
`Int`s beyond 2^53 lose precision across this boundary — send those as
a `String`/`Bytes` payload. And this applies to `@ffi("js")` only;
`@ffi("c")` keeps i64.) `String`/`Bytes` arrive as a pointer the loader
reads out of wasm memory. The loader ships a built-in `console_log` and
the libm set (so `std::math` just works); your page supplies the rest
through `run(glue)`:

```js
import { run } from "./main.mjs";
const inst = await run((h) => ({
  draw_line: (x1,y1,z1,x2,y2,z2) => { /* push to a WebGL buffer */ },
}));
```

## Letting the host call you: `@export` + the app locus

To run a game loop or react to network messages, the *host* needs
to call *into* Hale. The browser-client shape is an **`@export
locus`** — the persistent "app" of your program:

```hale
@export locus Client {
    params { sx: Float = 0.0; sy: Float = 0.0; ready: Bool = false; }
    birth() { }
    fn on_message() { /* parse an inbound frame, update fields */ }
    fn frame()      { /* render from the fields */ }
}
```

Each `fn` method becomes a wasm export the page calls by name
(`inst.exports.frame()`). State lives in the locus's fields and
**persists across calls** — `on_message()` writes `self.sx`,
`frame()` reads it, just like a native locus. On the native target
`@export` is a no-op. (There is also a lower-level `@export fn` for
free functions — same export, but stateless; see below.) Methods
may not be `fallible` (the host has no error channel), and the locus
must not define `run()` — the host drives it.

## The run-model: entry inversion

A native program blocks in `main`. A browser program can't — it
must return to the event loop so the page stays responsive. So a
program built with `@export` runs **inverted**: there is no `main`,
and the host drives the exports (typically `frame()` once per
`requestAnimationFrame`).

The compiler synthesizes an exported **`_hale_start()`** that sets
up a *persistent* program arena and **instantiates your `@export
locus`** (running `birth`). The loader calls it once at startup;
after that the page drives the methods:

```js
const inst = await run(glue);     // _hale_start ran here (Client is alive)
function tick() {
  inst.exports.frame();
  requestAnimationFrame(tick);
}
requestAnimationFrame(tick);
```

A program made of `@export` declarations needs no `fn main` at all.

## Inbound messages

The page hands network bytes to Hale through the **inbox**: write
them into wasm memory, publish the length, then call a method.

```js
// JS: hand a WebSocket frame to Hale, then notify it
const bytes = new TextEncoder().encode(ev.data);
const ptr = inst.exports.lotus_wasm_alloc(bytes.length);
new Uint8Array(inst.exports.memory.buffer).set(bytes, ptr);
inst.exports.lotus_wasm_set_inbox(bytes.length);
inst.exports.on_message();
```

```hale
@ffi("c") fn lotus_wasm_inbox() -> Bytes;   // the bytes JS wrote

// inside the Client locus:
fn on_message() {
    let msg = lotus_wasm_inbox();
    if len(msg) > 0 {
        let s = std::str::from_bytes(msg);
        // ... std::json parse, then store into self.* ...
        self.ready = true;
    }
}
```

This is the full pattern for a browser client: the page owns the
transport (fetch / WebSocket) and the GL context; the `@export
locus` parses the protocol with `std::json`, holds the game state
in its fields, runs the camera, and emits geometry — the same code
shape it would have natively.

## Lower-level: `@export fn` + the state cell

If you don't want a locus, you can export free functions
(`@export fn frame()`). These are stateless — each call's
allocations are released on return — so cross-call state must be
parked in the runtime **state cell**, packed into `Bytes`:

```hale
@ffi("c") fn lotus_wasm_state_set(b: Bytes);
@ffi("c") fn lotus_wasm_state_get() -> Bytes;
```

The `@export locus` model is preferred for anything with state; the
state cell exists for the free-fn path and for hand-rolled layouts.

See [`spec/ffi.md` § WASM host interface](../../../spec/ffi.md) for the
exact marshalling and diagnostic rules.
