// Drives flower.wasm's @export frame() like a rAF loop would, validates
// the draw-call stream, and measures per-frame wasm cost.
import { run } from "./flower.mjs";

let clears = 0, petals = 0, edges = 0, dones = [];
let bad = [];
const num = (v, name) => {
  if (typeof v !== "number" || !Number.isFinite(v)) bad.push(`${name}: ${v}`);
  return v;
};

const inst = await run(() => ({
  ctx_clear: (w, h) => { num(w, "clear.w"); num(h, "clear.h"); clears++; },
  ctx_petal: (cx, cy, angle, len, bloom, hue) => {
    num(cx, "petal.cx"); num(cy, "petal.cy"); num(angle, "petal.angle");
    num(len, "petal.len");
    if (bloom < -0.001 || bloom > 1.001) bad.push(`bloom out of range: ${bloom}`);
    petals++;
  },
  ctx_edge: (x1, y1, x2, y2, load) => {
    num(x1, "edge.x1"); num(y2, "edge.y2");
    if (load < -0.001 || load > 1.001) bad.push(`load out of range: ${load}`);
    edges++;
  },
  frame_done: (n, p) => dones.push([Number(n), Number(p)]),
}));

const FRAMES = 600;
// warmup
for (let f = 0; f < 60; f++) inst.exports.frame(f * 16.6667);
const t0 = process.hrtime.bigint();
for (let f = 60; f < 60 + FRAMES; f++) inst.exports.frame(f * 16.6667);
const t1 = process.hrtime.bigint();
const perFrameUs = Number(t1 - t0) / 1000 / FRAMES;

const lastDone = dones[dones.length - 1];
const ok =
  bad.length === 0 &&
  clears === 60 + FRAMES &&
  edges === 60 + FRAMES &&
  petals === (60 + FRAMES) * 36 &&
  dones.length === 60 + FRAMES &&
  lastDone[0] === 60 + FRAMES &&   // persistent counter survived every call
  lastDone[1] === 36;

console.log(JSON.stringify({
  ok, clears, petals, edges,
  persistentCounter: lastDone[0],
  perFrameUs: +perFrameUs.toFixed(2),
  bad: bad.slice(0, 5),
}));
process.exit(ok ? 0 : 1);
