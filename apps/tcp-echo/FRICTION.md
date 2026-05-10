# tcp-echo friction log

Append-only. Format per the app-dev brief.

## 2026-05-10 logger-not-shareable-with-fn-pointer-callback

**Tried:** Construct one `net` Logger in `main()` and reference it inside the named `handle_connection(s: Stream)` callback passed to `Listener.on_connection`.
**Hit:** `on_connection` is a `fn(Stream)` function pointer, not a closure. There is no way to capture the `net` binding from `main()`'s scope, and Aperio has no top-level `let` / no globals — so the callback cannot reach a Logger constructed in `main()`.
**Workaround:** Re-instantiate `let net = std::log::Logger { name: "net" };` and the child `echo` Logger inside `handle_connection` on every connection. Cheap because Logger is stateless beyond a computed full_path, but it is conceptually a per-connection cost.
**Why it matters:** Any callback-driven stdlib API (Listener, future timer/signal, etc.) that needs application context has to either (a) re-construct it on every callback invocation, or (b) push the context onto the bus and have the callback subscribe. Closures-as-values (or top-level `let` for app-singletons) is the obvious lift. Until then, every Listener-based program will repeat this dance.

## 2026-05-10 recv-returns-string-not-bytes

**Tried:** Echo arbitrary bytes back with full byte-fidelity using `Stream.recv` → `Stream.send_bytes`.
**Hit:** `recv(max) -> String` returns a `String`, but `send_bytes` takes `Bytes`. There is no `recv_bytes`, and no `String -> Bytes` conversion in scope. Stuck using `send(String)`, which is documented as truncating on embedded NULs.
**Workaround:** Use `s.send(buf)` and accept the NUL-truncation caveat for ASCII/UTF-8 traffic. Documented in README.
**Why it matters:** A truthful TCP echo cannot be written today. Any binary protocol (length-prefixed frames, BSON, gRPC-ish) would silently corrupt at the first NUL byte. A `Stream.recv_bytes(max) -> Bytes` would close the loop and is the symmetric counterpart of the m89 `send_bytes`.

## 2026-05-10 to_string-int-via-concatenation

**Tried:** Build a log message with `"port=" + port` (Int).
**Hit:** Compiler error — String + Int is not a valid concatenation. Had to wrap with the builtin `to_string(port)`.
**Workaround:** Used `to_string(port)` explicitly in every place an Int went into a log message. This works fine.
**Why it matters:** Minor papercut. `println` already concatenates mixed-type args natively; the asymmetry between `println("p=", n)` (works) and `let s = "p=" + n;` (rejected) is mild but it bites when constructing a single-string `msg` to hand to `Logger.info`.
