// The scenarios `run.mjs` runs the generated TypeScript client through
// (GH #1417, R8a), written in TypeScript so that `tsc --strict` holds the
// client's types to how a consumer uses them: the tagged union narrows, the
// payloads are typed, a `switch` over it is exhaustive.
//
// Each scenario answers a label per step, which `run.mjs` compares to the
// contract's recordings (`tests/api-contract/wire/`) as the replay server
// serves them.

import * as c from "./client.js";

function refusalText(r: c.Refusal): string {
  return "refusal:" + r.kind + ":" + r.reason + ":" + r.requires.join(",") + ":" + (r.served ?? "");
}

function exhausted(x: never): never {
  throw new Error("a variant the test does not know: " + JSON.stringify(x));
}

function place(o: c.OrdersPlaceOutcome): string {
  switch (o.kind) {
    case "result":
      return "result:" + o.value.order + ":" + o.value.notional;
    case "refusal":
      return refusalText(o.refusal);
    case "server_error":
      return "server_error";
    case "handler_error":
      // `Orders::place` may violate, so its failure is the server error and it has
      // no handler error: the type of `o.error` is `never`
      return exhausted(o.error);
    default:
      return exhausted(o);
  }
}

function cancel(o: c.OrdersCancelOutcome): string {
  switch (o.kind) {
    case "result":
      return "result:" + o.value.order + ":" + o.value.was_open;
    case "handler_error":
      return "handler_error:" + o.error.code + ":" + o.error.reason;
    case "refusal":
      return refusalText(o.refusal);
    case "server_error":
      return "server_error";
    default:
      return exhausted(o);
  }
}

async function lost(call: () => Promise<unknown>): Promise<string> {
  try {
    await call();
    return "answered";
  } catch (e) {
    return e instanceof c.TransportError ? "lost" : "other:" + String(e);
  }
}

export async function http(endpoint: string): Promise<string[]> {
  const opts: c.ClientOptions = { endpoint, bearer: "t-alice" };
  const order = (qty: number): c.PlaceOrder => ({ symbol: "ACME", qty, limit: 12500 });
  const out: string[] = [];
  out.push(place(await c.ordersPlace(opts, order(10))));
  out.push(cancel(await c.ordersCancel(opts, { order: 999 })));
  out.push(place(await c.ordersPlace(opts, order(50000))));
  out.push(place(await c.ordersPlace(opts, order(10))));
  out.push(place(await c.ordersPlace(opts, order(10))));
  out.push(place(await c.ordersPlace(opts, order(10))));
  out.push(place(await c.ordersPlace(opts, order(10))));
  out.push(place(await c.ordersPlace({ endpoint, bearer: "t-mallory" }, order(10))));
  out.push(cancel(await c.ordersCancel({ endpoint, bearer: "t-bob" }, { order: 41 })));
  out.push(place(await c.ordersPlace(opts, order(10))));
  return out;
}

export async function httpLost(endpoint: string): Promise<string> {
  return lost(() => c.ordersPlace({ endpoint, bearer: "t-alice" }, { symbol: "ACME", qty: 10, limit: 12500 }));
}

/** The events of one subscription to its end, as labels; a lost connection ends the list with `lost`. */
export async function stream(endpoint: string): Promise<string[]> {
  const labels: string[] = [];
  const sub = c.subscribeFills({ endpoint, bearer: "t-dave" });
  try {
    for await (const e of sub) {
      switch (e.kind) {
        case "subscribed":
          labels.push("subscribed");
          break;
        case "event":
          labels.push("event:" + e.seq + ":" + e.payload.order + ":" + e.payload.qty + ":" + e.payload.price);
          break;
        case "refusal":
          labels.push(refusalText(e.refusal));
          break;
        case "expired":
        case "revoked":
        case "closed":
          labels.push(e.kind);
          break;
        default:
          exhausted(e);
      }
    }
  } catch (e) {
    labels.push(e instanceof c.TransportError ? "lost" : "other:" + String(e));
  }
  return labels;
}

export function digest(): string {
  return c.SURFACE_DIGEST;
}
