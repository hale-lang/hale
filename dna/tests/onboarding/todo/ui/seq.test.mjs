// Run by the `ui` compose service before it serves the page, and by hand:
//   node --test ui/seq.test.mjs
// (node's built-in runner; there is no build step and no dependency).
import { test } from "node:test";
import assert from "node:assert/strict";
import { needs_resync } from "./seq.js";

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
