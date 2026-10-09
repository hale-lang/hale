// Run by the `ui` compose service before it serves the page, and by hand:
//   node --test ui/seq.test.mjs
// (node's built-in runner; there is no build step and no dependency).
import { test } from "node:test";
import assert from "node:assert/strict";
import { needs_resync, on_event, load_done } from "./seq.js";

// The page's loop (ui/index.html) over a stream and a list reader: `reads`
// are the lists the reader returns, one per load; `events` the stream as
// `{ seq, kind: "add"|"remove", id }`, with "land" where the load under way
// lands (the stream is quiet for that moment).
function run_page(events, reads) {
  let s = { last: null, loading: true, gap_seen: false }; // subscribed: the first load begins
  let items = new Set();
  const early = [];
  let read = 0;
  const apply = (e) => (e.kind === "add" ? items.add(e.id) : items.delete(e.id));
  const land = () => {
    items = new Set(reads[read++]);
    for (const e of early.splice(0)) apply(e);
    const d = load_done(s);
    s = d.state;
    if (d.relist) { s = { ...s, loading: true }; } // the page loads again
  };
  for (const e of events) {
    if (e === "land") { land(); continue; }
    const r = on_event(s, e.seq);
    s = r.state;
    if (r.act === "apply") apply(e);
    else {
      early.push(e);
      if (r.act === "relist") s = { ...s, loading: true };
    }
  }
  while (s.loading) land();
  return [...items].sort();
}

test("a snapshot read before a dropped removal is replaced: [2,3], not [1,2,3]", () => {
  // the first list ([1,2,3]) was read before "remove 1" (seq 2) was shed;
  // seq 1 and seq 3 reach the page while that list is on its way
  const out = run_page(
    [{ seq: 1, kind: "add", id: 3 }, { seq: 3, kind: "add", id: 4 }, "land"],
    [[1, 2, 3], [2, 3, 4]],
  );
  assert.deepEqual(out, [2, 3, 4]);
});

test("a second gap during catch-up is caught: [3,4,6], not [2,3,4,6]", () => {
  const out = run_page(
    [
      { seq: 1, kind: "add", id: 1 }, "land",
      { seq: 3, kind: "add", id: 4 },  // gap: seq 2 shed, the page lists again
      { seq: 5, kind: "add", id: 6 },  // another gap while that list is on its way
      "land",
    ],
    [[1], [2, 3, 4], [3, 4, 6]],
  );
  assert.deepEqual(out, [3, 4, 6]);
});

test("the flag is set by a gap during a load and cleared by the re-list", () => {
  const r = on_event({ last: 4, loading: true, gap_seen: false }, 6);
  assert.equal(r.act, "queue");
  assert.equal(r.state.gap_seen, true);
  const d = load_done(r.state);
  assert.equal(d.relist, true);
  assert.equal(load_done(d.state).relist, false);
});

test("contiguous events during a load need no second list", () => {
  let s = { last: 4, loading: true, gap_seen: false };
  s = on_event(s, 5).state;
  s = on_event(s, 6).state;
  assert.equal(load_done(s).relist, false);
});

test("the first event has nothing to follow", () => {
  assert.equal(needs_resync(null, 1), false);
  assert.equal(needs_resync(null, 40), false);
});

test("the next number is not a gap", () => {
  assert.equal(needs_resync(1, 2), false);
  assert.equal(needs_resync(63, 64), false);
});

test("a skipped number is", () => {
  assert.equal(needs_resync(1, 3), true);
  assert.equal(needs_resync(5, 70), true);
});

test("a repeated or earlier number is too: the stream was restarted", () => {
  assert.equal(needs_resync(4, 4), true);
  assert.equal(needs_resync(4, 1), true);
});
