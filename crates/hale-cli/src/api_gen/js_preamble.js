// ---- the wire, as every client of a surface speaks it (spec/api.md § The clients) ----
//
// Depends on `fetch` and `WebSocket` and nothing else. An endpoint is
// `http://host:port` for a member's call and `ws://host:port` (a hub) for a
// stream; a socket is not spoken (`fetch` has none). A call sends the
// surface's digest in `Hale-Surface-Digest` and answers one of four outcomes;
// the fifth, a connection that never answered, is thrown as a TransportError,
// because a request that may have been accepted is not a refusal.
//
// The types are JSDoc, the TypeScript client's own: an editor or `tsc
// --checkJs` holds a caller to them, and nothing needs compiling to run it.
// The globals the wire names as types are spelled `globalThis.X`, so that no
// type of the surface (a `Response`, a `Record`, a `Promise`) can shadow one.

/**
 * @typedef {object} ClientOptions
 * @property {string} endpoint `http://host:port` of an `http::Rpc` listener (or of a hub's listener).
 * @property {string} [bearer] The bearer token, sent as `Authorization: Bearer`.
 * @property {typeof fetch} [fetch] A `fetch` to use instead of the global one.
 */

/**
 * @typedef {object} StreamOptions
 * @property {string} endpoint `ws://host:port` of a hub.
 * @property {string} [bearer] The bearer token, sent as `?access_token=` (a browser cannot set a header on a WebSocket).
 * @property {typeof WebSocket} [WebSocket] A `WebSocket` constructor to use instead of the global one.
 */

/**
 * A refusal: the request was not run (spec/api.md § Outcomes).
 * @typedef {object} Refusal
 * @property {string} kind `malformed`, `digest_mismatch`, `unauthenticated`, `unauthorized`, `full`, `shutting_down` or `unavailable`.
 * @property {string} reason
 * @property {string[]} requires The roles a refused row requires.
 * @property {string} [served] The digest the server holds, for `digest_mismatch`.
 */

/** The connection ended, or answered no outcome: the request may or may not have run. */
export class TransportError extends Error {
  /** @param {string} message */
  constructor(message) {
    super(message);
    this.name = "TransportError";
  }
}

/**
 * @template T, E
 * @typedef {{ kind: "result", value: T } | { kind: "handler_error", error: E } | { kind: "refusal", refusal: Refusal } | { kind: "server_error" }} Outcome
 */

/**
 * @param {unknown} body
 * @returns {Refusal}
 */
function apiRefusal(body) {
  const r = /** @type {{ refusal?: globalThis.Record<string, unknown> } | null} */ (body)?.refusal ?? {};
  /** @type {Refusal} */
  const out = {
    kind: typeof r.kind === "string" ? r.kind : "malformed",
    reason: typeof r.reason === "string" ? r.reason : "",
    requires: Array.isArray(r.requires) ? r.requires.map(String) : [],
  };
  if (typeof r.served === "string") out.served = r.served;
  return out;
}

/**
 * @template T, E
 * @param {ClientOptions} opts
 * @param {string} member
 * @param {string} payload
 * @param {(json: unknown) => T} decodeValue
 * @param {(json: unknown) => E} decodeError
 * @returns {globalThis.Promise<Outcome<T, E>>}
 */
async function apiCall(opts, member, payload, decodeValue, decodeError) {
  /** @type {globalThis.Record<string, string>} */
  const headers = {
    "Content-Type": "application/json",
    "Hale-Surface-Digest": SURFACE_DIGEST,
  };
  if (opts.bearer) headers["Authorization"] = "Bearer " + opts.bearer;
  /** @type {globalThis.Response} */
  let res;
  /** @type {string} */
  let text;
  try {
    res = await (opts.fetch ?? fetch)(opts.endpoint + "/call/" + member, { method: "POST", headers, body: payload });
    text = await res.text();
  } catch (e) {
    throw new TransportError("the endpoint " + opts.endpoint + " cannot be reached: " + String(e));
  }
  /** @type {unknown} */
  let body = null;
  try {
    body = text.length > 0 ? JSON.parse(text) : null;
  } catch {
    throw new TransportError("the endpoint answered " + res.status + " with no JSON: " + text);
  }
  if (res.status === 200) return { kind: "result", value: decodeValue(body) };
  if (res.status === 422) return { kind: "handler_error", error: decodeError(body) };
  const refusal = apiRefusal(body);
  if (/** @type {{ refusal?: unknown } | null} */ (body)?.refusal === undefined) {
    throw new TransportError("the endpoint answered " + res.status + " with no outcome: " + text);
  }
  if (refusal.kind === "server") return { kind: "server_error" };
  return { kind: "refusal", refusal };
}

