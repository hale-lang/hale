// JS equivalent of fn_scratch_work.ap.

function doWork(n) {
    const v = [];
    for (let i = 0; i < n; i++) {
        v.push(i);
    }
    let sum = 0;
    for (let j = 0; j < n; j++) {
        sum = sum + v[j];
    }
    return sum;
}

const calls = 100;
const perCall = 1000;
const t0 = process.hrtime.bigint();
let total = 0;
for (let c = 0; c < calls; c++) {
    total = total + doWork(perCall);
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`calls=${calls}`);
console.log(`per_call=${perCall}`);
console.log(`total=${total}`);
console.log(`elapsed_ns=${elapsed}`);
