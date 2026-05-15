// JS equivalent of form_hashmap_set.ap.
// Uses Map (not plain object) for fair Int-keyed comparison —
// Map handles arbitrary key types and is closest to Aperio's
// hashmap by semantics.

const n = 1_000_000;
const m = new Map();
const t0 = process.hrtime.bigint();
for (let i = 0; i < n; i++) {
    m.set(i, { id: i, v: i + 1 });
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`n=${n}`);
console.log(`len=${m.size}`);
console.log(`elapsed_ns=${elapsed}`);