/**
 * The description the endpoint serves this caller, as parsed JSON.
 * @param {ClientOptions} opts
 * @returns {globalThis.Promise<unknown>}
 */
export async function describe(opts) {
  /** @type {globalThis.Record<string, string>} */
  const headers = { Accept: "application/json" };
  if (opts.bearer) headers["Authorization"] = "Bearer " + opts.bearer;
  try {
    const res = await (opts.fetch ?? fetch)(opts.endpoint + "/.description", { headers });
    return JSON.parse(await res.text());
  } catch (e) {
    throw new TransportError("the description of " + opts.endpoint + " cannot be read: " + String(e));
  }
}

// ---- streams over a hub ----

/**
 * What a subscription yields; it ends after `refusal`, `expired`, `revoked` or `closed`.
 * @template P
 * @typedef {{ kind: "subscribed" } | { kind: "event", seq: number, payload: P } | { kind: "refusal", refusal: Refusal } | { kind: "expired" } | { kind: "revoked" } | { kind: "closed" }} StreamEvent
 */

/**
 * An async iterable of a stream's events; `close()` ends the subscription and closes the connection.
 * @template P
 * @typedef {globalThis.AsyncIterable<StreamEvent<P>> & { close(): void }} Subscription
 */

/**
 * @template P
 * @param {StreamOptions} opts
 * @param {string} topic
 * @param {(json: unknown) => P} decode
 * @returns {Subscription<P>}
 */
function apiSubscribe(opts, topic, decode) {
  /** @type {globalThis.Array<StreamEvent<P> | TransportError | null>} */
  const queue = [];
  /** @type {(() => void) | undefined} */
  let wake;
  let ended = false;
  /** @param {StreamEvent<P> | TransportError | null} item */
  const push = (item) => {
    queue.push(item);
    if (wake) {
      const w = wake;
      wake = undefined;
      w();
    }
  };
  const url = new URL(opts.endpoint);
  if (opts.bearer) url.searchParams.set("access_token", opts.bearer);
  const ws = new (opts.WebSocket ?? WebSocket)(url.toString());
  const finish = () => {
    if (ended) return;
    ended = true;
    push(null);
    try {
      ws.close();
    } catch {
      /* already closed */
    }
  };
  /** @param {string} why */
  const lose = (why) => {
    if (ended) return;
    ended = true;
    push(new TransportError(why));
    push(null);
  };
  ws.onopen = () => ws.send(JSON.stringify({ type: "subscribe", topic }));
  /** @param {globalThis.MessageEvent} m */
  ws.onmessage = (m) => {
    if (ended) return;
    /** @type {globalThis.Record<string, unknown>} */
    let frame;
    try {
      frame = /** @type {globalThis.Record<string, unknown>} */ (JSON.parse(String(m.data)));
    } catch {
      return lose("the hub sent a frame that is not JSON: " + String(m.data));
    }
    if (frame.type !== "closed" && frame.topic !== undefined && frame.topic !== topic) return;
    switch (frame.type) {
      case "subscribed":
        return push({ kind: "subscribed" });
      case "event":
        return push({ kind: "event", seq: Number(frame.seq), payload: decode(frame.payload) });
      case "refusal":
        push({ kind: "refusal", refusal: apiRefusal(frame) });
        return finish();
      case "unauthorized":
        push({ kind: frame.reason === "expired" ? "expired" : "revoked" });
        return finish();
      case "closed":
        push({ kind: "closed" });
        return finish();
      default:
        return;
    }
  };
  ws.onerror = () => lose("the hub at " + opts.endpoint + " cannot be reached");
  ws.onclose = () => lose("the hub ended the connection with no closed frame");
  return {
    close: finish,
    async *[Symbol.asyncIterator]() {
      for (;;) {
        while (queue.length === 0) await new Promise((resolve) => (wake = () => resolve(undefined)));
        const item = queue.shift();
        if (item === null || item === undefined) return;
        if (item instanceof TransportError) throw item;
        yield item;
      }
    },
  };
}
