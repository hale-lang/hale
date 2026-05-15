// JS equivalent of vec_amortized.ap.

const n = 200_000;
const t0 = process.hrtime.bigint();

const v = [];
for (let i = 0; i < n; i++) {
    v.push(i);
}
let sum = 0;
for (let j = 0; j < n; j++) {
    sum = sum + v[j];
}

const elapsed = process.hrtime.bigint() - t0;
console.log(`n=${n}`);
console.log(`sum=${sum}`);
console.log(`elapsed_ns=${elapsed}`);
