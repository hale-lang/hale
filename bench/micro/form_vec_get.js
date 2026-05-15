// JS equivalent of form_vec_get.ap.
// Populate (outside timing) then time indexed reads. V8 inlines
// the indexed access; closest analog to Aperio's bounds-checked
// `get(i) or raise`.

const iters = 200_000;
const v = new Array(iters);
for (let i = 0; i < iters; i++) {
    v[i] = i;
}

const t0 = process.hrtime.bigint();
let acc = 0;
for (let j = 0; j < iters; j++) {
    acc = v[j];
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`iters=${iters}`);
console.log(`acc=${acc}`);
console.log(`elapsed_ns=${elapsed}`);
