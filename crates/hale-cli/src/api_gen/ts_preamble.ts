// ---- the wire, as every client of a surface speaks it (spec/api.md § The clients) ----
//
// Depends on `fetch` and `WebSocket` and nothing else. An endpoint is
// `http://host:port` for a member's call and `ws://host:port` (a hub) for a
// stream; a socket is not spoken (`fetch` has none). A call sends the
// surface's digest in `Hale-Surface-Digest` and answers one of four outcomes;
// the fifth, a connection that never answered, is thrown as a TransportError,
// because a request that may have been accepted is not a refusal.

export interface ClientOptions {
  /** `http://host:port` of an `http::Rpc` listener (or of a hub's listener). */
  endpoint: string;
  /** The bearer token, sent as `Authorization: Bearer`. */
  bearer?: string;
  /** A `fetch` to use instead of the global one. */
  fetch?: typeof fetch;
}

export interface StreamOptions {
  /** `ws://host:port` of a hub. */
  endpoint: string;
  /** The bearer token, sent as `?access_token=` (a browser cannot set a header on a WebSocket). */
  bearer?: string;
  /** A `WebSocket` constructor to use instead of the global one. */
  WebSocket?: typeof WebSocket;
}

/** A refusal: the request was not run (spec/api.md § Outcomes). */
export interface Refusal {
  /** `malformed`, `digest_mismatch`, `unauthenticated`, `unauthorized`, `full`, `shutting_down` or `unavailable`. */
  kind: string;
  reason: string;
  /** The roles a refused row requires. */
  requires: string[];
  /** The digest the server holds, for `digest_mismatch`. */
  served?: string;
}

/** The connection ended, or answered no outcome: the request may or may not have run. */
export class TransportError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TransportError";
  }
}

export type Outcome<T, E> =
  | { kind: "result"; value: T }
  | { kind: "handler_error"; error: E }
  | { kind: "refusal"; refusal: Refusal }
  | { kind: "server_error" };

function apiRefusal(body: unknown): Refusal {
  const r = (body as { refusal?: Record<string, unknown> } | null)?.refusal ?? {};
  const out: Refusal = {
    kind: typeof r.kind === "string" ? r.kind : "malformed",
    reason: typeof r.reason === "string" ? r.reason : "",
    requires: Array.isArray(r.requires) ? r.requires.map(String) : [],
  };
  if (typeof r.served === "string") out.served = r.served;
  return out;
}

async function apiCall<T, E>(
  opts: ClientOptions,
  member: string,
  payload: string,
  decodeValue: (json: unknown) => T,
  decodeError: (json: unknown) => E,
): Promise<Outcome<T, E>> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    "Hale-Surface-Digest": SURFACE_DIGEST,
  };
  if (opts.bearer) headers["Authorization"] = "Bearer " + opts.bearer;
  let res: Response;
  let text: string;
  try {
    res = await (opts.fetch ?? fetch)(opts.endpoint + "/call/" + member, { method: "POST", headers, body: payload });
    text = await res.text();
  } catch (e) {
    throw new TransportError("the endpoint " + opts.endpoint + " cannot be reached: " + String(e));
  }
  let body: unknown = null;
  try {
    body = text.length > 0 ? JSON.parse(text) : null;
  } catch {
    throw new TransportError("the endpoint answered " + res.status + " with no JSON: " + text);
  }
  if (res.status === 200) return { kind: "result", value: decodeValue(body) };
  if (res.status === 422) return { kind: "handler_error", error: decodeError(body) };
  const refusal = apiRefusal(body);
  if ((body as { refusal?: unknown } | null)?.refusal === undefined) {
    throw new TransportError("the endpoint answered " + res.status + " with no outcome: " + text);
  }
  if (refusal.kind === "server") return { kind: "server_error" };
  return { kind: "refusal", refusal };
}

/** The description the endpoint serves this caller, as parsed JSON. */
export async function describe(opts: ClientOptions): Promise<unknown> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (opts.bearer) headers["Authorization"] = "Bearer " + opts.bearer;
  try {
    const res = await (opts.fetch ?? fetch)(opts.endpoint + "/.description", { headers });
    return JSON.parse(await res.text());
  } catch (e) {
    throw new TransportError("the description of " + opts.endpoint + " cannot be read: " + String(e));
  }
}

// ---- streams over a hub ----

/** What a subscription yields; it ends after `refusal`, `expired`, `revoked` or `closed`. */
export type StreamEvent<P> =
  | { kind: "subscribed" }
  | { kind: "event"; seq: number; payload: P }
  | { kind: "refusal"; refusal: Refusal }
  | { kind: "expired" }
  | { kind: "revoked" }
  | { kind: "closed" };

export interface Subscription<P> extends AsyncIterable<StreamEvent<P>> {
  /** Ends the subscription and closes the connection. */
  close(): void;
}

function apiSubscribe<P>(opts: StreamOptions, topic: string, decode: (json: unknown) => P): Subscription<P> {
  const queue: Array<StreamEvent<P> | TransportError | null> = [];
  let wake: (() => void) | undefined;
  let ended = false;
  const push = (item: StreamEvent<P> | TransportError | null): void => {
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
  const finish = (): void => {
    if (ended) return;
    ended = true;
    push(null);
    try {
      ws.close();
    } catch {
      /* already closed */
    }
  };
  const lose = (why: string): void => {
    if (ended) return;
    ended = true;
    push(new TransportError(why));
    push(null);
  };
  ws.onopen = () => ws.send(JSON.stringify({ type: "subscribe", topic }));
  ws.onmessage = (m: MessageEvent) => {
    if (ended) return;
    let frame: Record<string, unknown>;
    try {
      frame = JSON.parse(String(m.data)) as Record<string, unknown>;
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
    async *[Symbol.asyncIterator](): AsyncGenerator<StreamEvent<P>> {
      for (;;) {
        while (queue.length === 0) await new Promise<void>((resolve) => (wake = resolve));
        const item = queue.shift();
        if (item === null || item === undefined) return;
        if (item instanceof TransportError) throw item;
        yield item;
      }
    },
  };
}
