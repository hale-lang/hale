// JS equivalent of form_hashmap_get.ap.

const n = 150_000;
const m = new Map();
for (let i = 0; i < n; i++) {
    m.set(i, { id: i, v: i + 1 });
}

const t0 = process.hrtime.bigint();
let acc = 0;
for (let j = 0; j < n; j++) {
    const e = m.get(j);
    acc = acc + e.v;
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`n=${n}`);
console.log(`acc=${acc}`);
console.log(`elapsed_ns=${elapsed}`);
